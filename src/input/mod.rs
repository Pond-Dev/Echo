//! The mouse, in both directions.
//!
//! [`raw`] reads what the player's hand did. [`send`] moves the view without
//! one. They are deliberately separate steps of the build and stay separate
//! here, because the whole product turns on telling the two apart.
//!
//! Whether they stay separate all the way through Windows was an open
//! question, and it has been measured rather than assumed: sending nine
//! hundred movements over eight seconds, with the player's hand off the
//! mouse, delivered no packets at all to [`raw`]. Injected movement does not
//! return through our own sink, so what arrives there is the hand alone.
//!
//! Measured under one set of conditions, though — this window in the
//! background, this machine — so the step that has to tell the two apart
//! should confirm it rather than inherit it.

pub mod raw;
pub mod send;

pub use raw::RawMouse;
pub use send::move_by;

use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VIRTUAL_KEY};

/// Whether a key is down right now.
///
/// Asked of the whole system rather than of a window, because the overlay
/// never has focus and would be told about nothing. The high bit is the
/// current state; the low bit — whether it was pressed since the last call —
/// is deliberately ignored, since a key that was tapped and released between
/// two frames must not read as held for one of them.
pub fn held(key: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(i32::from(key.0)) < 0 }
}
