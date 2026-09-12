//! Echo — the step that moves the view towards someone.
//!
//! The first aim assist, and deliberately a raw one. Hold the key and the
//! view walks onto the nearest enemy in front of you and stays there. It does
//! not care what your hand is doing: push against it and it pushes back,
//! every pass, because the thing that yields to the player is the next step
//! and not this one. That is what raw means here — worth knowing before
//! holding the button down in a round that matters.
//!
//! The steering is feedback, not calculation. Each pass asks where the view
//! is, where it should be, and moves a share of the difference; the next pass
//! asks again. So nothing has to be exactly right — the number converting
//! degrees to mouse counts is one machine's sensitivity, and being wrong
//! about it changes how fast the view arrives, not where.
//!
//! Guards, in the order they refuse: the key must be down this instant, the
//! game must be the window in front, our own reading must be plausible, and
//! the target must be a living enemy inside a cone around where the player is
//! already pointing. Each refusal has its own name in the log, because a
//! reason that is only "nothing happened" is not a reason.
//!
//! The check is your eyes and the log together. Point near an enemy, hold the
//! key: the crosshair should arrive and settle without trembling. The log
//! says which enemy and how far off, every half second, so a view that walks
//! away instead of towards is visible as a figure that grows.

use std::time::{Duration, Instant};

use echo::aim;
use echo::aim::{Offset, Steering};
use echo::game::{Game, LocalPlayer, Player, Team, ViewAngles, ViewMatrix};
use echo::input::{self, RawMouse};
use echo::log::Log;
use echo::overlay::{FrameCost, Overlay, rgb};
use echo::process::AttachError;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_XBUTTON2};
use windows::core::w;

/// Target frame time. The game draws far faster than this, so a box is always
/// a little behind; the gap is what makes it float when the view swings.
const FRAME: Duration = Duration::from_millis(8);
/// How often to look for the game's window again while there is no overlay.
const ATTACH_RETRY: Duration = Duration::from_millis(500);
/// How often the achieved rate is written to the log.
const PACE_REPORT: Duration = Duration::from_secs(2);
/// How often a hold in progress is written to the log.
///
/// The totals at the end of an eight-second hold cannot say whether the view
/// moved steadily or moved for one second and then stopped. Lines along the
/// way can, by subtraction — which is the same mistake as measuring a turn
/// from its two ends, made about time instead of about angle.
const AIM_REPORT: Duration = Duration::from_millis(500);
const GAME_WINDOW: windows::core::PCWSTR = w!("Counter-Strike 2");
/// Held to steer — the fifth mouse button, under the thumb.
///
/// Unlike the key this started on, the game sees this press too. It is
/// unbound in a default install, so nothing happens twice, but a player who
/// has bound it will get both — which is theirs to decide, not ours to
/// prevent.
///
/// Read straight from the keyboard state rather than from the raw input this
/// program already receives. The two would disagree on the frame the button
/// goes down, and the one that matters is the one the game is acting on.
const AIM_KEY: VIRTUAL_KEY = VK_XBUTTON2;

/// How the view is steered.
///
/// `counts_per_degree` was measured, not assumed: five holds of about a
/// thousand counts each, every one landing within a thousandth of 0.0196
/// degrees a count on the machine it was measured on. That is one player's
/// sensitivity and not a property of the game, which the feedback loop is
/// what makes survivable — a share of the remaining distance each pass
/// arrives wherever the true figure is, only sooner or later.
const STEERING: Steering = Steering {
    counts_per_degree: 51.0,
    gain: 0.35,
    // Five degrees a pass, which at this rate is a fast flick and not a spin.
    // What a reading caught mid-write is allowed to cost.
    cap: 250,
    deadzone: 0.15,
    cone: 30.0,
};

