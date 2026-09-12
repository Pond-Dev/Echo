//! Which addresses inside `client.dll` Echo depends on.
//!
//! The values themselves are not written here. They come from the generated
//! dump in `vendor/cs2-dumper/offsets.rs`, taken verbatim from
//! <https://github.com/a2x/cs2-dumper> so that updating is a file replacement
//! rather than a transcription:
//!
//! ```text
//! curl -sS -o vendor/cs2-dumper/offsets.rs \
//!   https://raw.githubusercontent.com/a2x/cs2-dumper/main/output/offsets.rs
//! ```
//!
//! **These expire.** Valve moves them on most game updates, so a stale dump is
//! the ordinary failure here. It shows up as a reading that fails its own
//! plausibility check, never as a crash — which is what those checks are for.
//!
//! What this module adds over the raw dump is the *list*: everything Echo
//! actually reads, under Echo's own names. When the dump is refreshed, this is
//! the set to re-verify.

#[path = "../../vendor/cs2-dumper/offsets.rs"]
mod generated;

use generated::cs2_dumper::offsets::client_dll;

/// Where the player is looking: two `f32` (pitch, yaw) laid out back to back.
pub const VIEW_ANGLES: usize = client_dll::dwViewAngles;
