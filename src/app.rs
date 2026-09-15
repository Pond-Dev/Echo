//! Always-on head AimLock while CS2 owns the foreground window.

use crate::aim;
use crate::game::{Game, LocalPlayer, Player, ViewAngles};
use crate::input;
use crate::log::Log;
use crate::process::AttachError;
use std::time::{Duration, Instant};

const FRAME: Duration = Duration::from_millis(8);
const REPORT: Duration = Duration::from_secs(2);

pub fn start() {
    let mut log = Log::create();
    if let Some(path) = log.path() {
        println!("log: {}", path.display());
    }
    log.say("Head AimLock active while CS2 is in front. Close this window to stop.");
    if let Err(error) = run(&mut log) {
        log.say(&format!("failed: {error}"));
        println!("Press Enter to close.");
        let _ = std::io::stdin().read_line(&mut String::new());
    }
}

fn run(log: &mut Log) -> Result<(), AttachError> {
    let game = Game::attach()?.ok_or(AttachError::NotRunning)?;
    if !game.client_looks_like_a_module()? {
        log.say("Invalid client.dll module.");
        return Ok(());
    }
    log.record(&format!(
        "diagnostics=dynamic-head-v5 pid={} cone={} gain={} deadzone={} head_index={} bone_array_offset=0x{:X} crosshair_weight={} switch_margin={}",
        game.pid(), aim::CONE, aim::GAIN, aim::DEADZONE,
        crate::game::offsets::scene_node::HEAD, crate::game::offsets::scene_node::BONE_ARRAY,
        aim::CROSSHAIR_WEIGHT, aim::SWITCH_MARGIN,
    ));
    let mut locked = None;
    let mut last_state = None;
    let mut next_report = Instant::now();
    loop {
        let started = Instant::now();
        let me = game.local_player()?;
        let players = game.players()?;
        let view = game.view_angles()?;
        let choice = if input::focused(game.pid()) {
            aim::choose(me, &players, view, locked)
        } else {
            Err("not-in-front")
        };
        let mut reason = choice.as_ref().err().copied().unwrap_or("locked");
        let mut sent = [0, 0];
        let previous = locked;
        locked = choice.as_ref().ok().map(|choice| choice.pawn);
        if let Ok(choice) = &choice {
            if input::move_by(game.pid(), choice.counts[0], choice.counts[1]) {
                sent = choice.counts;
            } else {
                reason = "input-blocked";
                locked = None;
            }
        }
        let state = (locked, reason);
        let due = started >= next_report;
        if last_state != Some(state) || due {
            log.record(&format!(
                "aim-head reason={reason} target={} previous={previous:?} view={view:?} offset={:?} distance={:?} score={:?} sent={sent:?}",
                locked.map_or_else(|| "none".to_owned(), |pawn| format!("0x{pawn:X}")),
                choice.as_ref().ok().map(|choice| choice.offset),
                choice.as_ref().ok().map(|choice| choice.distance),
                choice.as_ref().ok().map(|choice| choice.score),
            ));
            last_state = Some(state);
        }
        if due {
            for line in lock_geometry(me, &players, view) {
                log.record(&line);
            }
            if let Some(me) = me.filter(|me| me.plausible() && me.alive()) {
                log.record(&game.geometry_probe(me.pawn));
                for player in &players {
                    if player.plausible() && player.alive() && me.team.opposes(player.team) {
                        log.record(&game.geometry_probe(player.pawn));
                    }
                }
            }
            next_report = started + REPORT;
        }
        std::thread::sleep(FRAME.saturating_sub(started.elapsed()));
    }
}

/// Snapshot geometry, including candidates that never reached the aiming cone.
fn lock_geometry(me: Option<LocalPlayer>, players: &[Player], view: ViewAngles) -> Vec<String> {
    let local = me.and_then(|me| players.iter().find(|p| p.pawn == me.pawn));
    let eye = local.and_then(|p| p.eye);
    let mut lines = vec![format!(
        "lock-local me={me:?} in_roster={} origin={:?} eye={eye:?} players={}",
        local.is_some(),
        local.and_then(|p| p.origin),
        players.len(),
    )];
    for player in players
        .iter()
        .filter(|p| me.is_some_and(|me| p.pawn != me.pawn && me.team.opposes(p.team)))
    {
        let at = eye
            .zip(player.head)
            .and_then(|(eye, head)| crate::aim::look_at(eye, head))
            .map(|desired| crate::aim::offset(view, desired));
        let distance = eye
            .zip(player.head)
            .map(|(eye, head)| aim::distance(eye, head));
        let score = at
            .zip(distance)
            .map(|(at, distance)| aim::score(at, distance));
        let status = if !player.plausible() {
            "invalid-health"
        } else if !player.alive() {
            "dead"
        } else if eye.is_none() {
            "missing-eye"
        } else if player.head.is_none() {
            "missing-head"
        } else if at.is_none_or(|at| !at.size().is_finite()) {
            "invalid-geometry"
        } else if !at.is_some_and(|at| at.size() <= aim::CONE) {
            "outside-cone"
        } else {
            "eligible"
        };
        lines.push(format!(
            "lock-candidate pawn=0x{:X} health={} origin={:?} head={:?} off_deg={:?} offset={at:?} distance={distance:?} score={score:?} status={status}",
            player.pawn, player.health, player.origin, player.head, at.map(|at| at.size()),
        ));
    }
    lines
}
