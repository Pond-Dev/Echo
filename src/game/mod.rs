//! What CS2 looks like from outside: typed reads over raw addresses.
//!
//! This is the only module that knows CS2 exists. It turns bytes at an offset
//! into values with names and validity rules, so callers never see an address.

use crate::process::{AttachError, Module, Process};

pub mod entities;
pub mod offsets;

const EXE: &str = "cs2.exe";
const CLIENT_MODULE: &str = "client.dll";

/// A live connection to the running game.
pub struct Game {
    process: Process,
    client: Module,
}

impl Game {
    /// Attach to CS2 and locate `client.dll`.
    ///
    /// `Ok(None)` means the process is there but the module has not loaded
    /// yet, which is the normal state for the first seconds of startup.
    pub fn attach() -> Result<Option<Self>, AttachError> {
        let process = Process::attach(EXE)?;
        let Some(client) = process.module(CLIENT_MODULE)? else {
            return Ok(None);
        };
        Ok(Some(Self { process, client }))
    }

    pub const fn client(&self) -> Module {
        self.client
    }

    pub const fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// Confirm `client.dll` really is a loaded PE image. Cheap sanity check
    /// that separates "attached to the wrong thing" from "offset is stale".
    pub fn client_looks_like_a_module(&self) -> windows::core::Result<bool> {
        let mut header = [0u8; 2];
        self.process.read(self.client.base, &mut header)?;
        Ok(&header == b"MZ")
    }

    /// Where the player is currently looking.
    pub fn view_angles(&self) -> windows::core::Result<ViewAngles> {
        // Both floats in one read. They are adjacent, so two reads would pay
        // twice for the same page and could straddle a write between them.
        let mut bytes = [0u8; 8];
        self.process
            .read(self.client.base + offsets::module::VIEW_ANGLES, &mut bytes)?;
        Ok(ViewAngles {
            pitch: f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            yaw: f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        })
    }

    /// The local player, when there is one.
    ///
    /// `None` is an ordinary state, not a failure: there is no pawn in the
    /// main menu, between rounds, or while spectating.
    pub fn local_player(&self) -> windows::core::Result<Option<LocalPlayer>> {
        let base = self.client.base + offsets::module::LOCAL_PLAYER_PAWN;
        let Some(pawn) = self.process.read_pointer(base)? else {
            return Ok(None);
        };

        // The pawn can be freed between reading the pointer and following it —
        // a round ending mid-tick is enough. Treat a failed dereference as
        // "no player right now" and try again next tick, rather than as a
        // fault: the next read either finds a pawn or does not.
        let Ok(health) = self.process.read_i32(pawn + offsets::entity::HEALTH) else {
            return Ok(None);
        };

        let team = Team::from_raw(self.process.read_u8(pawn + offsets::entity::TEAM)?);
        Ok(Some(LocalPlayer { pawn, health, team }))
    }

    /// Every connected player the game will tell us about.
    ///
    /// Nothing is cached between calls. The entity table moves under us — a
    /// respawn allocates a new pawn at a new address — so every pass re-reads
    /// the chunk pointers as well as the entities themselves.
    pub fn players(&self) -> windows::core::Result<Vec<Player>> {
        let Some(system) = self
            .process
            .read_pointer(self.client.base + offsets::module::ENTITY_SYSTEM)?
        else {
            return Ok(Vec::new());
        };

        let mut players = Vec::new();
        for index in entities::CONTROLLER_INDICES {
            // An empty slot is the normal case: a server with ten players
            // leaves fifty-four of these null.
            let Ok(Some(controller)) = self.entity(system, index) else {
                continue;
            };
            if let Ok(Some(player)) = self.player_from_controller(system, controller) {
                players.push(player);
            }
        }
        Ok(players)
    }

    /// Resolve one entity index to its address, through the chunk table.
    fn entity(&self, system: usize, index: u32) -> windows::core::Result<Option<usize>> {
        let Some(chunk_pointer) = entities::chunk_pointer(system, index) else {
            return Ok(None);
        };
        let Some(chunk) = self.process.read_pointer(chunk_pointer)? else {
            return Ok(None);
        };
        self.process
            .read_pointer(chunk + entities::entry_offset(index))
    }

