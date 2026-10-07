//! Mouse output restricted to the game in front, and the global unload key.

use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_DELETE};

pub mod send;
pub use send::{focused, move_by};

pub fn unload_requested() -> bool {
    // The high bit means held now; the low "pressed since" bit is shared.
    unsafe { GetAsyncKeyState(i32::from(VK_DELETE.0)) < 0 }
}
