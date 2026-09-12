//! What CS2 looks like from outside: typed reads over raw addresses.
//!
//! This is the only module that knows CS2 exists. It turns bytes at an offset
//! into values with names and validity rules, so callers never see an address.

use crate::process::{AttachError, Module, Process};

pub use projection::ViewMatrix;

pub mod entities;
pub mod offsets;
pub mod projection;

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

    /// Reads made since this was last called, and reset.
    pub fn take_reads(&self) -> u64 {
        self.process.take_reads()
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

    /// The matrix the game is currently rendering with.
    ///
    /// Read in one go rather than field by field: the game rewrites it every
    /// frame, and sixteen separate reads could straddle a write and mix two
    /// matrices into one that projects to nowhere real.
    pub fn view_matrix(&self) -> windows::core::Result<ViewMatrix> {
        let mut bytes = [0u8; 64];
        self.process
            .read(self.client.base + offsets::module::VIEW_MATRIX, &mut bytes)?;
        let mut values = [0f32; 16];
        for (slot, chunk) in values.iter_mut().zip(bytes.chunks_exact(4)) {
            *slot = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        }
        Ok(ViewMatrix::from_raw(values))
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

        Ok(Some(Player {
            controller,
            pawn,
            health: self.process.read_i32(pawn + offsets::entity::HEALTH)?,
            team: Team::from_raw(self.process.read_u8(pawn + offsets::entity::TEAM)?),
            origin: self.origin_of(pawn)?,
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
        let mut bytes = [0u8; 12];
        self.process
            .read(node + offsets::scene_node::ORIGIN, &mut bytes)?;
        Ok(Some([
            f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            f32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
            f32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        ]))
    }
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

    pub const fn label(self) -> &'static str {
        match self {
            Self::Unassigned => "none",
            Self::Spectator => "spec",
            Self::Terrorist => "T",
            Self::CounterTerrorist => "CT",
            Self::Unknown(_) => "?",
        }
    }
}

/// One connected player, as of this pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    /// The persistent object for this player.
    pub controller: usize,
    /// The body they are currently driving. Changes on every respawn.
    pub pawn: usize,
    pub health: i32,
    pub team: Team,
    /// `None` when the scene node could not be reached this pass.
    pub origin: Option<[f32; 3]>,
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
