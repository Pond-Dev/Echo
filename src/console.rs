//! A console that can be redrawn in place.
//!
//! Steps one to three printed a single line and overwrote it with a carriage
//! return. A list of players needs more than one line, so the screen is redrawn
//! from the top each pass instead.
//!
//! Windows consoles ignore ANSI escapes until asked not to, and the request can
//! fail — a redirected stream has no console at all. When it does, the frames
//! simply scroll instead of replacing each other, which is worse to read but
//! still correct. Nothing here is allowed to stop the run.

use std::io::Write;

use windows::Win32::System::Console::{
    CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetStdHandle,
    STD_OUTPUT_HANDLE, SetConsoleMode,
};

pub struct Screen {
    /// Whether escapes are honoured. Without them, frames scroll.
    redraws: bool,
}

impl Screen {
    pub fn new() -> Self {
        Self {
            redraws: enable_escapes().is_some(),
        }
    }

    /// Replace the screen with these lines.
    pub fn draw(&self, lines: &[String]) {
        let mut out = String::new();
        if self.redraws {
            // Home the cursor, then clear from there down: clearing first
            // would blank the screen for a moment and make it flicker.
            out.push_str("\x1b[H\x1b[J");
        }
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(out.as_bytes());
        let _ = stdout.flush();
    }
}

impl Default for Screen {
    fn default() -> Self {
        Self::new()
    }
}

/// Ask the console to interpret ANSI escapes. `None` if it will not.
fn enable_escapes() -> Option<()> {
    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE).ok()?;
        let mut mode = CONSOLE_MODE::default();
        GetConsoleMode(handle, &mut mode).ok()?;
        SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING).ok()?;
    }
    Some(())
}
