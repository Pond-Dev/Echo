//! Echo — closing the angle, once.
//!
//! Fire, and the view is pulled towards the nearest enemy in front of you.
//! The moment it is anywhere inside them the pull is finished and the last of
//! it is yours, and it stays yours until the trigger is let go and pressed
//! again. One press, one pull. Move the mouse during it and the grip eases
//! off; it never has to be let go of first, and it never argues.
//!
//! On the trigger, so the first bullet is always unhelped — the game fires on
//! the press, and nothing outside it moves the view before that. And nothing
//! here knows what is in the player's hands, so a grenade, a knife or a click
//! in the buy menu pull the view exactly as a rifle does.
//!
//! That rule is asked again every pass, and it is never told that a hold has
//! begun. Both matter. The product this one replaces decided at the moment of
//! the press, so pressing while the hand was moving — which is what everyone
//! does — skipped the entire hold; it was measured happening six times in
//! every thirty seconds of play. Here the same press costs the settle time
//! and nothing else.
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

use echo::aim::{Grip, Offset, RAMP, Refusal, STEERING, Situation};
use echo::game::{Game, LocalPlayer, Player, Team, ViewAngles, ViewMatrix};
use echo::input::{self, RawMouse};
use echo::log::Log;
use echo::overlay::{FrameCost, Overlay, rgb};
use echo::process::AttachError;
use windows::Win32::Foundation::COLORREF;
use windows::Win32::UI::Input::KeyboardAndMouse::{VIRTUAL_KEY, VK_LBUTTON};
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
/// Held to steer — the trigger.
///
/// Which makes the pull part of shooting rather than something done before
/// it, and only works at all because the pull now ends once: an assist that
/// held on through the trigger would be fighting the recoil the player is
/// compensating by hand, which is the failure recorded as K1.
///
/// **The first bullet can never be helped.** The game fires on the press and
/// nothing outside it can move the view before that; the pull begins after
/// the shot has left. What it reaches is the second bullet onwards.
///
/// **Nothing here knows what is in the player's hands.** A grenade is thrown
/// by releasing this button, so the pull lands in the middle of a lineup;
/// a knife and a click in the buy menu do the same. Reading the active
/// weapon is its own step and this wants it.
///
/// Read straight from the keyboard state rather than from the raw input this
/// program already receives. The two would disagree on the frame the button
/// goes down, and the one that matters is the one the game is acting on.
const AIM_KEY: VIRTUAL_KEY = VK_LBUTTON;

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
        // `None` rather than zero when there is no mouse to read. A hand that
        // cannot be read is not a hand at rest, and the difference is the
        // whole of whether the player can push the assist off.
        let (hand, packets) = Stages::time(&mut stages.hand, || {
            mouse.as_mut().map_or((None, 0), |mouse| {
                let packets = mouse.poll();
                (Some(mouse.take()), packets)
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
            // One pass stale, since the period is closed at the end of a
            // pass and this is the middle of the next. A few hundred
            // microseconds against a settle measured in tens of milliseconds.
            aim.steer(allowed, me, &players, angles, hand, pace.period);
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
                log.record(&aim.tally());
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
    /// Run `work`, and record what it took in `slot`.
    ///
    /// Recorded, not added: timing twice into one slot keeps the second
    /// reading only. Which is fine while every stage is timed once, and is
    /// why that is worth saying — a stage split in two and timed twice would
    /// under-report itself, and since every share is measured against the
    /// total, every other stage would read high. That is the failure the one
    /// list of stages exists to prevent, coming back through the helper.
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
    /// How firmly the view is held. Asked every pass, kept here rather than
    /// rebuilt, because what it knows is where along the ramp it has got to
    /// and that does not belong to any one pass.
    grip: Grip,
    /// How far away the target was, in world units. Kept so the offset can
    /// be read as a length on a chest rather than as an angle, which cannot
    /// be read at all without it.
    distance: f32,
    /// Whether the pull has finished during this press.
    ///
    /// Belongs to the press, so it lives here rather than in the deciding —
    /// which is never told that a press began, and must not be, since that is
    /// the one thing the old product decided on and got wrong for years.
    delivered: bool,
    /// Whether the mouse could be read at all this pass. Kept for the
    /// readout, because a run where it never can is one where nothing works
    /// and the reason is one line at startup otherwise.
    hand_readable: bool,
    /// Passes this hold, and how many of them the hand was moving through.
    ///
    /// The share is what says whether the ramp is set anywhere near right. A
    /// hold the hand pushes against the whole way is an assist that never
    /// helps; one it never pushes against is an assist that never lets go.
    passes: u32,
    pushed: u32,
    /// Why nothing is happening, when nothing is happening.
    reason: Refusal,
    /// How many passes ended in each refusal, since the program started.
    ///
    /// The log records changes of state, which cannot tell a reason that
    /// never happens from a reason that is never written down. A count that
    /// sits at zero for a whole session is a discovery — a rule that has
    /// quietly become impossible looks exactly like that, and is the failure
    /// the old product carried for fifteen versions without anyone noticing.
    ///
    /// So the whole tally is printed every time, zeros and all. A line that
    /// only lists what happened is the same silence in a shorter form.
    tally: [u32; Refusal::COUNT],
}

impl Aim {
    /// What the log is keyed on: a line is written when any of this changes.
    ///
    /// Steering and sitting on a target used to be collapsed together here,
    /// because a target on foot crossed the line between them several times a
    /// second and wrote three hundred and forty-seven lines for fifteen
    /// seconds of play. Handing over once a press removed the crossing rather
    /// than papering over it, so there is nothing left to collapse.
    const fn state(&self) -> (bool, bool, Option<usize>, Refusal) {
        (self.active, self.blocked, self.target, self.reason)
    }

    fn steer(
        &mut self,
        allowed: bool,
        me: Option<LocalPlayer>,
        players: &[Player],
        angles: ViewAngles,
        hand: Option<[i64; 2]>,
        elapsed: Duration,
    ) {
        let held = input::held(AIM_KEY);
        self.blocked = held && !allowed;
        let active = held && allowed;
        // Each hold counts from zero, so the figures describe this press and
        // not every press since the program started.
        if active && !self.active {
            self.sent = [0; 2];
            self.target = None;
            self.offset = Offset::default();
            self.passes = 0;
            self.pushed = 0;
            self.delivered = false;
        }
        self.active = active;

        // A hand nobody can read counts as a hand pushing as hard as it can.
        // The other way round is the dangerous way: a failed registration
        // would read as a hand at rest for the whole session, the grip would
        // climb to full and stay there, and no amount of mouse movement would
        // ever reduce it — the assist gripping hardest exactly when there is
        // no way to take it back.
        let push = hand.map_or(1.0, |hand| RAMP.push(hand, elapsed));
        // Moved every pass and unconditionally, so the strength between holds
        // decays to nothing and the next press has to earn it back. Leaving
        // it alone while the button is up would have a second press seize the
        // view at whatever the first one ended on.
        self.hand_readable = hand.is_some();
        // Delivered counts as not engaged, so the grip decays afterwards and
        // the next press has to earn it back from nothing like any other.
        let grip = self
            .grip
            .update(active && !self.delivered, push, elapsed, RAMP);
        if active {
            self.passes += 1;
            self.pushed += u32::from(push > 0.0);
        }

        let choice = STEERING.choose(
            &Situation {
                held,
                in_front: allowed,
                hand,
                me,
                players,
                angles,
                delivered: self.delivered,
            },
            grip,
            push,
        );
        // Kept even when nothing was sent, so a line about the player taking
        // the view back still says which enemy it was taken from — except on
        // the pass the button comes up, which leaves the last hold's figures
        // standing because that is the one moment anyone wants them.
        if held {
            self.target = choice.target;
            self.offset = choice.offset;
            self.distance = choice.distance;
        }
        // Latched, never unlatched until the next press. Asking again each
        // pass would hand the view back the instant the target moved off it,
        // which is the tracking this deliberately does not do.
        self.delivered |= choice.arrived;
        let reason = match choice.counts {
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
        self.reason = reason;
        self.tally[reason.slot()] += 1;
    }

    /// Every refusal and how often it has happened, zeros included.
    fn tally(&self) -> String {
        Refusal::ALL
            .iter()
            .fold("aim tally".to_owned(), |line, refusal| {
                line + &format!(" {}={}", refusal.key(), self.tally[refusal.slot()])
            })
    }

    fn describe(&self) -> String {
        let mut line = format!("aim {}", self.reason.label());
        if let Some(pawn) = self.target {
            // The same miss twice: as the angle the steering works in, and as
            // the length on a chest that says whether a shot would have
            // landed. An angle on its own says neither.
            let across = self.distance * self.offset.size().to_radians().tan();
            line += &format!(
                "   target 0x{pawn:X}   off {:.2} deg = {across:.0}u of {:.0}u at {:.0}u  (yaw {:+.2} pitch {:+.2})",
                self.offset.size(),
                STEERING.body_half_width,
                self.distance,
                self.offset.yaw,
                self.offset.pitch
            );
        }
        if self.sent != [0; 2] {
            line += &format!("   sent {:+} {:+}", self.sent[0], self.sent[1]);
        }
        if self.active || self.passes > 0 {
            line += &format!("   grip {:.0}%", self.grip.firmness() * 100.0);
        }
        if !self.hand_readable {
            line += "   no mouse to read";
        }
        if let Some(share) = (self.pushed * 100).checked_div(self.passes) {
            line += &format!("   hand {share}% of {} passes", self.passes);
        }
        if self.refused > 0 {
            line += &format!("   {} refused", self.refused);
        }
        line
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
    hand: Option<[i64; 2]>,
    mouse: Option<&'a RawMouse>,
    aim: Aim,
}

impl Readouts<'_> {
    fn lines(&self) -> Vec<String> {
        vec![
            self.pace.describe(),
            match (self.mouse, self.hand) {
                (Some(mouse), Some(hand)) => format!(
                    "hand {:>6} {:>6}   {} packets{}",
                    hand[0],
                    hand[1],
                    mouse.packets(),
                    if mouse.absolute() > 0 {
                        format!("   {} absolute — unreadable device", mouse.absolute())
                    } else {
                        String::new()
                    }
                ),
                _ => "no raw mouse — the assist will not steer".to_owned(),
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