    /// The pawn a controller is driving, and what it looks like right now.
    ///
    /// A controller without a live pawn is a player who is connected but not
    /// embodied — spectating, or between rounds — which is ordinary.
    fn player_from_controller(
        &self,
        system: usize,
        controller: usize,
    ) -> windows::core::Result<Option<Player>> {
        let handle = self
            .process
            .read_u32(controller + offsets::controller::PAWN_HANDLE)?;
        let Some(index) = entities::handle_index(handle) else {
            return Ok(None);
        };
        let Some(pawn) = self.entity(system, index)? else {
            return Ok(None);
        };

        let origin = self.origin_of(pawn)?;
        let eye = origin.and_then(|origin| {
            let view = self.vector_at(pawn + offsets::pawn::VIEW_OFFSET).ok()?;
            (view[0].abs() <= 16.0 && view[1].abs() <= 16.0 && (16.0..=80.0).contains(&view[2]))
                .then_some([
                    origin[0] + view[0],
                    origin[1] + view[1],
                    origin[2] + view[2],
                ])
        });
        let head = origin.and_then(|origin| self.head_of(pawn, origin));
        Ok(Some(Player {
            pawn,
            health: self.process.read_i32(pawn + offsets::entity::HEALTH)?,
            team: Team::from_raw(self.process.read_u8(pawn + offsets::entity::TEAM)?),
            origin,
            eye,
            head,
        }))
    }

    /// Where an entity stands: the point its feet are on.
    ///
    /// Not where its eyes are. The game's own `cl_showpos` reports the eye
    /// position, which is this plus a view offset — 64 units while standing —
    /// so the two disagree by that much and both are right. Anything aiming
    /// at a player will want the eye or a hitbox, not this.
    fn origin_of(&self, entity: usize) -> windows::core::Result<Option<[f32; 3]>> {
        let Some(node) = self
            .process
            .read_pointer(entity + offsets::entity::SCENE_NODE)?
        else {
            return Ok(None);
        };
        self.vector_at(node + offsets::scene_node::ORIGIN).map(Some)
    }

    /// The point to aim at: the centre of the head capsule, accepted only when
    /// the whole capsule lands where a head can be.
    fn head_of(&self, pawn: usize, origin: [f32; 3]) -> Option<[f32; 3]> {
        let node = self
            .process
            .read_pointer(pawn + offsets::entity::SCENE_NODE)
            .ok()??;
        let capsule = self.stable_head_capsule(node).ok()??;
        let centre = std::array::from_fn(|axis| (capsule[0][axis] + capsule[1][axis]) * 0.5);
        capsule
            .iter()
            .chain(std::iter::once(&centre))
            .all(|point| valid_head(origin, *point))
            .then_some(centre)
    }

    /// Three attempts, each thrown away unless the bone array is the same one
    /// before and after the read. The array is replaced while the game poses
    /// the skeleton, and a transform torn across that swap is not a pose any
    /// player ever had. It is worth another attempt, and worth none at all
    /// rather than aiming at it.
    fn stable_head_capsule(&self, node: usize) -> windows::core::Result<Option<[[f32; 3]; 2]>> {
        let address = node + offsets::scene_node::BONE_ARRAY;
        for _ in 0..3 {
            let Some(bones) = self.process.read_pointer(address)? else {
                continue;
            };
            let capsule = self.head_capsule_at(bones)?;
            if capsule.is_some() && self.process.read_pointer(address)? == Some(bones) {
                return Ok(capsule);
            }
        }
        Ok(None)
    }

    fn head_capsule_at(&self, bones: usize) -> windows::core::Result<Option<[[f32; 3]; 2]>> {
        let mut bytes = [0u8; offsets::scene_node::BONE_STRIDE];
        self.process.read(
            bones + offsets::scene_node::HEAD * offsets::scene_node::BONE_STRIDE,
            &mut bytes,
        )?;
        Ok(head_capsule(bytes))
    }

