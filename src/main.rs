//! Echo — step 5: draw on top of the game.
//!
//! Four steps of reading memory end here; this one puts something on screen.
//! An always-on-top, click-through window is laid over the game's client area
//! and a border, a centre marker and a few lines of text are drawn into it.
//!
//! Nothing drawn comes from the world yet — that is step 6. What this proves
//! is that the window lands in the right place, that the game shows through
//! everywhere we did not draw, and that the mouse still reaches the game.
//!
//! The check is your eyes: the border should hug the picture, the marker
//! should sit around the game's own crosshair, and shooting should still work.

use std::time::Duration;

use echo::console::Screen;
use echo::game::{Game, LocalPlayer, Player, Team, ViewAngles};
use echo::log::Log;
use echo::overlay::{Overlay, rgb};
use echo::process::AttachError;
use windows::core::w;

const POLL: Duration = Duration::from_millis(100);
const GAME_WINDOW: windows::core::PCWSTR = w!("Counter-Strike 2");
const ACCENT: windows::Win32::Foundation::COLORREF = rgb(0, 220, 120);
const TEXT: windows::Win32::Foundation::COLORREF = rgb(235, 235, 235);

fn main() {
    let mut log = Log::create();
    if let Some(path) = log.path() {
        println!("log: {}", path.display());
    }

    if let Err(error) = run(&mut log) {
        println!("\nFailed: {error}");
        log.record(&format!("failed: {error}"));
    }
    wait_before_closing();
}

fn run(log: &mut Log) -> Result<(), AttachError> {
    let Some(game) = Game::attach()? else {
        log.say("cs2.exe is running but client.dll has not loaded yet.");
        return Ok(());
    };

    let client = game.client();
    log.record(&format!("cs2.exe pid={}", game.pid()));
    log.record(&format!(
        "client.dll base=0x{:X} size={}",
        client.base, client.size
    ));

    if !game.client_looks_like_a_module()? {
        log.say("No PE signature at the module base — attached to the wrong thing.");
        return Ok(());
    }

    let screen = Screen::new();
    let mut overlay = Overlay::over(GAME_WINDOW)?;
    match &overlay {
        Some(overlay) => log.record(&format!("overlay over {:?}", overlay.bounds())),
        None => log.record("no game window — the overlay will not be shown"),
    }

    let mut last_logged = None;
    loop {
        let angles = game.view_angles()?;
        let me = game.local_player()?;
        let mut players = game.players()?;
        // Enemies first, then teammates, then everyone else; stable within a
        // group by slot so the list does not jump around between frames.
        players.sort_by_key(|player| rank(*player, me));

        screen.draw(&frame(&game, angles, me, &players));

        if let Some(overlay) = overlay.as_mut() {
            overlay.pump();
            if overlay.target_is_alive() {
                overlay.follow_target();
                draw_overlay(overlay, angles, me, &players);
            }
        }

        // The log records the roster, not the movement. Fourteen players
        // walking around change their positions every single pass, and a file
        // that re-states all of them ten times a second records that the clock
        // is running. What is worth finding afterwards is who was present, on
        // which side, and whether they were alive — so that is the key.
        let roster = roster(me, &players);
        if last_logged.as_ref() != Some(&roster) {
            for line in records(me, &players) {
                log.record(&line);
            }
            last_logged = Some(roster);
        }

        std::thread::sleep(POLL);
    }
}

/// Step 5 draws nothing derived from the world yet — that is step 6. What it
/// proves is that the window is in the right place, that the game shows
/// through it, and that it does not eat the mouse.
fn draw_overlay(
    overlay: &Overlay,
    angles: ViewAngles,
    me: Option<LocalPlayer>,
    players: &[Player],
) {
    let bounds = overlay.bounds();
    overlay.frame(|canvas| {
        // A border on the client area. If this hugs the picture, the overlay
        // is aligned; if it is off, the sizing is wrong and every box drawn
        // later would be wrong the same way.
        canvas.rect(0, 0, bounds.width - 1, bounds.height - 1, ACCENT, 1);

        // Centre marker. The game's own crosshair should sit inside it.
        let (cx, cy) = (bounds.width / 2, bounds.height / 2);
        canvas.line((cx - 12, cy), (cx - 4, cy), ACCENT, 1);
        canvas.line((cx + 4, cy), (cx + 12, cy), ACCENT, 1);
        canvas.line((cx, cy - 12), (cx, cy - 4), ACCENT, 1);
        canvas.line((cx, cy + 4), (cx, cy + 12), ACCENT, 1);

        let enemies = players
            .iter()
            .filter(|p| me.is_some_and(|me| me.team.opposes(p.team)) && p.alive())
            .count();
        let status = [
            format!("echo  {}x{}", bounds.width, bounds.height),
            format!("pitch {:.1}  yaw {:.1}", angles.pitch, angles.yaw),
            match me {
                Some(me) => format!("{} health {}", me.team.label(), me.health),
                None => "no pawn".to_owned(),
            },
            format!("{} players, {enemies} enemies alive", players.len()),
        ];
        for (row, line) in status.iter().enumerate() {
            canvas.text(12, 12 + row as i32 * 18, line, TEXT);
        }
    });
}

