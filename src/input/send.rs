//! Moving the view without a hand on the mouse.
//!
//! Sent as a *relative* movement, the same shape a mouse reports, rather than
//! as a cursor position. A game does not read the cursor — it reads movement —
//! so a position would move the arrow on the desktop and leave the view
//! exactly where it was.
//!
//! This is the first thing Echo does that the player can feel. Everything
//! before it read or drew, and being wrong cost a misplaced box; being wrong
//! here drags someone's aim off target in the middle of a round. The rule that
//! follows, and which the caller enforces rather than this module: send
//! nothing unless the game is the window in front.

use std::mem::size_of;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
};

/// Move the view by this many counts. `true` if Windows accepted it.
///
/// Counts, not pixels and not degrees. How far a count turns the view depends
/// on the player's sensitivity, which is the game's business and not ours to
/// know yet — this step only establishes that the movement arrives.
pub fn move_by(dx: i32, dy: i32) -> bool {
    if dx == 0 && dy == 0 {
        // Zero is not free: it still crosses into the kernel, and it still
        // arrives at every raw input listener — including our own — as a
        // packet that moved nothing.
        return true;
    }
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                // Without ABSOLUTE alongside it, these are counts to add to
                // where the pointer is, which is what a mouse sends.
                dwFlags: MOUSEEVENTF_MOVE,
                // Zero means "stamp it now". A time of our own would have to
                // agree with the system's tick count, and disagreeing with it
                // reorders the packet against the player's real ones.
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    // Returns how many events it managed. Anything short means the input was
    // blocked — most often by a window running at a higher integrity level
    // than ours, which is why the program asks for elevation.
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) == 1 }
}
