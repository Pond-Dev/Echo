//! Echo — step 4: walk the entity table and find the other players.
//!
//! Steps two and three read one value and then one pointer. This walks a
//! table: sixty-four controller slots, each holding a handle to the pawn that
//! player is currently driving, each pawn reached through a chunk table. Most
//! slots are empty, most handles name nothing, and both are ordinary.
//!
//! Nothing is cached between passes. Step three's log showed a pawn address
//! changing three times across one death and respawn, so every pass re-reads
//! the chunk pointers as well as the entities in them.
//!
//! The check is the scoreboard: the players listed here, their sides and their
//! health should match it.

use std::time::Duration;

use echo::console::Screen;
use echo::game::{Game, LocalPlayer, Player, Team, ViewAngles};
use echo::log::Log;
use echo::process::AttachError;

const POLL: Duration = Duration::from_millis(100);

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
    let mut last_logged = None;
    loop {
        let angles = game.view_angles()?;
        let me = game.local_player()?;
        let mut players = game.players()?;
        // Enemies first, then teammates, then everyone else; stable within a
        // group by slot so the list does not jump around between frames.
        players.sort_by_key(|player| rank(*player, me));

        screen.draw(&frame(&game, angles, me, &players));

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