    fn vector_at(&self, address: usize) -> windows::core::Result<[f32; 3]> {
        let mut bytes = [0u8; 12];
        self.process.read(address, &mut bytes)?;
        Ok([
            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        ])
    }

    /// A separate, low-rate probe. These are fresh reads, not the steering snapshot.
    ///
    /// `eye_z` and `above_origin` are the pair worth reading: the capsule's
    /// two heights up the player against the height of that player's own
    /// eyes. Both ends well under the eyes means [`offsets::scene_node::HEAD`]
    /// is not naming the head, whatever the rest of the line says.
    pub fn geometry_probe(&self, pawn: usize) -> String {
        let eye_z = self
            .vector_at(pawn + offsets::pawn::VIEW_OFFSET)
            .map(|view| view[2]);
        let mut line = format!(
            "bone-probe pawn=0x{pawn:X} eye_z={}",
            crate::log::num(eye_z.ok(), 2)
        );
        let mut probe = || -> Result<(), String> {
            let node = self
                .process
                .read_pointer(pawn + offsets::entity::SCENE_NODE)
                .map_err(|e| format!("scene-node-read:{e}"))?
                .ok_or("scene-node-null")?;
            line += &format!(" node=0x{node:X}");
            let origin = self
                .vector_at(node + offsets::scene_node::ORIGIN)
                .map_err(|e| format!("origin-read:{e}"))?;
            line += &format!(" origin={}", crate::log::point(Some(origin)));
            let address = node + offsets::scene_node::BONE_ARRAY;
            let bones = self
                .process
                .read_pointer(address)
                .map_err(|e| format!("bone-array-read:{e}"))?
                .ok_or("bone-array-null")?;
            line += &format!(" bones=0x{bones:X}");
            let capsule = self
                .head_capsule_at(bones)
                .map_err(|e| format!("head-read:{e}"))?;
            line += &format!(
                " head=[{},{}] above_origin=[{},{}] head_valid={}",
                crate::log::point(capsule.map(|capsule| capsule[0])),
                crate::log::point(capsule.map(|capsule| capsule[1])),
                crate::log::num(capsule.map(|capsule| capsule[0][2] - origin[2]), 1),
                crate::log::num(capsule.map(|capsule| capsule[1][2] - origin[2]), 1),
                capsule.is_some_and(|capsule| capsule.iter().all(|p| valid_head(origin, *p)))
            );
            let after = self
                .process
                .read_pointer(address)
                .map_err(|e| format!("bone-array-recheck:{e}"))?;
            line += &format!(" array_stable={}", after == Some(bones));
            Ok(())
        };
        if let Err(error) = probe() {
            line += &format!(" error={error}");
        }
        line
    }
}

/// One bone transform: translation, uniform scale, then an XYZW quaternion.
///
/// Both capsule endpoints are carried through the bone's own rotation. A
/// fixed offset along world Z would be right only while the head is upright,
/// and a head is never upright at the moment somebody is worth shooting.
fn head_capsule(bytes: [u8; 32]) -> Option<[[f32; 3]; 2]> {
    let f: [f32; 8] =
        std::array::from_fn(|i| f32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()));
    // The gate is on the magnitude, the vector keeps the sign: a rig is
    // allowed to mirror a bone, and rejecting that loses a real head.
    if !f.iter().all(|v| v.is_finite()) || !(0.01..=100.0).contains(&f[3].abs()) {
        return None;
    }
    let norm = f[4..].iter().map(|v| v * v).sum::<f32>();
    if !(0.81..=1.21).contains(&norm) {
        return None;
    }
    let [x, y, z, w] = [f[4], f[5], f[6], f[7]].map(|v| v / norm.sqrt());
    Some(offsets::scene_node::HEAD_CAPSULE.map(|local| {
        let [vx, vy, vz] = local.map(|v| v * f[3]);
        let [tx, ty, tz] = [
            2.0 * (y * vz - z * vy),
            2.0 * (z * vx - x * vz),
            2.0 * (x * vy - y * vx),
        ];
        [
            f[0] + vx + w * tx + y * tz - z * ty,
            f[1] + vy + w * ty + z * tx - x * tz,
            f[2] + vz + w * tz + x * ty - y * tx,
        ]
    }))
}