/// How far above a player's feet their eyes are, standing.
///
/// Measured rather than guessed: the game's own position readout and the
/// entity's origin differ by exactly this, which is what said the origin is
/// feet and the readout is eyes. Anything aiming at a player needs the second.
const EYE_HEIGHT: f32 = 64.0;
const ENEMY: COLORREF = rgb(255, 70, 70);
const TEXT: COLORREF = rgb(235, 235, 235);
const WARN: COLORREF = rgb(255, 190, 60);

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

    // Retried rather than built once: the game window may not exist yet at
    // startup, may go away and come back, and there is no console readout any
    // more — an overlay that gave up would leave the program running silently
    // forever with nothing on screen.
    let mut overlay: Option<Overlay> = None;
    let mut next_attach = Instant::now();
    // Bound to the overlay's window, so it is created and dropped with it.
    let mut mouse: Option<RawMouse> = None;

    let mut aim = Aim::default();
    let mut next_aim = Instant::now();
    let mut last_logged = None;
    let mut next_pace = Instant::now();
    let mut pace = Pace::default();
    loop {
        let started = Instant::now();

        let mut stages = Stages::default();
        let me = Stages::time(&mut stages.me, || game.local_player())?;
        let mut players = Stages::time(&mut stages.players, || game.players())?;
        // Enemies first, then teammates, then everyone else; stable within a
        // group by slot so the list does not jump around between frames.
        players.sort_by_key(|player| rank(*player, me));

        // The matrix is read last, right before it is used. It is what decides
        // where a box lands, and reading six hundred player values after it
        // would leave it a whole pass out of date — which is a box that
        // floats behind the enemy whenever the view swings.
        let view = Stages::time(&mut stages.matrix, || game.view_matrix())?;

        // Drop an overlay whose window has gone, so the next attempt builds a
        // fresh one over the game's new window rather than drawing into a
        // handle that no longer names anything.
        if overlay.as_ref().is_some_and(|o| !o.target_is_alive()) {
            log.record("game window gone — dropping the overlay");
            overlay = None;
            mouse = None;
        }
        if overlay.is_none() && started >= next_attach {
            overlay = Overlay::over(GAME_WINDOW)?;
            match &overlay {
                Some(new) => {
                    log.record(&format!("overlay over {:?}", new.bounds()));
                    // Raw input is delivered to a window, so it can only be
                    // asked for once there is one.
                    mouse = match RawMouse::listen(new.window()) {
                        Ok(mouse) => Some(mouse),
                        Err(error) => {
                            log.record(&format!("no raw mouse: {error}"));
                            None
                        }
                    };
                }
                None => next_attach = started + ATTACH_RETRY,
            }
        }

        // Drained before the overlay's pump, which would otherwise take these
        // messages out of the queue and throw them away.
        let (hand, packets) = Stages::time(&mut stages.hand, || {
            mouse.as_mut().map_or(([0; 2], 0), |mouse| {
                let packets = mouse.poll();
                (mouse.take(), packets)
            })
        });
        stages.packets = packets;

        // Read last of the readings and just before it is used, because the
        // movement sent below is measured against it. A pass-old angle steers
        // towards where the target was relative to where the view was, and
        // those are two different moments.
        let angles = Stages::time(&mut stages.angles, || game.view_angles())?;
        // Focus is a guard, not a preference. With the game behind something
        // else, the same counts drag the pointer across whatever is in front.
        let allowed = overlay.as_ref().is_some_and(Overlay::target_has_focus);
        let before = aim.state();
        Stages::time(&mut stages.steer, || {
            aim.steer(allowed, me, &players, angles);
        });
        // Every change of state, and then every half second while it lasts.
        // A single line at the end of a hold cannot say whether the view
        // walked onto the target or walked away from it.
        if aim.state() != before || (aim.active && started >= next_aim) {
            log.record(&aim.describe());
            next_aim = started + AIM_REPORT;
        }

        if let Some(overlay) = overlay.as_mut() {
            Stages::time(&mut stages.pump, || overlay.pump());
            Stages::time(&mut stages.follow, || overlay.follow_target());
            stages.drawing = Stages::time(&mut stages.overlay, || {
                draw_overlay(
                    overlay,
                    view,
                    me,
                    &players,
                    &Readouts {
                        pace,
                        hand,
                        mouse: mouse.as_ref(),
                        aim,
                    },
                )
            });
        }
        stages.reads = game.take_reads();

        // The log records the roster, not the movement. Fourteen players
        // walking around change their positions every single pass, and a file
        // that re-states all of them ten times a second records that the clock
        // is running. What is worth finding afterwards is who was present, on
        // which side, and whether they were alive — so that is the key.
        let roster = roster(me, &players);
        let due_report = started >= next_pace;
        Stages::time(&mut stages.log, || {
            if last_logged.as_ref() != Some(&roster) {
                for line in records(me, &players) {
                    log.record(&line);
                }
                last_logged = Some(roster);
            }

            // Whether the loop is keeping up belongs in the file, not only on
            // the screen: the screen is gone by the time anyone asks why the
            // boxes lagged. The breakdown is one frame behind as a result,
            // which is the price of measuring the thing that reports it.
            if due_report {
                for line in pace.report() {
                    log.record(&line);
                }
            }
        });
        if due_report {
            next_pace = started + PACE_REPORT;
        }

        pace.finish(started.elapsed(), stages, started);
    }
}

