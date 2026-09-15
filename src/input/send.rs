//! Send head AimLock movement only while the game owns the foreground window.

use std::mem::size_of;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE, MOUSEINPUT, SendInput,
};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub fn focused(game_pid: u32) -> bool {
    let mut owner = 0;
    unsafe {
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return false;
        }
        GetWindowThreadProcessId(window, Some(&mut owner));
    }
    game_pid != 0 && owner == game_pid
}

pub fn move_by(game_pid: u32, dx: i32, dy: i32) -> bool {
    // Recheck at the send boundary: focus can change after target selection.
    if !focused(game_pid) {
        return false;
    }
    if dx == 0 && dy == 0 {
        return true;
    }
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: MOUSEEVENTF_MOVE,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe { SendInput(&[input], size_of::<INPUT>() as i32) == 1 }
}