fn valid_head(origin: [f32; 3], head: [f32; 3]) -> bool {
    origin.iter().chain(head.iter()).all(|v| v.is_finite())
        && (head[0] - origin[0]).hypot(head[1] - origin[1]) <= 64.0
        && (16.0..=96.0).contains(&(head[2] - origin[2]))
}

/// Which side an entity plays for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Team {
    Unassigned,
    Spectator,
    Terrorist,
    CounterTerrorist,
    /// Anything the game reports that is none of the above — a stale offset
    /// reads as this rather than being silently folded into a real side.
    Unknown(u8),
}

impl Team {
    pub const fn from_raw(raw: u8) -> Self {
        match raw {
            0 => Self::Unassigned,
            1 => Self::Spectator,
            2 => Self::Terrorist,
            3 => Self::CounterTerrorist,
            other => Self::Unknown(other),
        }
    }

    /// Whether this side actually plays. Spectators and the unassigned are
    /// never targets, and neither is a team we failed to recognise.
    pub const fn plays(self) -> bool {
        matches!(self, Self::Terrorist | Self::CounterTerrorist)
    }

    /// Whether `self` and `other` are on opposite playing sides.
    ///
    /// Spelled out rather than derived from "different team", so that an
    /// unrecognised side can never be promoted into an enemy: a stale offset
    /// reads as garbage, and garbage must not become a target.
    pub const fn opposes(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Terrorist, Self::CounterTerrorist) | (Self::CounterTerrorist, Self::Terrorist)
        )
    }
}

/// One connected player, as of this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    /// The body they are currently driving. Changes on every respawn.
    pub pawn: usize,
    pub health: i32,
    pub team: Team,
    /// `None` when the scene node could not be reached this pass.
    pub origin: Option<[f32; 3]>,
    /// Current camera position, including crouch view offset.
    pub eye: Option<[f32; 3]>,
    /// Current world-space head hitbox center; absent on a failed or invalid read.
    pub head: Option<[f32; 3]>,
}

impl Player {
    pub const fn alive(self) -> bool {
        self.health > 0
    }

    /// Same bounds as the local player: outside them, the pointer led
    /// somewhere that is not a pawn.
    pub const fn plausible(self) -> bool {
        0 <= self.health && self.health <= 100
    }
}

/// Pitch and yaw in degrees, as the engine stores them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewAngles {
    pub pitch: f32,
    pub yaw: f32,
}

impl ViewAngles {
    /// Whether these could be real view angles at all.
    ///
    /// The engine clamps pitch so the player cannot look past straight up or
    /// down, and normalises yaw into half a turn either way. A reading outside
    /// those bounds is not a strange camera — it is the wrong address, which
    /// is what a stale offset looks like.
    pub fn plausible(self) -> bool {
        self.pitch.is_finite()
            && self.yaw.is_finite()
            && (-90.0..=90.0).contains(&self.pitch)
            && (-180.0..=180.0).contains(&self.yaw)
    }

    /// How far the yaw swung from an earlier reading to this one, the short
    /// way round. Negative is rightward: the engine counts yaw anticlockwise,
    /// so moving the mouse right makes it fall.
    ///
    /// Subtracting the two directly is wrong at one place on the compass: yaw
    /// is normalised into half a turn either way, so a one-degree swing that
    /// happens to cross that seam subtracts into nearly a whole circle. The
    /// reading would be right everywhere a player happens to test it and
    /// wrong in one direction on the map.
    ///
    /// Only good for swings under half a turn — which is what "the short way
    /// round" means, and is not a limitation that can be lifted from two
    /// angles alone. A view that spun 371 degrees and one that moved 11 are
    /// the same two readings. Anything measuring a longer movement has to add
    /// up the steps as they happen.
    pub fn turn_from(self, earlier: Self) -> f32 {
        (self.yaw - earlier.yaw + 540.0).rem_euclid(360.0) - 180.0
    }
}