/// Sort key: enemies, then teammates, then the rest.
fn rank(player: Player, me: Option<LocalPlayer>) -> (u8, usize) {
    let group = match me {
        Some(me) if me.team.opposes(player.team) => 0,
        Some(me) if me.team == player.team => 1,
        _ => 2,
    };
    (group, player.controller)
}

fn frame(
    game: &Game,
    angles: ViewAngles,
    me: Option<LocalPlayer>,
    players: &[Player],
) -> Vec<String> {
    let client = game.client();
    let mut lines = vec![
        format!("echo — pid {}   client.dll 0x{:X}", game.pid(), client.base),
        String::new(),
    ];

    lines.push(if angles.plausible() {
        format!(
            "  view    pitch {:>7.2}   yaw {:>8.2}",
            angles.pitch, angles.yaw
        )
    } else {
        format!("  view    implausible ({} {})", angles.pitch, angles.yaw)
    });

    lines.push(match me {
        None => "  me      no pawn — menu, between rounds, or spectating".to_owned(),
        Some(me) if !me.plausible() => format!("  me      health implausible ({})", me.health),
        Some(me) => format!(
            "  me      {:<4} health {:>4}{}",
            me.team.label(),
            me.health,
            if me.alive() { "" } else { "   dead" }
        ),
    });

    lines.push(String::new());
    if players.is_empty() {
        lines.push("  no players — not in a server".to_owned());
        return lines;
    }

    let enemies = players
        .iter()
        .filter(|p| me.is_some_and(|me| me.team.opposes(p.team)) && p.alive())
        .count();
    lines.push(format!(
        "  {} players, {enemies} enemies alive",
        players.len()
    ));
    lines.push(String::new());
    lines.push("  side  health  position                        relation".to_owned());
    for player in players {
        lines.push(describe(*player, me));
    }
    lines
}

fn describe(player: Player, me: Option<LocalPlayer>) -> String {
    let position = match player.origin {
        Some([x, y, z]) => format!("{x:>9.1} {y:>9.1} {z:>9.1}"),
        None => "       — no scene node —".to_owned(),
    };
    let relation = match me {
        Some(me) if me.pawn == player.pawn => "me",
        Some(me) if me.team.opposes(player.team) => "ENEMY",
        Some(me) if me.team == player.team => "team",
        _ => "",
    };
    let health = if player.plausible() {
        format!("{:>6}", player.health)
    } else {
        format!("{:>6}?", player.health)
    };
    format!(
        "  {:<4}{health}  {position}    {relation}{}",
        player.team.label(),
        if player.alive() { "" } else { "  (dead)" }
    )
}

/// What makes this pass different from the last one, for the log's purposes.
///
/// Positions are deliberately not part of it: they change every pass and
/// would make every pass "different".
fn roster(me: Option<LocalPlayer>, players: &[Player]) -> Vec<(usize, Team, i32)> {
    let mut key: Vec<_> = players
        .iter()
        .map(|player| (player.pawn, player.team, player.health))
        .collect();
    if let Some(me) = me {
        key.push((me.pawn, me.team, me.health));
    }
    key
}

/// The same pass, shaped for the log: one line per player, fields not prose.
fn records(me: Option<LocalPlayer>, players: &[Player]) -> Vec<String> {
    let mut lines = vec![match me {
        None => "me=none".to_owned(),
        Some(me) => format!(
            "me pawn=0x{:X} team={} health={}",
            me.pawn,
            me.team.label(),
            me.health
        ),
    }];
    for player in players {
        // Enough precision to tell a real coordinate from a quantised one:
        // a position that only ever lands on a grid is reading the wrong
        // field, and rounding to whole units would hide that.
        let position = player.origin.map_or_else(
            || "origin=none".to_owned(),
            |[x, y, z]| format!("x={x:.2} y={y:.2} z={z:.2}"),
        );
        lines.push(format!(
            "  player pawn=0x{:X} team={} health={} {position}{}",
            player.pawn,
            player.team.label(),
            player.health,
            if player.plausible() {
                ""
            } else {
                " HEALTH_IMPLAUSIBLE"
            }
        ));
    }
    lines
}

/// Double-clicking a console binary closes the window the moment it returns,
/// so the output is unreadable. Hold it open until a key is pressed.
fn wait_before_closing() {
    println!("\nPress Enter to close.");
    let _ = std::io::stdin().read_line(&mut String::new());
}