/// Where a frame's time went, one named stage at a time.
///
/// A single number for the whole pass says only whether it is keeping up, not
/// which part to work on. That was the defect the old product carried: a
/// measurement that could attribute a cost to a phase and no further, so the
/// investigation stopped there and refining it meant restructuring first.
///
/// The rule that follows: nothing is a stage until it has a name.
#[derive(Clone, Copy, Default)]
struct Stages {
    me: Duration,
    players: Duration,
    matrix: Duration,
    overlay: Duration,
    /// Writing to the log. Measured because it is file I/O in the middle of
    /// the frame, and the one part of the loop still unaccounted for when the
    /// stages added up to far less than the frame did.
    log: Duration,
    /// Reading the player's own mouse.
    hand: Duration,
    /// Reading the view angle, which is this step's evidence.
    angles: Duration,
    /// Choosing a target and sending the movement. Named separately from
    /// everything else because it is the one stage whose effect leaves this
    /// process.
    steer: Duration,
    /// Reading the overlay's own message queue.
    pump: Duration,
    /// Keeping the overlay over the game and on top of it. Asks the window
    /// manager to move a window every frame, which is not obviously cheap —
    /// and was four fifths of the overlay's time before it had a name.
    follow: Duration,
    /// Reads made this frame. The other currency: each is a system call into
    /// another process, and a stage can be slow either by doing expensive work
    /// or by doing many cheap reads.
    reads: u64,
    /// Mouse packets drained this frame. Reading the hand is the worst frame's
    /// largest stage more often than anything else, and without this there is
    /// no telling a flood of packets from a slow packet from a frame that was
    /// simply descheduled while it happened to be in there.
    packets: u32,
    /// What the overlay's own time went on. `overlay` above is one number for
    /// the whole of it, which is exactly as useful as one number for the whole
    /// frame was.
    drawing: FrameCost,
}

impl Stages {
    /// Run `work`, adding what it took to `slot`.
    fn time<T>(slot: &mut Duration, work: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let value = work();
        *slot = started.elapsed();
        value
    }