/// The local player's pawn, as far as Echo reads it today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalPlayer {
    /// Address of the pawn inside the game, kept for reads that follow.
    pub pawn: usize,
    pub health: i32,
    /// Which side we are on — the reference every enemy check is made against.
    pub team: Team,
}

impl LocalPlayer {
    /// Whether this could be a real player at all.
    ///
    /// Health is bounded by the game, so a value outside those bounds means
    /// the pointer led somewhere that is not a pawn — the shape a stale field
    /// offset takes. A dead player reads zero, which is a real state.
    pub const fn plausible(self) -> bool {
        0 <= self.health && self.health <= 100
    }

    pub const fn alive(self) -> bool {
        self.health > 0
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalPlayer, Team, ViewAngles};

    #[test]
    fn head_capsule_center_follows_rotation_and_rejects_bad_transforms() {
        // Installed player DATA skeleton starts with root_motion; index 6 is neck.
        let names = [
            "root_motion",
            "pelvis",
            "spine_0",
            "spine_1",
            "spine_2",
            "spine_3",
            "neck_0",
            "head_0",
        ];
        assert_eq!(names[super::offsets::scene_node::HEAD], "head_0");
        let encode = |f: [f32; 8]| -> [u8; 32] {
            let mut bytes = [0; 32];
            for (out, value) in bytes.chunks_exact_mut(4).zip(f) {
                out.copy_from_slice(&value.to_le_bytes());
            }
            bytes
        };
        let centre = |bytes| {
            super::head_capsule(bytes)
                .map(|c: [[f32; 3]; 2]| std::array::from_fn(|i| (c[0][i] + c[1][i]) * 0.5))
        };
        assert_eq!(
            super::head_capsule(encode([10.0, 20.0, 60.0, 1.0, 0.0, 0.0, 0.0, 1.0])),
            Some([[9.0, 21.8, 60.0], [13.5, 20.2, 60.0]])
        );
        assert_eq!(
            centre(encode([10.0, 20.0, 60.0, 1.0, 0.0, 0.0, 0.0, 1.0])),
            Some([11.25, 21.0, 60.0])
        );
        // A 90-degree turn around Y sends local +X toward world -Z.
        let q = std::f32::consts::FRAC_1_SQRT_2;
        let rotated: [f32; 3] = centre(encode([10.0, 20.0, 60.0, 2.0, 0.0, q, 0.0, q])).unwrap();
        for (actual, expected) in rotated.into_iter().zip([10.0, 22.0, 57.5]) {
            assert!((actual - expected).abs() < 0.0001, "got {rotated:?}");
        }
        // A mirrored bone is a pose, not a bad read: the sign drives the
        // vector while the magnitude alone decides plausibility.
        assert_eq!(
            centre(encode([10.0, 20.0, 60.0, -1.0, 0.0, 0.0, 0.0, 1.0])),
            Some([8.75, 19.0, 60.0])
        );
        assert!(super::head_capsule([0; 32]).is_none());
        assert!(super::head_capsule(encode([f32::NAN; 8])).is_none());
        assert!(super::head_capsule(encode([0.0, 0.0, 60.0, 1.0, 0.0, 0.0, 0.0, 2.0])).is_none());
    }

    #[test]
    fn head_geometry_rejects_zero_stale_and_nonfinite_transforms() {
        let origin = [100.0, 200.0, 10.0];
        assert!(super::valid_head(origin, [105.0, 202.0, 58.0]));
        assert!(super::valid_head(origin, [100.0, 200.0, 74.0]));
        for invalid in [
            [0.0; 3],
            origin,
            [1000.0, 200.0, 74.0],
            [100.0, 200.0, 1000.0],
            [f32::NAN, 200.0, 74.0],
        ] {
            assert!(!super::valid_head(origin, invalid));
        }
    }

    fn facing(yaw: f32) -> ViewAngles {
        ViewAngles { pitch: 0.0, yaw }
    }

