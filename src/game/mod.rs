//! What CS2 looks like from outside: typed reads over raw addresses.
//!
//! This is the only module that knows CS2 exists. It turns bytes at an offset
//! into values with names and validity rules, so callers never see an address.

use crate::process::{AttachError, Module, Process};

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
        let address = self.client.base + offsets::module::VIEW_ANGLES;
        Ok(ViewAngles {
            pitch: self.process.read_f32(address)?,
            yaw: self.process.read_f32(address + 4)?,
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

        Ok(Some(LocalPlayer { pawn, health }))
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
}

/// The local player's pawn, as far as Echo reads it today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalPlayer {
    /// Address of the pawn inside the game, kept for reads that follow.
    pub pawn: usize,
    pub health: i32,
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
    use super::{LocalPlayer, ViewAngles};

    fn player(health: i32) -> LocalPlayer {
        LocalPlayer {
            pawn: 0x1234_5678,
            health,
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
