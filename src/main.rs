//! Echo — step 3: follow a pointer into the game.
//!
//! Step 2 read a value at a fixed offset. This one reads a *pointer* at a
//! fixed offset and then reads through it, which is how everything else in
//! the game is reached. The pointer is null whenever there is no pawn — the
//! main menu, between rounds, spectating — and that is an ordinary state
//! rather than a failure, so the first real fail-closed decision lives here.
//!
//! The check is the game: health tracks what the HUD shows, and disappears
//! when the pawn does.

use std::io::Write;
use std::time::Duration;

use echo::game::{Game, LocalPlayer, ViewAngles};
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

    log.say("\nMove your mouse and take some damage — both should follow.");
    log.say("Ctrl+C to stop.\n");

    let mut stdout = std::io::stdout();
    let mut last_logged = None;
    loop {
        let reading = (game.view_angles()?, game.local_player()?);

        print!("\r  {:<70}", describe(reading));
        let _ = stdout.flush();

        // Only changes are worth a line. A still mouse produces the same
        // reading twenty times a second, and recording that says the clock is
        // running, not that anything happened. The console already shows the
        // loop is alive.
        if last_logged != Some(reading) {
            log.record(&record(reading));
            last_logged = Some(reading);
        }

        std::thread::sleep(POLL);
    }
}

/// One line for the live console readout.
fn describe((angles, player): (ViewAngles, Option<LocalPlayer>)) -> String {
    let aim = if angles.plausible() {
        format!("pitch {:>7.2}   yaw {:>8.2}", angles.pitch, angles.yaw)
    } else {
        format!("angles implausible ({} {})", angles.pitch, angles.yaw)
    };

    let body = match player {
        None => "no pawn — menu, between rounds, or spectating".to_owned(),
        Some(player) if !player.plausible() => {
            format!("health implausible ({})", player.health)
        }
        Some(player) if player.alive() => format!("health {:>4}", player.health),
        Some(_) => "dead".to_owned(),
    };

    format!("{aim}   {body}")
}

/// The same reading, shaped for the log: fields rather than prose, so a run
/// can be scanned or grepped afterwards.
fn record((angles, player): (ViewAngles, Option<LocalPlayer>)) -> String {
    let mut line = format!("pitch={:.3} yaw={:.3}", angles.pitch, angles.yaw);
    if !angles.plausible() {
        line.push_str(" ANGLES_IMPLAUSIBLE");
    }
    match player {
        None => line.push_str(" pawn=none"),
        Some(player) => {
            line.push_str(&format!(
                " pawn=0x{:X} health={}",
                player.pawn, player.health
            ));
            if !player.plausible() {
                line.push_str(" HEALTH_IMPLAUSIBLE");
            }
        }
    }
    line
}

/// Double-clicking a console binary closes the window the moment it returns,
/// so the output is unreadable. Hold it open until a key is pressed.
fn wait_before_closing() {
    println!("\nPress Enter to close.");
    let _ = std::io::stdin().read_line(&mut String::new());
}
