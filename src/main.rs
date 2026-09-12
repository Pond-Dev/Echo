//! Echo — step 2: read the player's view angles live.
//!
//! Step 1 proved the plumbing (attach, find `client.dll`, read a byte). This
//! adds the first real game value: where the player is looking. The view
//! angles sit at a fixed offset with no pointer to follow, which isolates
//! "is the offset right" from "can we walk a chain".
//!
//! The check is the mouse: move it and the numbers must move with it.

use std::io::Write;
use std::time::Duration;

use echo::game::Game;
use echo::log::Log;
use echo::process::AttachError;

const POLL: Duration = Duration::from_millis(50);

fn main() {
    let mut log = Log::create();
    if let Some(path) = log.path() {
        println!("log: {}", path.display());
    }

    if let Err(error) = run(&mut log) {
        log.say(&format!("\nFailed: {error}"));
    }
    wait_before_closing();
}

fn run(log: &mut Log) -> Result<(), AttachError> {
    let Some(game) = Game::attach()? else {
        log.say("cs2.exe is running but client.dll has not loaded yet.");
        return Ok(());
    };

    let client = game.client();
    log.say(&format!("cs2.exe      pid={}", game.pid()));
    log.say(&format!(
        "client.dll   base=0x{:X}  size={} bytes",
        client.base, client.size
    ));

    if !game.client_looks_like_a_module()? {
        log.say("\nNo PE signature at the module base — attached to the wrong thing.");
        return Ok(());
    }

    log.say("\nMove your mouse — pitch and yaw should follow it.");
    log.say("Ctrl+C to stop.\n");

    let mut stdout = std::io::stdout();
    let mut was_plausible = true;
    let mut last_logged = None;
    loop {
        let angles = game.view_angles()?;
        let plausible = angles.plausible();

        if plausible {
            print!(
                "\r  pitch {:>7.2}    yaw {:>8.2}          ",
                angles.pitch, angles.yaw
            );
        } else {
            print!(
                "\r  implausible: pitch {} yaw {} — the offset is probably stale  ",
                angles.pitch, angles.yaw
            );
        }
        let _ = stdout.flush();

        // The console is a live readout; the log is a history. Only the log
        // keeps every sample, and a change of verdict is called out because
        // that is the line worth finding afterwards.
        if plausible != was_plausible {
            log.record(if plausible {
                "--- readings became plausible ---"
            } else {
                "--- readings became implausible: stale offset? ---"
            });
            was_plausible = plausible;
        }
        // Only changes are worth a line. A still mouse produces the same
        // reading twenty times a second, and recording that says the clock is
        // running, not that anything happened. The console already shows the
        // loop is alive.
        if last_logged != Some(angles) {
            log.record(&format!(
                "pitch={:.3} yaw={:.3}{}",
                angles.pitch,
                angles.yaw,
                if plausible { "" } else { "  IMPLAUSIBLE" }
            ));
            last_logged = Some(angles);
        }

        std::thread::sleep(POLL);
    }
}

/// Double-clicking a console binary closes the window the moment it returns,
/// so the output is unreadable. Hold it open until a key is pressed.
fn wait_before_closing() {
    println!("\nPress Enter to close.");
    let _ = std::io::stdin().read_line(&mut String::new());
}