    /// Every stage, named. The one place a stage is listed.
    ///
    /// The total and the breakdown are both taken from here, because they were
    /// once two lists and one of them was missing an entry. A stage left out
    /// of the total is not simply absent from a sum: every other stage's share
    /// is then measured against a frame that is too short, so they all read
    /// high, and the missing one reads as more than the whole frame. The log
    /// printed `read hand 3.183 ms 228.1%` before this was one list.
    fn rows(self) -> [(&'static str, Duration); 10] {
        [
            ("read players", self.players),
            ("draw overlay", self.overlay),
            ("read matrix", self.matrix),
            ("read me", self.me),
            ("write log", self.log),
            ("read hand", self.hand),
            ("read angles", self.angles),
            ("steer", self.steer),
            ("pump messages", self.pump),
            ("follow window", self.follow),
        ]
    }

    fn total(self) -> Duration {
        self.rows().iter().map(|(_, took)| *took).sum()
    }

    /// One line per stage, worst first, with its share of the frame. Sorted so
    /// the line that matters is the one at the top rather than the one whose
    /// name comes first alphabetically.
    fn breakdown(self) -> Vec<String> {
        let total = self.total().as_secs_f64().max(1e-9);
        let mut rows = self.rows();
        rows.sort_by_key(|(_, took)| std::cmp::Reverse(*took));

        let mut lines = vec![format!(
            "  {:<14} {:>7.3} ms   {} reads   {} packets",
            "stages total",
            total * 1000.0,
            self.reads,
            self.packets
        )];
        lines.extend(rows.iter().map(|(name, took)| {
            let ms = took.as_secs_f64() * 1000.0;
            format!(
                "  {name:<14} {ms:>7.3} ms   {:>4.1}%",
                ms / (total * 1000.0) * 100.0
            )
        }));
        lines.extend(self.drawing.breakdown());
        lines
    }
}

/// Holds the loop to a rate, and remembers whether it managed it.
///
/// Paced by deadline, not by delay: sleeping a fixed amount *after* the work
/// makes the period the work plus the sleep plus whatever the operating system
/// adds, so the achieved rate is always well under the target and the miss
/// grows with the work. Sleeping only the remainder of the frame keeps the
/// period at the target until the work alone exceeds it.
///
/// This is the one defect the old product carried its whole life, worth about
/// a quarter of its rate. Not repeating it costs two lines.
#[derive(Clone, Copy, Default)]
struct Pace {
    /// How long the work took last frame.
    last: Duration,
    /// The whole period between one frame starting and the next — work, sleep,
    /// sleep overshoot and whatever the scheduler took. Computing a rate from
    /// the target instead would print the target back and never show a miss,
    /// which is the same class of error as pacing by delay.
    period: Duration,
    /// The worst frame since the last report. Reset each time it is read, so
    /// one stall at startup does not hide every stall after it.
    worst: Duration,
    /// What that worst frame was doing.
    ///
    /// Without this the report describes whichever frame happened to land on
    /// the two-second mark — an ordinary one — and says nothing about the one
    /// that actually missed. Chasing a tail means sampling the tail.
    worst_stages: Stages,
    /// When the previous frame began, for measuring the period.
    previous_start: Option<Instant>,
}

impl Pace {
    fn finish(&mut self, elapsed: Duration, stages: Stages, started: Instant) {
        self.period = self.previous_start.map_or(Duration::ZERO, |previous| {
            started.saturating_duration_since(previous)
        });
        self.previous_start = Some(started);
        self.last = elapsed;
        if elapsed > self.worst {
            self.worst = elapsed;
            self.worst_stages = stages;
        }
        if let Some(remaining) = FRAME.checked_sub(elapsed) {
            std::thread::sleep(remaining);
        }
    }

    /// What the loop is actually achieving, and what it could achieve if it
    /// never slept — the second is what says whether the target is realistic.
    ///
    /// Read-only, because the live readouts call this every frame.
    fn describe(&self) -> String {
        let work = self.last.as_secs_f64().max(1e-9);
        let period = self.period.as_secs_f64();
        let rate = if period > 0.0 {
            format!("{:.1} Hz", 1.0 / period)
        } else {
            "— Hz".to_owned()
        };
        format!(
            "{rate}   work {:.2} ms (worst {:.2})   ceiling {:.0} Hz",
            self.last.as_secs_f64() * 1000.0,
            self.worst.as_secs_f64() * 1000.0,
            1.0 / work,
        )
    }

    /// The report for the log: the summary, then the worst frame's breakdown.
    /// Starts a new worst-case window.
    ///
    /// Only the log resets it. A worst case that never resets is dominated
    /// forever by the first frame, where the window is still being created;
    /// one that resets on every read — which is what the live readouts would
    /// do — never accumulates anything at all.
    fn report(&mut self) -> Vec<String> {
        let mut lines = vec![self.describe(), "  worst frame:".to_owned()];
        lines.extend(self.worst_stages.breakdown());
        self.worst = Duration::ZERO;
        self.worst_stages = Stages::default();
        lines
    }
}

/// Steering the view onto someone while a key is held.
///
/// Holds no reading of its own. Everything it decides from is passed in, so
/// what it decides can be reasoned about from the log alone: the same inputs
/// on the same pass produce the same line.
#[derive(Clone, Copy, Default)]
struct Aim {
    active: bool,
    /// Held, but the game is not the window in front.
    blocked: bool,
    /// Which pawn is being steered towards. In the log so that a target
    /// swapping back and forth between two enemies is visible as a swap
    /// rather than as a view that will not settle.
    target: Option<usize>,
    offset: Offset,
    /// Counts sent since the key went down, one figure per axis.
    sent: [i64; 2],
    /// Sends Windows would not accept. Elevation exists to keep this at zero.
    refused: u64,
    /// Why nothing is happening, when nothing is happening.
    ///
    /// Every refusal has its own name. A count that never moves off one of
    /// them is how a rule that has quietly become impossible shows itself —
    /// which is the failure the old product carried for fifteen versions
    /// without anyone noticing.
    reason: Refusal,
}

/// What stopped the view from being steered this pass, if anything did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Refusal {
    #[default]
    NotHeld,
    NotInFront,
    NoLocalPlayer,
    ViewImplausible,
    NoPositionForUs,
    NoEnemyInTheCone,
    AlreadyOnTarget,
    WindowsRefused,
    Steering,
}

