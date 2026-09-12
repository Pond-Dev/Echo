//! The mouse, in both directions.
//!
//! [`raw`] reads what the player's hand did. [`send`] moves the view without
//! one. They are deliberately separate steps of the build and stay separate
//! here, because the whole product turns on telling the two apart.
//!
//! One thing they are not separate in: anything sent through [`send`] comes
//! straight back through [`raw`], since Windows delivers injected movement to
//! raw input the same as a device's. Reading the hand therefore does not yet
//! mean reading *only* the hand — which is the problem the step about yielding
//! to the player exists to solve, and is not solved here.

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