    #[test]
    fn a_turn_is_the_difference_in_yaw_and_keeps_its_direction() {
        assert!((facing(30.0).turn_from(facing(10.0)) - 20.0).abs() < 1e-3);
        assert!((facing(10.0).turn_from(facing(30.0)) + 20.0).abs() < 1e-3);
        assert!(facing(45.0).turn_from(facing(45.0)).abs() < 1e-3);
    }

    #[test]
    fn a_small_turn_across_the_seam_stays_small_instead_of_becoming_a_full_circle() {
        // Facing one way along the seam and swinging a degree past it. Plain
        // subtraction gives 359, which would report a spin for a nudge.
        let across = facing(-179.5).turn_from(facing(179.5));
        assert!((across - 1.0).abs() < 1e-3, "got {across}");

        let back = facing(179.5).turn_from(facing(-179.5));
        assert!((back + 1.0).abs() < 1e-3, "got {back}");
    }

    #[test]
    fn half_a_turn_is_reported_as_half_a_turn_rather_than_as_nothing() {
        let half = facing(90.0).turn_from(facing(-90.0));
        assert!(half.abs() > 179.0, "got {half}");
    }

    fn player(health: i32) -> LocalPlayer {
        LocalPlayer {
            pawn: 0x1234_5678,
            health,
            team: Team::CounterTerrorist,
        }
    }

    #[test]
    fn health_inside_the_games_own_bounds_is_plausible_and_zero_means_dead() {
        assert!(player(100).plausible());
        assert!(player(1).plausible());
        assert!(player(100).alive());

        // Dead is a real state, not a bad reading.
        assert!(player(0).plausible());
        assert!(!player(0).alive());
    }

    #[test]
    fn only_the_two_playing_sides_can_ever_be_enemies() {
        let t = Team::Terrorist;
        let ct = Team::CounterTerrorist;

        assert!(t.opposes(ct));
        assert!(ct.opposes(t));
        assert!(!t.opposes(t), "a teammate is not an enemy");

        // Nothing that is not playing can be on either end of it. This is the
        // guard that stops a stale offset turning into a target.
        for bystander in [
            Team::Unassigned,
            Team::Spectator,
            Team::Unknown(7),
            Team::Unknown(255),
        ] {
            assert!(!bystander.plays(), "{bystander:?}");
            assert!(!bystander.opposes(t), "{bystander:?}");
            assert!(!t.opposes(bystander), "{bystander:?}");
            assert!(!bystander.opposes(bystander), "{bystander:?}");
        }
    }

    #[test]
    fn an_unrecognised_side_keeps_its_raw_value_instead_of_being_folded_into_a_real_one() {
        assert_eq!(Team::from_raw(2), Team::Terrorist);
        assert_eq!(Team::from_raw(3), Team::CounterTerrorist);
        assert_eq!(Team::from_raw(9), Team::Unknown(9));
        assert_eq!(Team::from_raw(200), Team::Unknown(200));
    }

    #[test]
    fn a_pointer_that_did_not_lead_to_a_pawn_reads_as_implausible() {
        assert!(!player(-1).plausible());
        assert!(!player(101).plausible());
        assert!(!player(1_819_043_144).plausible());
    }

    fn angles(pitch: f32, yaw: f32) -> ViewAngles {
        ViewAngles { pitch, yaw }
    }

    #[test]
    fn ordinary_angles_are_plausible_including_the_exact_limits() {
        assert!(angles(0.0, 0.0).plausible());
        assert!(angles(-89.0, 179.9).plausible());
        assert!(angles(90.0, -180.0).plausible());
    }

    #[test]
    fn a_stale_offset_reads_as_implausible_rather_than_as_a_strange_camera() {
        // What garbage memory actually looks like when reinterpreted as f32.
        assert!(!angles(1.0e30, 0.0).plausible());
        assert!(!angles(0.0, 4.2e9).plausible());
        assert!(!angles(f32::NAN, 0.0).plausible());
        assert!(!angles(0.0, f32::INFINITY).plausible());
        // Just past the engine's own clamp.
        assert!(!angles(90.1, 0.0).plausible());
        assert!(!angles(0.0, 180.1).plausible());
    }
}