impl Refusal {
    const fn label(self) -> &'static str {
        match self {
            Self::NotHeld => "idle",
            Self::NotInFront => "held, but the game is not in front",
            Self::NoLocalPlayer => "no plausible reading of us",
            Self::ViewImplausible => "view angles implausible — stale offsets?",
            Self::NoPositionForUs => "our own position is not readable",
            Self::NoEnemyInTheCone => "no living enemy in the cone",
            Self::AlreadyOnTarget => "on target",
            Self::WindowsRefused => "Windows refused the movement — not elevated?",
            Self::Steering => "steering",
        }
    }
}

impl Aim {
    /// What the log is keyed on: a line is written when any of this changes.
    const fn state(&self) -> (bool, bool, Option<usize>, Refusal) {
        (self.active, self.blocked, self.target, self.reason)
    }

    fn steer(
        &mut self,
        allowed: bool,
        me: Option<LocalPlayer>,
        players: &[Player],
        angles: ViewAngles,
    ) {
        let held = input::held(AIM_KEY);
        self.blocked = held && !allowed;
        let active = held && allowed;
        // Each hold counts from zero, so the figures describe this press and
        // not every press since the program started.
        if active && !self.active {
            self.sent = [0; 2];
        }
        self.active = active;
        self.target = None;
        self.offset = Offset::default();

        self.reason = match self.aim_at(active, held, me, players, angles) {
            Err(refusal) => refusal,
            Ok(counts) => {
                if input::move_by(counts[0], counts[1]) {
                    self.sent[0] += i64::from(counts[0]);
                    self.sent[1] += i64::from(counts[1]);
                    Refusal::Steering
                } else {
                    self.refused += 1;
                    Refusal::WindowsRefused
                }
            }
        };
    }

    /// The decision, with every way of declining to make one named.
    ///
    /// Separated from the sending so that the reasons read as one list rather
    /// than as a staircase of early returns with the movement at the bottom.
    fn aim_at(
        &mut self,
        active: bool,
        held: bool,
        me: Option<LocalPlayer>,
        players: &[Player],
        angles: ViewAngles,
    ) -> Result<[i32; 2], Refusal> {
        if !active {
            return Err(if held {
                Refusal::NotInFront
            } else {
                Refusal::NotHeld
            });
        }
        // Our own side is what every enemy test is made against, so a bad
        // reading of it would make targets of teammates.
        let me = me
            .filter(|me| me.plausible())
            .ok_or(Refusal::NoLocalPlayer)?;
        if !angles.plausible() {
            return Err(Refusal::ViewImplausible);
        }
        let eye = players
            .iter()
            .find(|player| player.pawn == me.pawn)
            .and_then(|player| player.origin)
            .map(eyes)
            .ok_or(Refusal::NoPositionForUs)?;

        // The one nearest to where the player is already pointing. Measured as
        // an angle rather than as a distance on screen, because a target at
        // the edge of the view is further away than the same gap in pixels
        // near the middle, and it is the angle the movement has to cover.
        let target = players
            .iter()
            .filter(|player| {
                player.pawn != me.pawn
                    && player.plausible()
                    && me.team.opposes(player.team)
                    && player.alive()
            })
            .filter_map(|player| {
                let desired = aim::look_at(eye, eyes(player.origin?))?;
                let offset = aim::offset(angles, desired);
                STEERING
                    .within_cone(offset)
                    .then_some((player.pawn, offset))
            })
            .min_by(|(_, a), (_, b)| a.size().total_cmp(&b.size()));

        let (pawn, offset) = target.ok_or(Refusal::NoEnemyInTheCone)?;
        self.target = Some(pawn);
        self.offset = offset;
        STEERING.counts(offset).ok_or(Refusal::AlreadyOnTarget)
    }

    fn describe(&self) -> String {
        let mut line = format!("aim {}", self.reason.label());
        if let Some(pawn) = self.target {
            line += &format!(
                "   target 0x{pawn:X}   off {:.2} deg (yaw {:+.2} pitch {:+.2})",
                self.offset.size(),
                self.offset.yaw,
                self.offset.pitch
            );
        }
        if self.sent != [0; 2] {
            line += &format!("   sent {:+} {:+}", self.sent[0], self.sent[1]);
        }
        if self.refused > 0 {
            line += &format!("   {} refused", self.refused);
        }
        line
    }
}

