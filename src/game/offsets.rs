//! Which addresses inside `client.dll` Echo depends on.
//!
//! The values themselves are not written here. They come from the generated
//! dumps in `vendor/cs2-dumper/`, taken verbatim from
//! <https://github.com/a2x/cs2-dumper> so that updating is a file replacement
//! rather than a transcription:
//!
//! ```text
//! for f in offsets client_dll; do
//!   curl -sS -o "vendor/cs2-dumper/$f.rs" \
//!     "https://raw.githubusercontent.com/a2x/cs2-dumper/main/output/$f.rs"
//! done
//! ```
//!
//! **These expire.** Valve moves them on most game updates, so a stale dump is
//! the ordinary failure here. It shows up as a reading that fails its own
//! plausibility check, never as a crash — which is what those checks are for.
//!
//! What this module adds over the raw dumps is the *list*: everything Echo
//! actually reads, under Echo's own names. When a dump is refreshed, this is
//! the set to re-verify.

#[path = "../../vendor/cs2-dumper/offsets.rs"]
mod generated_offsets;
#[path = "../../vendor/cs2-dumper/client_dll.rs"]
mod generated_schemas;

use generated_offsets::cs2_dumper::offsets::client_dll;
use generated_schemas::cs2_dumper::schemas::client_dll as schemas;

/// Addresses relative to the `client.dll` module base.
pub mod module {
    use super::client_dll;

    /// Where the player is looking: two `f32` (pitch, yaw) back to back.
    pub const VIEW_ANGLES: usize = client_dll::dwViewAngles;

    /// Pointer to the local player's pawn. Null whenever there is no pawn —
    /// in the main menu, between rounds, while spectating.
    pub const LOCAL_PLAYER_PAWN: usize = client_dll::dwLocalPlayerPawn;

    /// Pointer to the entity system, which owns the chunk table every other
    /// entity is reached through.
    pub const ENTITY_SYSTEM: usize = client_dll::dwEntityList;

    /// The 4x4 matrix the game renders with: sixteen `f32` in a row.
    pub const VIEW_MATRIX: usize = client_dll::dwViewMatrix;
}

/// Field positions inside an entity, relative to the entity's own address.
pub mod entity {
    use super::schemas;

    /// Current health. `i32`.
    pub const HEALTH: usize = schemas::C_BaseEntity::m_iHealth;

    /// Which side the entity plays for. `u8`.
    pub const TEAM: usize = schemas::C_BaseEntity::m_iTeamNum;

    /// Pointer to the node holding this entity's place in the world.
    pub const SCENE_NODE: usize = schemas::C_BaseEntity::m_pGameSceneNode;
}

/// Field positions inside a player controller — the persistent object for a
/// connected player, as opposed to the pawn it currently drives.
pub mod controller {
    use super::schemas;

    /// Handle to the pawn this player is currently controlling.
    pub const PAWN_HANDLE: usize = schemas::CCSPlayerController::m_hPlayerPawn;
}

/// Field positions inside a scene node.
pub mod scene_node {
    use super::schemas;

    /// Position in the world: three `f32`.
    pub const ORIGIN: usize = schemas::CGameSceneNode::m_vecAbsOrigin;
}
