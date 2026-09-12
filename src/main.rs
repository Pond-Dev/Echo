//! Echo — the step that sends the mouse somewhere.
//!
//! Every step before this one read or drew, and being wrong cost a box in the
//! wrong place. This one moves the view, so being wrong drags someone's aim
//! off target mid-round. It is therefore the smallest movement that can still
//! be seen: hold one key and the view turns steadily to the right, at a
//! constant rate, until the key is let go. Nothing aims at anything.
//!
//! Two guards, both about not touching what we were not invited to touch:
//! nothing is sent unless the game is the window in front, and nothing is
//! sent unless the key is down this instant.
//!
//! The check is the turn readout. Windows moving a pointer would prove only
//! that Windows moved a pointer — the view angle changing is what says the
//! counts arrived inside CS2. Hold the key and the figure climbs; let go and
//! it stops.
//!
//! Watch the hand readout while holding it, too. It counts our own movement
//! as well as yours, because Windows hands injected input to raw input like
//! any other. Telling those apart is a later step, and nothing here tries to.

use std::time::{Duration, Instant};

use echo::game::{Game, LocalPlayer, Player, Team, ViewAngles, ViewMatrix};
use echo::input::{self, RawMouse};
use echo::log::Log;
use echo::overlay::{FrameCost, Overlay, rgb};
use echo::process::AttachError;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_INSERT};
use windows::core::w;

/// Target frame time. The game draws far faster than this, so a box is always
/// a little behind; the gap is what makes it float when the view swings.
const FRAME: Duration = Duration::from_millis(8);
/// How often to look for the game's window again while there is no overlay.
const ATTACH_RETRY: Duration = Duration::from_millis(500);
/// How often the achieved rate is written to the log.
const PACE_REPORT: Duration = Duration::from_secs(2);
const GAME_WINDOW: windows::core::PCWSTR = w!("Counter-Strike 2");
/// Held to send movement. Chosen for being bound to nothing in CS2, so the
/// only thing that happens while it is down is the thing under test.
const NUDGE_KEY: VIRTUAL_KEY = VK_INSERT;
/// Counts sent per frame while that key is held. Small enough that the turn
/// is a drift rather than a jump — a jump would prove the same thing and be a
/// much worse thing to be surprised by.
const NUDGE: i32 = 4;
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

    let mut nudge = Nudge::default();
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

        // Read for this step alone, and read here rather than with the others
        // because it is measured against a movement sent a moment later.
        let angles = Stages::time(&mut stages.angles, || game.view_angles())?;
        // Focus is a guard, not a preference. With the game behind something
        // else, the same counts drag the pointer across whatever is in front.
        let allowed = overlay.as_ref().is_some_and(Overlay::target_has_focus);
        let before = (nudge.sending, nudge.blocked);
        Stages::time(&mut stages.nudge, || nudge.send(allowed, angles.yaw));
        // Both edges of a hold, and nothing in between. The start says where
        // the view was pointing and the end says what the counts bought —
        // which is this step's whole evidence, and a readout on a screen that
        // has since been closed cannot be asked about it afterwards.
        if (nudge.sending, nudge.blocked) != before {
            log.record(&nudge.describe());
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
                        nudge,
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
    /// Sending movement. Named separately from everything else because it is
    /// the one stage whose effect leaves this process.
    nudge: Duration,
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
            ("send nudge", self.nudge),
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

/// Pushing the view sideways while a key is held, and the evidence that it
/// worked.
///
/// The counts sent and the angle turned are kept side by side on purpose. One
/// is what we asked for and the other is what the game did, and this step is
/// finished when the second answers the first. How much turn a count buys is
/// a question for the step that has somewhere to aim.
#[derive(Clone, Copy)]
struct Nudge {
    sending: bool,
    /// Why not, when not. A key held while the game sits behind a browser has
    /// to read as refused rather than as idle, or the readout makes it look
    /// as though sending is broken.
    blocked: bool,
    /// Counts sent since the key went down.
    sent: i64,
    /// Sends Windows would not accept. Elevation exists to keep this at zero.
    refused: u64,
    /// Where the view was pointing at the previous send.
    previous: ViewAngles,
    /// The whole turn since the key went down, added up a frame at a time.
    ///
    /// Not the difference between where the view started and where it is now.
    /// Two angles cannot tell a turn of 11 degrees from one of 371, and a log
    /// recorded exactly that: 3976 counts sent, 11 degrees reported, when the
    /// counts before it had bought a degree every thirty. One frame's step is
    /// far too small to be mistaken for a longer one, so adding the steps up
    /// has no such ceiling.
    turned: f32,
}

impl Default for Nudge {
    fn default() -> Self {
        Self {
            sending: false,
            blocked: false,
            sent: 0,
            refused: 0,
            previous: ViewAngles {
                pitch: 0.0,
                yaw: 0.0,
            },
            turned: 0.0,
        }
    }
}

impl Nudge {
    fn send(&mut self, allowed: bool, yaw: f32) {
        let down = input::held(NUDGE_KEY);
        self.blocked = down && !allowed;
        let sending = down && allowed;

        let now = ViewAngles { pitch: 0.0, yaw };
        // Each hold is measured on its own. Carrying the totals across holds
        // would leave the turn figure describing several presses at once, so
        // a run of small holds would read as one large one.
        if sending && !self.sending {
            self.sent = 0;
            self.turned = 0.0;
            // No step on the first frame: there is no earlier reading to
            // measure one against, and the last hold's would credit this one
            // with whatever the player did in between.
            self.previous = now;
        }
        self.sending = sending;
        if !sending {
            return;
        }

        // Added before this frame's send rather than after it, because what
        // has moved the view so far is every send before this one. Crediting
        // a turn to a movement the game has not seen yet would be the readout
        // agreeing with itself.
        self.turned += now.turn_from(self.previous);
        self.previous = now;

        if input::move_by(NUDGE, 0) {
            self.sent += i64::from(NUDGE);
        } else {
            self.refused += 1;
        }
    }

    fn describe(&self) -> String {
        let state = match (self.sending, self.blocked) {
            (true, _) => "sending",
            (_, true) => "held, but the game is not in front",
            _ => "idle",
        };
        // The ratio is what the next step needs and what says whether this
        // one worked: a turn without counts behind it is the player's hand,
        // and counts without a turn are movement the game never saw.
        let per_count = if self.sent == 0 {
            String::new()
        } else {
            format!("   {:+.4} deg/count", self.turned / self.sent as f32)
        };
        format!(
            "nudge {state}   sent {} counts   turned {:+.1} deg{per_count}{}",
            self.sent,
            self.turned,
            if self.refused > 0 {
                format!("   {} refused — not elevated?", self.refused)
            } else {
                String::new()
            }
        )
    }
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
    nudge: Nudge,
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
            self.nudge.describe(),
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