/// A player's eyes, from the feet their origin records.
fn eyes(origin: [f32; 3]) -> [f32; 3] {
    [origin[0], origin[1], origin[2] + EYE_HEIGHT]
}

/// How tall a standing player is, in world units.
///
/// ponytail: one constant. Crouching makes a player shorter and this will draw
/// a box too tall for them; read the real bounds when that starts to matter.
const PLAYER_HEIGHT: f32 = 72.0;

/// How wide a box is as a fraction of its own height on screen. Deriving the
/// width from the height keeps the box the same shape at every distance, which
/// a fixed pixel width would not.
const BOX_ASPECT: f32 = 0.45;

/// Draw a box around each living enemy.
///
/// Only enemies, only living ones, and only those the projection places in
/// front of the camera. Every one of those is a refusal to draw rather than a
/// guess, because a box drawn on a teammate is worse than no box at all.
fn draw_overlay(
    overlay: &mut Overlay,
    view: ViewMatrix,
    me: Option<LocalPlayer>,
    players: &[Player],
    readouts: &Readouts<'_>,
) -> FrameCost {
    let bounds = overlay.bounds();
    overlay.frame(|canvas| {
        // Nothing is drawn from a reading that failed its own check. Our own
        // side is the reference every enemy test is made against, so a bad
        // reading of it turns everyone into a target — teammates included.
        let Some(me) = me.filter(|me| me.plausible()) else {
            canvas.text(12, 12, "readings implausible — stale offsets?", WARN);
            return;
        };

        let mut drawn = 0usize;
        let mut rejected = 0usize;
        for player in players {
            if !player.plausible() {
                rejected += 1;
                continue;
            }
            if !me.team.opposes(player.team) || !player.alive() {
                continue;
            }
            let Some(feet) = player.origin else { continue };
            let head = [feet[0], feet[1], feet[2] + PLAYER_HEIGHT];

            // Both ends must be in front of the camera. Projecting only one
            // and guessing the other is how a box ends up stretched across
            // the whole screen when a player is half behind us.
            let (Some(bottom), Some(top)) = (
                view.project(feet, bounds.width, bounds.height),
                view.project(head, bounds.width, bounds.height),
            ) else {
                continue;
            };

            let height = bottom.1 - top.1;
            if height <= 0 {
                continue;
            }
            let width = (height as f32 * BOX_ASPECT).round() as i32;
            canvas.rect(top.0 - width / 2, top.1, width, height, ENEMY, 2);
            drawn += 1;
        }

        let mut status = vec![
            format!("echo  {}x{}", bounds.width, bounds.height),
            format!("{} health {}", me.team.label(), me.health),
            format!("{drawn} enemies on screen of {}", players.len()),
        ];
        status.extend(readouts.lines());

        // Readings that failed their check are called out rather than being
        // silently skipped: a count that climbs is what a game update looks
        // like from here. Kept in their own list so the warning colour follows
        // the warnings, rather than every line past a counted-out row.
        let mut rows: Vec<(String, COLORREF)> =
            status.into_iter().map(|line| (line, TEXT)).collect();
        if rejected > 0 {
            rows.push((format!("{rejected} implausible — stale offsets?"), WARN));
        }
        for (row, (line, colour)) in rows.iter().enumerate() {
            canvas.text(12, 12 + row as i32 * 18, line, *colour);
        }
    })
}

/// What the corner of the screen says about the run itself, rather than about
/// anything in the world.
struct Readouts<'a> {
    pace: Pace,
    hand: [i64; 2],
    mouse: Option<&'a RawMouse>,
    aim: Aim,
}

impl Readouts<'_> {
    fn lines(&self) -> Vec<String> {
        vec![
            self.pace.describe(),
            match self.mouse {
                Some(mouse) => format!(
                    "hand {:>6} {:>6}   {} packets{}",
                    self.hand[0],
                    self.hand[1],
                    mouse.packets(),
                    if mouse.absolute() > 0 {
                        format!("   {} absolute — unreadable device", mouse.absolute())
                    } else {
                        String::new()
                    }
                ),
                None => "no raw mouse".to_owned(),
            },
            self.aim.describe(),
        ]
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
