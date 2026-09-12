//! Deciding where to point, and how far to move to get there.
//!
//! The first module that decides something rather than reading or drawing it,
//! and so the first with tests worth writing. Nothing in here touches the game
//! or the mouse: it takes positions and angles in, and gives an answer out.
//! That is the point — a decision that can only be watched is a decision that
//! can only be argued about.
//!
//! It steers by feedback rather than by calculation. Each pass asks where the
//! view is now, where it should be, and moves a fraction of the difference;
//! the next pass asks again. Nothing has to be exactly right for that to
//! arrive — a movement that falls short is made up next pass, and the one
//! number that converts degrees into mouse counts can be well off before the
//! behaviour changes from "arrives quickly" to "arrives slowly".
//!
//! It closes the angle once and then stops. The view is pulled towards the
//! target's chest, and the moment it is anywhere inside them the pull is
//! finished and the last of it belongs to the player — until the button is
//! let go and pressed again. That is a choice about the game this
//! is for: a CS2 duel is decided by where the first bullet goes, spray
//! patterns are learned by hand and an assist holding through one is fighting
//! what the player practised, and inaccuracy while moving keeps duels down to
//! a few hundred milliseconds. There is no long window to track through.
//!
//! Handing over anywhere on the body rather than on the head is the
//! difference the whole product is named for. A pull that ends on the head
//! has done the aiming; a pull that ends on the chest has closed the angle
//! and left the aiming — the shot connects, and whether it kills is still the
//! player's. It also needs no rule about distance: a body subtends less angle
//! the further away it is, so the further the shot — the harder it is, and
//! the more obvious an assist would be — the less this touches.
//!
//! The point aimed at and the width handed over at have to describe the same
//! shape, and once did not. The pull aimed at the head and handed over at the
//! width of the body, so it would stop a body's half-width to one side of a
//! head — which is air, since a player is only as wide as their shoulders
//! down at the chest and far narrower at the head. A log caught it exactly:
//! fifteen presses in three seconds, every one answered "delivered" without
//! moving anything, every one a degree and a bit off.
//!
//! Nothing in here switches on or off. Every edge is a curve, in both of the
//! places an edge would otherwise be felt. A movement asked for approaches
//! its ceiling instead of striking it, so there is no distance at which the
//! assist stops accelerating and starts coasting. And the strength it steers
//! with rises and falls over time rather than between two passes, so pushing
//! against it meets a grip that eases off rather than one that lets go.

use std::time::Duration;

use crate::game::{LocalPlayer, Player, ViewAngles};

/// How far the view is from where it should be, on each axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Offset {
    /// Positive is anticlockwise, which is leftward — the direction the engine
    /// counts yaw, and the opposite of the direction a mouse count moves it.
    pub yaw: f32,
    /// Positive is downward, as the engine stores pitch.
    pub pitch: f32,
}

impl Offset {
    /// How far off the view is overall, for comparing one target to another.
    pub fn size(self) -> f32 {
        self.yaw.hypot(self.pitch)
    }
}

/// The angles that point an eye at a place in the world.
///
/// `None` when the two are the same point, which has no direction rather than
/// an obvious one.
pub fn look_at(eye: [f32; 3], target: [f32; 3]) -> Option<ViewAngles> {
    let [dx, dy, dz] = [target[0] - eye[0], target[1] - eye[1], target[2] - eye[2]];
    let flat = dx.hypot(dy);
    if !flat.is_finite() || !dz.is_finite() || (flat == 0.0 && dz == 0.0) {
        return None;
    }
    Some(ViewAngles {
        // Negated because the engine stores pitch positive downwards, while
        // the world's z counts upwards.
        pitch: -dz.atan2(flat).to_degrees(),
        yaw: dy.atan2(dx).to_degrees(),
    })
}

/// How far `current` is from pointing at `desired`.
pub fn offset(current: ViewAngles, desired: ViewAngles) -> Offset {
    Offset {
        // The short way round, so a target across the seam where yaw wraps is
        // a small correction and not most of a circle in the wrong direction.
        yaw: desired.turn_from(current),
        pitch: desired.pitch - current.pitch,
    }
}

/// How the view is steered: how hard, how fast, and when to stop.
#[derive(Clone, Copy, Debug)]
pub struct Steering {
    /// Mouse counts that turn the view one degree.
    ///
    /// A property of the player's sensitivity, not of the game. Being wrong
    /// about it changes how quickly the view arrives and not where it arrives,
    /// because every pass measures the remaining distance again.
    pub counts_per_degree: f32,
    /// The share of the remaining distance to cover each pass.
    ///
    /// Under one, so each pass leaves something for the next and the view
    /// settles. At one it would arrive in a single pass if every number were
    /// exact, and past one it would overshoot further every pass.
    pub gain: f32,
    /// The ceiling one pass approaches but never reaches, on one axis.
    ///
    /// The limit on what a wrong reading can do: a position caught mid-write
    /// asks for a movement of any size at all, and without a ceiling the view
    /// is somewhere else before the next pass can disagree.
    ///
    /// Approached rather than enforced. A hard limit is a corner — small
    /// movements grow with the distance and large ones all come out the same
    /// size, and the distance where that changes is felt as the assist going
    /// from accelerating to coasting. A curve that bends towards the ceiling
    /// has no such place in it.
    pub cap: f32,
    /// Below this many degrees the view is treated as already there.
    ///
    /// A target is not a point: it is a person, several degrees wide at the
    /// distances this matters at. Chasing the last fraction of a degree only
    /// trades one rounding error for another, every pass, which is a tremble
    /// rather than aim.
    pub deadzone: f32,
    /// How far above our own feet our own camera is, standing.
    ///
    /// Where the pull is measured *from*, and a fact about the game rather
    /// than a decision: the game's own position readout and the entity's
    /// origin differ by exactly this, which is what said the origin is feet
    /// and the readout is eyes.
    ///
    /// ponytail: standing only. Crouched it is about eighteen units lower.
    /// Read the pawn's own view offset when that starts to matter.
    pub eye_height: f32,
    /// How far above a target's feet the pull aims, standing.
    ///
    /// Where the pull is measured *to*, and a decision rather than a fact —
    /// which is why it is a separate figure from the one above even though
    /// both are heights above a pair of feet. The two were one for a while
    /// and the pull aimed at heads.
    ///
    /// The chest. Together with the handover width it makes a circle that
    /// lies inside the body wherever it stops, so a shot at the moment of
    /// handover connects. Aiming at the head cannot: the head sits at the top
    /// of the body, so half the circle around it is over the shoulders and
    /// into the sky.
    ///
    /// It also means the assist never hands the player a headshot. Closing
    /// the angle until the shot connects is help; putting the crosshair on
    /// the head is doing the aiming, and the product is named for the
    /// difference.
    pub aim_height: f32,
    /// Half the width of the part of a player a bullet registers on.
    ///
    /// What the pull is finished at, as an angle worked out against the
    /// target's distance rather than fixed: a body is over a degree wide at
    /// five hundred units and a third of one at two thousand, and a single
    /// angle would mean handing over short of the target up close and well
    /// past it at range.
    ///
    /// ponytail: an estimate. A standing player's bounding box is thirty-two
    /// units across, but the person inside it is narrower than the box, so
    /// handing over at the box's half-width parks the view on an edge where a
    /// shot grazes or misses. Twelve is a guess at the shoulders. Replace it
    /// with the real hitbox geometry, which the old product already has.
    pub body_half_width: f32,
    /// Targets further than this from where the player is already pointing are
    /// not targets.
    ///
    /// Someone behind you is not who you meant, and the movement to reach them
    /// would take the view off everything you can see on the way.
    pub cone: f32,
}

impl Steering {
    /// The mouse movement that closes `offset` at this much of full strength,
    /// or nothing when there is nothing worth sending.
    ///
    /// `grip` runs from nothing to one and scales what is sent after the
    /// ceiling rather than before it, so the ceiling always means the most a
    /// pass may move at full strength and not some fraction of it.
    pub fn counts(self, offset: Offset, grip: f32) -> Option<[i32; 2]> {
        if !offset.size().is_finite() || !grip.is_finite() || offset.size() < self.deadzone {
            return None;
        }
        let scale = self.counts_per_degree * self.gain;
        // Yaw is negated: a mouse count to the right turns the view right,
        // which the engine records as yaw falling. Pitch is not: a count
        // downwards looks downwards, which it records as pitch rising.
        let movement = [
            soften(-offset.yaw * scale, self.cap, grip),
            soften(offset.pitch * scale, self.cap, grip),
        ];
        (movement != [0, 0]).then_some(movement)
    }

    /// Whether something this far off is worth steering towards at all.
    pub fn within_cone(self, offset: Offset) -> bool {
        offset.size() <= self.cone
    }
}

/// Bent towards the ceiling, scaled by the grip, and rounded.
///
/// `tanh` because it is the curve that is already the identity for small
/// values and already the ceiling for large ones, with everything in between
/// bending between the two and no point anywhere along it where the slope
/// changes suddenly. A movement well inside the ceiling is left alone, which
/// is what keeps the approach to a target unchanged.
fn soften(counts: f32, cap: f32, grip: f32) -> i32 {
    if !counts.is_finite() || cap <= 0.0 {
        return 0;
    }
    (cap * (counts / cap).tanh() * grip.clamp(0.0, 1.0)).round() as i32
}

/// The tuned values, kept here rather than in the binary.
///
/// Not because a module should own a product's settings, but because the
/// binary cannot be tested at all: a suite living beside it would pass
/// identically at any value, which is the same as not checking them. What is
/// worth checking is not any single figure but the arithmetic between them —
/// the speed a push has to reach is three constants multiplied together, and
/// changing one of them silently moves it.
pub const STEERING: Steering = Steering {
    counts_per_degree: 51.0,
    gain: 0.35,
    // Five degrees a pass, which at this rate is a fast flick and not a spin.
    // What a reading caught mid-write is allowed to cost. Approached and
    // never reached, so there is no distance at which the assist stops
    // accelerating and starts coasting.
    cap: 250.0,
    deadzone: 0.15,
    body_half_width: 12.0,
    eye_height: 64.0,
    aim_height: 55.0,
    cone: 30.0,
};

/// How the strength rises and falls.
///
/// A speed, and not a number of counts in a pass, because counts in a pass
/// measure the same hand differently on a machine that runs the loop at a
/// different rate. Nothing under it is ignored either: everything between
/// nothing and this works against the rise in proportion, and the push that
/// exactly cancels it is [`Ramp::holding_push`] — about eleven degrees a
/// second here, which is a push rather than a correction.
///
/// The durations replace what was once a plain switch. A session's tally read
/// `hand=2853 steering=359`: the hand took three passes in four, and not
/// because it was steering the view for three quarters of the time but
/// because the assist was flickering on and off several times a second.
/// Nothing about that is visible as a decision — it is felt as a hard edge.
pub const RAMP: Ramp = Ramp {
    full_push: 750.0,
    // Short because the trigger is the button, and a shot is short. Measured:
    // presses ran fifty to a hundred milliseconds, and a rise of a hundred
    // and forty left the grip at a fifth to two fifths of full when the
    // button came back up — the pull covered about a degree of a five degree
    // gap and gave up. Thirty-four seconds of play held the view for less
    // than one of them.
    //
    // A pass covers a fifth of this, so the ease-in is five passes and the
    // curve is still a curve — but only just. Shortening it further would
    // meet the clamp that stops one pass crossing a whole ramp, and quietly
    // become the switch this replaced.
    rise: Duration::from_millis(40),
    fall: Duration::from_millis(120),
};

/// How the strength the assist steers with rises and falls.
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    /// Hand speed, in counts a second, that pushes the grip off at full rate.
    ///
    /// A speed, not a number of counts in a pass. Counts in a pass measure
    /// the same hand differently on a machine that runs the loop at a
    /// different rate — half as often is twice as many counts each time — so
    /// a threshold written that way is a different product on every machine.
    ///
    /// Nothing below it is ignored. A hand moving at a tenth of this pushes
    /// at a tenth of the rate and still gets the view, in ten times as long.
    /// A threshold would have had a speed under which the player could not
    /// take the view back at all, however long they pushed for, which is a
    /// worse thing than a slow handover.
    pub full_push: f32,
    /// From nothing to full strength, with the hand still.
    ///
    /// Also what a hold begins with, since the strength is at nothing between
    /// holds. So every press eases in rather than seizing the view, and a
    /// hand still moving when the button goes down only delays the rise.
    pub rise: Duration,
    /// From full strength to nothing, with the hand pushing at full rate.
    ///
    /// Pushing against the assist meets a grip that eases off over this,
    /// rather than one that lets go between two passes. The player still ends
    /// with the view — the difference is that the handover can be felt
    /// happening instead of arriving already done.
    pub fall: Duration,
}

impl Ramp {
    /// How hard the hand is pushing, from nothing to full.
    ///
    /// Per axis, not combined: a movement that is purely vertical is as
    /// deliberate as one that is not, and adding the two would let a diagonal
    /// of two small movements count as one large one.
    pub fn push(self, counts: [i64; 2], elapsed: Duration) -> f32 {
        let moved = counts[0].abs().max(counts[1].abs());
        let seconds = elapsed.as_secs_f32();
        if moved == 0 || seconds <= 0.0 || self.full_push <= 0.0 {
            return 0.0;
        }
        (moved as f32 / seconds / self.full_push).clamp(0.0, 1.0)
    }

    /// The push that exactly holds the grip where it is.
    ///
    /// Below it the grip climbs even though the hand is moving, because a
    /// hand nudging alongside the assist is not taking the view from it.
    /// Above it the grip falls, and the harder the push the sooner it reaches
    /// nothing.
    ///
    /// Not a constant of its own. It falls out of the two durations, so it
    /// cannot come to disagree with them — which is the only way a third
    /// number describing the same crossing could end up.
    pub fn holding_push(self) -> f32 {
        let rise = self.rise.as_secs_f32();
        let fall = self.fall.as_secs_f32();
        if rise + fall <= 0.0 {
            return 0.0;
        }
        fall / (rise + fall)
    }
}

/// How firmly the assist is holding the view, from nothing to full.
///
/// Asked again every pass, and never told that a hold has begun. Both matter,
/// and the second is the more expensive to get wrong: the product this one
/// replaces decided at the moment of the press, so pressing while the hand
/// was moving — which is what every player does — skipped the whole hold. It
/// was measured happening six times in every thirty seconds of play, and it
/// is what "I pressed and nothing grabbed" was.
///
/// Here a hand that is moving when the button goes down delays the rise and
/// nothing else. The assist arrives late; it does not fail to arrive. Nothing
/// in this knows what a hold is, which is what makes the old failure
/// impossible rather than merely avoided.
#[derive(Clone, Copy, Debug, Default)]
pub struct Grip {
    /// How far along the ramp, before the curve is applied.
    ///
    /// Kept separately from the strength it produces so that the curve is
    /// applied to the position and not compounded into it each pass, which
    /// would make the rise depend on how often it was asked.
    along: f32,
}

/// The most of a ramp one pass may cover.
///
/// A pass longer than the ramp itself would otherwise carry the grip from
/// nothing to full in one step, which is the hard edge the ramp exists to
/// remove — and long passes are reachable: the overlay is rebuilt behind a
/// half-second gate whenever the game window comes back, and the scheduler
/// takes the thread away without asking. A stall should cost the ramp some
/// time, not turn it back into a switch.
const MOST_OF_A_RAMP: f32 = 0.25;

impl Grip {
    /// Move the grip one pass, and say how firm it is now.
    ///
    /// `engaged` is whether the button is down at all; `push` is how hard the
    /// hand is working against it. With the button up the grip falls at full
    /// rate, so the next press has to earn its strength back from nothing.
    ///
    /// `elapsed` is passed in rather than measured, so the ramp can be tested
    /// without waiting for a clock, and so it takes the same time at any rate
    /// the loop happens to run at.
    pub fn update(&mut self, engaged: bool, push: f32, elapsed: Duration, ramp: Ramp) -> f32 {
        // A push nobody could measure is treated as a push at full rate. Every
        // uncertainty here falls the same way: towards the player having the
        // view, never towards the assist keeping it.
        let push = if engaged && push.is_finite() {
            push.clamp(0.0, 1.0)
        } else {
            1.0
        };
        // Both at once, in proportion, rather than one or the other. Choosing
        // between them on whether the hand moved at all made a single count
        // enough to forbid the rise entirely, and a hand on a mouse produces
        // a count in most passes — so through a session of ordinary play the
        // grip could only ever fall. It sat between nothing and one per cent.
        self.along +=
            Self::step(elapsed, ramp.rise) * (1.0 - push) - Self::step(elapsed, ramp.fall) * push;
        self.along = self.along.clamp(0.0, 1.0);
        smooth(self.along)
    }

    /// The share of a ramp one pass of this length covers.
    fn step(elapsed: Duration, over: Duration) -> f32 {
        let over = over.as_secs_f32();
        if over <= 0.0 {
            return MOST_OF_A_RAMP;
        }
        (elapsed.as_secs_f32() / over).min(MOST_OF_A_RAMP)
    }

    /// How firm it is, without moving it. For anything that only reports.
    pub fn firmness(self) -> f32 {
        smooth(self.along)
    }
}

/// The curve that leaves both ends flat.
///
/// What makes the strength ease in and ease out rather than ramp straight up
/// and straight down. Straight would still be an improvement on switching,
/// but it has a corner at each end — the moment the assist starts and the
/// moment it reaches full — and a corner is the thing being removed.
fn smooth(along: f32) -> f32 {
    let along = along.clamp(0.0, 1.0);
    along * along * (3.0 - 2.0 * along)
}

/// Everything one pass knows, as plain values.
///
/// Gathered into one thing so the deciding can be done away from the reading:
/// what the assist does with a situation is then a question with an answer,
/// asked and checked without a game running. Which the ladder asked for from
/// the start — the logic that decides something must be testable without
/// opening the game — and which was not true while it lived in the binary.
pub struct Situation<'a> {
    /// Whether the button is down this instant.
    pub held: bool,
    /// Whether the game is the window the player is actually in.
    pub in_front: bool,
    /// The player's own mouse. `None` when it cannot be read at all, which is
    /// never the same as a hand that is not moving.
    pub hand: Option<[i64; 2]>,
    pub me: Option<LocalPlayer>,
    pub players: &'a [Player],
    pub angles: ViewAngles,
    /// Whether the pull has already finished during this press.
    ///
    /// Held by the caller because it belongs to the press and not to the
    /// pass. Nothing in here may set it — that would be this deciding when a
    /// press began, which is the one thing the yielding rule is built not to
    /// know.
    pub delivered: bool,
}

/// What the assist decided, and why.
pub struct Choice {
    /// The enemy the view was steered towards, if one was chosen. Set even
    /// when nothing was sent, so a refusal can say who it was about.
    pub target: Option<usize>,
    pub offset: Offset,
    /// Whether the view is inside the target this pass — the moment the pull
    /// is finished. The caller latches it for the rest of the press.
    pub arrived: bool,
    /// The movement to send, or the reason there is none.
    pub counts: Result<[i32; 2], Refusal>,
}

impl Steering {
    /// Where to move the view this pass, or why not to.
    ///
    /// `grip` is how firmly the view is being held and `push` how hard the
    /// hand is working against it — both from the [`Grip`], which is stateful
    /// and so is kept by the caller across passes.
    pub fn choose(self, now: &Situation<'_>, grip: f32, push: f32) -> Choice {
        let mut choice = Choice {
            target: None,
            offset: Offset::default(),
            arrived: false,
            counts: Err(Refusal::NotHeld),
        };
        choice.counts = self.decide(now, grip, push, &mut choice);
        choice
    }

    /// The angle a target of this distance is handed over at.
    ///
    /// Never under the deadzone, which is the width below which a movement is
    /// a tremble rather than aim. At the distances a CS2 map allows, the body
    /// is wider than that everywhere, so the floor is a guard and not a rule
    /// anyone will meet.
    pub fn handover(self, distance: f32) -> f32 {
        if !distance.is_finite() || distance <= 0.0 {
            return self.deadzone;
        }
        (self.body_half_width / distance)
            .atan()
            .to_degrees()
            .max(self.deadzone)
    }

    fn decide(
        self,
        now: &Situation<'_>,
        grip: f32,
        push: f32,
        choice: &mut Choice,
    ) -> Result<[i32; 2], Refusal> {
        if !now.held {
            return Err(Refusal::NotHeld);
        }
        if !now.in_front {
            // The same counts would drag the pointer across whatever window
            // is in front instead of turning the view.
            return Err(Refusal::NotInFront);
        }
        // Before anything else worth doing. Steering while unable to tell
        // whether the player is pushing back is the one failure the whole
        // yielding step exists to prevent, so it is a refusal and not a
        // fallback to some safe-looking default.
        if now.hand.is_none() {
            return Err(Refusal::HandUnreadable);
        }
        // Our own side is what every enemy test is made against, so a bad
        // reading of it would make targets of teammates.
        let me = now
            .me
            .filter(|me| me.plausible())
            .ok_or(Refusal::NoLocalPlayer)?;
        // Dead is a real state and a plausible one, and the camera is
        // somewhere else entirely while it lasts: the origin still read is a
        // corpse's, the angles are whoever is being watched. Steering between
        // two unrelated frames of reference drags the spectator camera about.
        if !me.alive() {
            return Err(Refusal::NotAlive);
        }
        if !now.angles.plausible() {
            return Err(Refusal::ViewImplausible);
        }
        let eye = now
            .players
            .iter()
            .find(|player| player.pawn == me.pawn)
            .and_then(|player| player.origin)
            .map(|origin| self.eyes(origin))
            .ok_or(Refusal::NoPositionForUs)?;

        // The one nearest to where the player is already pointing. Measured
        // as an angle rather than as a distance on screen, because a target
        // at the edge of the view is further away than the same gap in pixels
        // near the middle, and it is the angle the movement has to cover.
        let (pawn, offset, distance) = now
            .players
            .iter()
            .filter(|player| {
                player.pawn != me.pawn
                    && player.plausible()
                    && me.team.opposes(player.team)
                    && player.alive()
            })
            .filter_map(|player| {
                let chest = self.aim_point(player.origin?);
                let desired = look_at(eye, chest)?;
                let at = offset(now.angles, desired);
                self.within_cone(at)
                    .then_some((player.pawn, at, apart(eye, chest)))
            })
            .min_by(|(_, a, _), (_, b, _)| a.size().total_cmp(&b.size()))
            .ok_or(Refusal::NoEnemyInTheCone)?;

        // Recorded before the last refusals rather than after them, so a line
        // about the player taking the view back can still say which enemy it
        // was taken from.
        choice.target = Some(pawn);
        choice.offset = offset;
        choice.arrived = offset.size() <= self.handover(distance);

        // The pull is one pull. Once the view is anywhere inside the target
        // the angle is closed and the rest is the player's, and it stays
        // theirs until they let the button go — which is the caller's to
        // remember, since nothing here may know what a press is.
        if now.delivered || choice.arrived {
            return Err(Refusal::Delivered);
        }

        self.counts(offset, grip).ok_or({
            // Nothing to send means two different things: the player has
            // taken the view, or the grip is simply too low yet to round to a
            // movement — which happens on every rise and has nothing to do
            // with the hand. Calling them one thing quietly spoiled the count
            // the tuning rests on.
            if push > 0.0 {
                Refusal::HandWins
            } else {
                Refusal::Easing
            }
        })
    }

    /// Our own camera, from the feet our origin records.
    fn eyes(self, origin: [f32; 3]) -> [f32; 3] {
        [origin[0], origin[1], origin[2] + self.eye_height]
    }

    /// The place on a target the pull aims at.
    fn aim_point(self, origin: [f32; 3]) -> [f32; 3] {
        [origin[0], origin[1], origin[2] + self.aim_height]
    }
}

/// How far apart two places are.
fn apart(from: [f32; 3], to: [f32; 3]) -> f32 {
    let [x, y, z] = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    x.hypot(y).hypot(z)
}

/// What stopped the view from being steered this pass, if anything did.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Refusal {
    #[default]
    NotHeld,
    NotInFront,
    NoLocalPlayer,
    NotAlive,
    ViewImplausible,
    NoPositionForUs,
    HandUnreadable,
    NoEnemyInTheCone,
    HandWins,
    Easing,
    Delivered,
    WindowsRefused,
    Steering,
}

impl Refusal {
    /// Every refusal there is, which is what makes a tally of them complete.
    ///
    /// Listed rather than derived, and held to the real list by a test: a
    /// reason missing from here would be a reason nothing ever reports, which
    /// is the exact shape of the failure the tally exists to catch.
    pub const ALL: [Self; Self::COUNT] = [
        Self::NotHeld,
        Self::NotInFront,
        Self::NoLocalPlayer,
        Self::NotAlive,
        Self::ViewImplausible,
        Self::NoPositionForUs,
        Self::HandUnreadable,
        Self::NoEnemyInTheCone,
        Self::HandWins,
        Self::Easing,
        Self::Delivered,
        Self::WindowsRefused,
        Self::Steering,
    ];

    /// How many there are, for sizing a tally that cannot be indexed past.
    ///
    /// Written out rather than taken from the roll call, so that the two have
    /// to be made to agree instead of one silently following the other.
    pub const COUNT: usize = 13;

    /// Which slot of the tally this one is counted in.
    ///
    /// A match rather than the variant's own number, because a match must
    /// cover every variant: a reason added to the enum cannot compile until
    /// it has been given a slot, and the roll call test then refuses to pass
    /// until that slot is inside the tally and belongs to nothing else. The
    /// variant's own number would have needed neither, and a reason with no
    /// slot is a reason counted where nobody looks.
    pub const fn slot(self) -> usize {
        match self {
            Self::NotHeld => 0,
            Self::NotInFront => 1,
            Self::NoLocalPlayer => 2,
            Self::NotAlive => 3,
            Self::ViewImplausible => 4,
            Self::NoPositionForUs => 5,
            Self::HandUnreadable => 6,
            Self::NoEnemyInTheCone => 7,
            Self::HandWins => 8,
            Self::Easing => 9,
            Self::Delivered => 10,
            Self::WindowsRefused => 11,
            Self::Steering => 12,
        }
    }

    /// A short name, for a line that has to carry every one of them at once.
    pub const fn key(self) -> &'static str {
        match self {
            Self::NotHeld => "idle",
            Self::NotInFront => "not-in-front",
            Self::NoLocalPlayer => "no-local-player",
            Self::ViewImplausible => "view-implausible",
            Self::NotAlive => "not-alive",
            Self::HandUnreadable => "hand-unreadable",
            Self::Easing => "easing",
            Self::NoPositionForUs => "no-position",
            Self::NoEnemyInTheCone => "no-enemy",
            Self::HandWins => "hand",
            Self::Delivered => "delivered",
            Self::WindowsRefused => "windows-refused",
            Self::Steering => "steering",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::NotHeld => "idle",
            Self::NotInFront => "held, but the game is not in front",
            Self::NoLocalPlayer => "no plausible reading of us",
            Self::ViewImplausible => "view angles implausible — stale offsets?",
            Self::NotAlive => "we are not alive",
            Self::HandUnreadable => "the mouse cannot be read — refusing to steer blind",
            Self::Easing => "easing in",
            Self::NoPositionForUs => "our own position is not readable",
            Self::NoEnemyInTheCone => "no living enemy in the cone",
            Self::HandWins => "your hand",
            Self::Delivered => "delivered — the rest is yours",
            Self::WindowsRefused => "Windows refused the movement — not elevated?",
            Self::Steering => "steering",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Team;

    fn angles(pitch: f32, yaw: f32) -> ViewAngles {
        ViewAngles { pitch, yaw }
    }

    fn steering() -> Steering {
        Steering {
            counts_per_degree: 50.0,
            gain: 0.5,
            cap: 200.0,
            deadzone: 0.2,
            body_half_width: 12.0,
            eye_height: 64.0,
            aim_height: 55.0,
            cone: 30.0,
        }
    }

    #[test]
    fn looking_along_each_axis_gives_the_angle_the_engine_would_store() {
        let eye = [0.0, 0.0, 0.0];
        // Yaw counts anticlockwise from the x axis.
        let east = look_at(eye, [100.0, 0.0, 0.0]).expect("east");
        assert!(east.yaw.abs() < 1e-3, "{east:?}");
        let north = look_at(eye, [0.0, 100.0, 0.0]).expect("north");
        assert!((north.yaw - 90.0).abs() < 1e-3, "{north:?}");
        assert!(east.pitch.abs() < 1e-3 && north.pitch.abs() < 1e-3);
    }

    #[test]
    fn looking_up_is_a_negative_pitch_because_the_engine_counts_it_downwards() {
        // Forty-five degrees above: as far up as along.
        let above = look_at([0.0, 0.0, 0.0], [100.0, 0.0, 100.0]).expect("above");
        assert!((above.pitch + 45.0).abs() < 1e-3, "{above:?}");

        let below = look_at([0.0, 0.0, 0.0], [100.0, 0.0, -100.0]).expect("below");
        assert!((below.pitch - 45.0).abs() < 1e-3, "{below:?}");
    }

    #[test]
    fn a_target_in_the_same_place_as_the_eye_has_no_direction() {
        assert_eq!(look_at([5.0, 5.0, 5.0], [5.0, 5.0, 5.0]), None);
        assert_eq!(look_at([0.0; 3], [f32::NAN, 0.0, 0.0]), None);
    }

    #[test]
    fn every_angle_look_at_produces_is_one_the_engine_would_accept() {
        // Otherwise the offset against a real reading is measured between a
        // real angle and an impossible one.
        for (x, y, z) in [
            (1.0, 0.0, 0.0),
            (-1.0, -1.0, 5.0),
            (0.0, -1.0, -900.0),
            (-0.001, 0.0, 0.0),
        ] {
            let angles = look_at([0.0; 3], [x, y, z]).expect("a direction");
            assert!(angles.plausible(), "{angles:?} from {x},{y},{z}");
        }
    }

    #[test]
    fn an_offset_across_the_seam_is_the_short_way_round() {
        let offset = offset(angles(0.0, 179.0), angles(0.0, -179.0));
        assert!((offset.yaw - 2.0).abs() < 1e-3, "{offset:?}");
    }

    #[test]
    fn a_target_to_the_left_is_reached_by_moving_the_mouse_left() {
        // Yaw counts anticlockwise, so a target at a higher yaw is to the
        // left, and reaching it means a negative mouse movement.
        let left = steering()
            .counts(
                Offset {
                    yaw: 10.0,
                    pitch: 0.0,
                },
                1.0,
            )
            .expect("a movement");
        assert!(left[0] < 0, "{left:?}");

        let right = steering()
            .counts(
                Offset {
                    yaw: -10.0,
                    pitch: 0.0,
                },
                1.0,
            )
            .expect("a movement");
        assert!(right[0] > 0, "{right:?}");
    }

    #[test]
    fn a_target_below_is_reached_by_moving_the_mouse_down() {
        // Pitch counts downwards, so a target at a higher pitch is below, and
        // reaching it means a positive mouse movement.
        let below = steering()
            .counts(
                Offset {
                    yaw: 0.0,
                    pitch: 10.0,
                },
                1.0,
            )
            .expect("a movement");
        assert!(below[1] > 0, "{below:?}");
    }

    /// The movement for one axis, at full strength.
    fn one_pass(yaw: f32) -> i32 {
        steering()
            .counts(
                Offset {
                    yaw: -yaw,
                    pitch: 0.0,
                },
                1.0,
            )
            .map_or(0, |movement| movement[0])
    }

    #[test]
    fn a_movement_well_inside_the_ceiling_is_left_as_it_is() {
        // Four tenths of a degree at fifty counts a degree, half of it: ten
        // counts against a ceiling of two hundred. The curve has to be the
        // identity down here, or softening the far end would quietly change
        // how the view settles on a target at the near end.
        assert_eq!(one_pass(0.4), 10);
        assert_eq!(one_pass(0.8), 20);
    }

    #[test]
    fn no_single_pass_reaches_the_ceiling_however_far_away_the_target_is() {
        // What a position read mid-write looks like: an enormous distance.
        assert!(one_pass(170.0) <= 200, "{}", one_pass(170.0));
        assert!(one_pass(100_000.0) <= 200, "{}", one_pass(100_000.0));
        // And it gets close, rather than giving up somewhere short.
        assert!(one_pass(170.0) > 190, "{}", one_pass(170.0));
    }

    #[test]
    fn there_is_no_distance_at_which_the_movement_stops_growing_abruptly() {
        // A hard ceiling has a corner in it: below the corner the movement
        // grows with the distance, above it every distance gives the same
        // movement, and the change between the two is what is felt. Here each
        // step in distance still buys something, and always less than the one
        // before it — a bend, not a corner.
        let steps: Vec<i32> = (1..=12).map(|degrees| one_pass(degrees as f32)).collect();
        let gains: Vec<i32> = steps.windows(2).map(|pair| pair[1] - pair[0]).collect();
        assert!(gains.iter().all(|gain| *gain > 0), "{steps:?}");
        assert!(
            gains.windows(2).all(|pair| pair[1] <= pair[0]),
            "the curve must only ever bend one way: {gains:?}"
        );
    }

    #[test]
    fn strength_scales_what_is_sent_without_moving_the_ceiling() {
        let full = one_pass(4.0);
        let half = steering()
            .counts(
                Offset {
                    yaw: -4.0,
                    pitch: 0.0,
                },
                0.5,
            )
            .expect("a movement");
        assert_eq!(half[0], (full as f32 * 0.5).round() as i32);

        // At no strength there is nothing to send, however far off the view.
        assert_eq!(
            steering().counts(
                Offset {
                    yaw: -170.0,
                    pitch: 0.0
                },
                0.0
            ),
            None
        );
    }

    #[test]
    fn a_view_already_on_target_is_left_alone_rather_than_trembling() {
        assert_eq!(
            steering().counts(
                Offset {
                    yaw: 0.1,
                    pitch: 0.1
                },
                1.0
            ),
            None,
            "inside the deadzone"
        );
    }

    #[test]
    fn a_reading_that_was_not_a_number_moves_nothing() {
        assert_eq!(
            steering().counts(
                Offset {
                    yaw: f32::NAN,
                    pitch: 0.0
                },
                1.0
            ),
            None
        );
        assert_eq!(
            steering().counts(
                Offset {
                    yaw: -10.0,
                    pitch: 0.0
                },
                f32::NAN
            ),
            None
        );
    }

    const PASS: Duration = Duration::from_millis(8);

    fn ramp() -> Ramp {
        Ramp {
            // Six counts in an eight millisecond pass.
            full_push: 750.0,
            rise: Duration::from_millis(120),
            fall: Duration::from_millis(120),
        }
    }

    /// Counts in one pass that push at full rate.
    fn a_full_push() -> [i64; 2] {
        [(ramp().full_push * PASS.as_secs_f32()).ceil() as i64, 0]
    }

    /// Run the ramp one way for this long, and give the strength it reaches.
    fn ramped(grip: &mut Grip, engaged: bool, push: f32, over: Duration) -> f32 {
        let mut passed = Duration::ZERO;
        let mut firmness = grip.firmness();
        while passed < over {
            firmness = grip.update(engaged, push, PASS, ramp());
            passed += PASS;
        }
        firmness
    }

    #[test]
    fn a_hand_at_rest_pushes_not_at_all_and_a_fast_one_pushes_as_hard_as_it_can() {
        assert_eq!(ramp().push([0, 0], PASS), 0.0);
        assert_eq!(ramp().push(a_full_push(), PASS), 1.0, "sideways");
        let [counts, _] = a_full_push();
        assert_eq!(ramp().push([0, -counts], PASS), 1.0, "and vertically");
        // Nothing in between is ignored: a slower hand pushes more gently
        // rather than not at all, which is what keeps a speed under which the
        // view could never be taken back from existing at all.
        let gentle = ramp().push([counts / 4, 0], PASS);
        assert!(gentle > 0.0 && gentle < 0.5, "{gentle}");
    }

    #[test]
    fn the_same_hand_pushes_the_same_however_often_the_loop_asks() {
        // Counts in a pass measure the same hand differently at a different
        // rate. A speed does not, and the feel of the assist must not depend
        // on the machine it is running on.
        let fast = ramp().push([6, 0], Duration::from_millis(8));
        let slow = ramp().push([12, 0], Duration::from_millis(16));
        assert!((fast - slow).abs() < 1e-6, "{fast} against {slow}");
    }

    #[test]
    fn a_hand_moving_alongside_the_assist_does_not_stop_it_from_taking_hold() {
        // The failure this replaces: the rise was refused whenever the hand
        // had moved at all, and a hand resting on a mouse produces a count in
        // most passes, so through ordinary play the grip could only ever
        // fall. A session held it between nothing and one per cent.
        let mut grip = Grip::default();
        let nudging = ramp().holding_push() / 3.0;
        let firmness = ramped(&mut grip, true, nudging, ramp().rise * 3);
        assert!(
            firmness > 0.9,
            "a nudge should not forbid the rise: {firmness}"
        );
    }

    #[test]
    fn a_push_past_the_holding_point_ends_with_the_player_having_the_view() {
        // And the harder the push the sooner, rather than every push above
        // some line arriving at the same moment.
        let mut hard = Grip::default();
        let mut firm = Grip::default();
        ramped(&mut hard, true, 0.0, ramp().rise);
        ramped(&mut firm, true, 0.0, ramp().rise);

        let over = ramp().fall * 2;
        assert_eq!(ramped(&mut hard, true, 1.0, over), 0.0);
        assert_eq!(ramped(&mut firm, true, 0.75, over * 3), 0.0);

        // Exactly at the holding point it goes nowhere, which is what makes
        // the point worth naming.
        let mut held = Grip::default();
        let half = ramped(&mut held, true, 0.0, ramp().rise / 2);
        let after = ramped(&mut held, true, ramp().holding_push(), ramp().rise * 2);
        assert!((after - half).abs() < 0.05, "{half} then {after}");
    }

    #[test]
    fn one_long_pass_may_not_carry_the_grip_all_the_way_in_a_single_step() {
        // Reachable: the overlay is rebuilt behind a half second gate when
        // the game window comes back, and the scheduler takes the thread away
        // without asking. A stall must cost the ramp time, not turn it back
        // into the switch it replaced.
        let mut grip = Grip::default();
        let firmness = grip.update(true, 0.0, ramp().rise * 4, ramp());
        assert!(firmness < 0.5, "seized the view in one pass: {firmness}");

        let mut full = Grip::default();
        ramped(&mut full, true, 0.0, ramp().rise);
        let after = full.update(false, 0.0, ramp().fall * 4, ramp());
        assert!(after > 0.2, "dropped the view in one pass: {after}");
    }

    #[test]
    fn a_hold_begins_with_no_grip_at_all_and_takes_the_rise_to_reach_full() {
        // Seizing the view on the pass the button goes down is the thing that
        // made this feel hard-edged. Every press starts from nothing.
        let mut grip = Grip::default();
        assert_eq!(grip.firmness(), 0.0);

        let part_way = ramped(&mut grip, true, 0.0, ramp().rise / 2);
        assert!(part_way > 0.0 && part_way < 1.0, "{part_way}");

        let full = ramped(&mut grip, true, 0.0, ramp().rise);
        assert!(full > 0.99, "{full}");
    }

    #[test]
    fn the_first_moments_of_a_rise_are_gentler_than_an_even_one_would_be() {
        // What makes it a curve rather than a straight line: no corner where
        // the assist goes from doing nothing to climbing at full rate.
        let mut grip = Grip::default();
        let tenth = ramp().rise / 10;
        let early = ramped(&mut grip, true, 0.0, tenth);
        assert!(
            early < 0.1,
            "an even rise would be at a tenth by now: {early}"
        );

        // And the same at the top, arriving rather than striking.
        let mut grip = Grip::default();
        let nine_tenths = ramped(&mut grip, true, 0.0, tenth * 9);
        assert!(nine_tenths > 0.9, "{nine_tenths}");
    }

    #[test]
    fn pushing_against_it_eases_the_grip_off_rather_than_taking_it_away() {
        let mut grip = Grip::default();
        assert!(ramped(&mut grip, true, 0.0, ramp().rise) > 0.99);

        // Partway through the fall the assist is still there, and weaker.
        let easing = ramped(&mut grip, true, 1.0, ramp().fall / 2);
        assert!(easing > 0.0 && easing < 0.9, "{easing}");

        // Keep pushing and the view belongs entirely to the player.
        assert_eq!(ramped(&mut grip, true, 1.0, ramp().fall), 0.0);
    }

    #[test]
    fn the_grip_never_leaves_the_range_it_is_measured_in() {
        let mut grip = Grip::default();
        for (engaged, push) in [
            (true, 0.0),
            (true, 1.0),
            (false, 0.0),
            (false, 1.0),
            (true, 0.5),
            (true, -3.0),
            (true, f32::NAN),
        ] {
            for _ in 0..500 {
                let firmness = grip.update(engaged, push, PASS, ramp());
                assert!((0.0..=1.0).contains(&firmness), "{firmness}");
            }
        }
    }

    #[test]
    fn the_rise_takes_the_same_time_however_often_it_is_asked() {
        // Otherwise the feel of the assist changes with the frame rate, and
        // a slower machine gets a different product.
        let mut fast = Grip::default();
        let mut slow = Grip::default();
        let mut passed = Duration::ZERO;
        while passed < ramp().rise {
            fast.update(true, 0.0, Duration::from_millis(2), ramp());
            fast.update(true, 0.0, Duration::from_millis(2), ramp());
            slow.update(true, 0.0, Duration::from_millis(4), ramp());
            passed += Duration::from_millis(4);
        }
        assert!((fast.firmness() - slow.firmness()).abs() < 1e-3);
    }

    #[test]
    fn a_hand_moving_when_the_button_goes_down_costs_the_rise_and_no_more() {
        // The failure this replaces: deciding at the moment of the press, so
        // pressing mid-movement skipped the entire hold. Nothing here is told
        // that a hold began, which is what makes that impossible rather than
        // merely avoided.
        let mut grip = Grip::default();
        assert_eq!(ramp().push([300, 50], PASS), 1.0, "mid-flick, button down");
        assert_eq!(ramped(&mut grip, true, 1.0, ramp().fall), 0.0);

        // The hand settles, and the assist is all the way there one rise
        // later — late, not absent.
        assert!(ramped(&mut grip, true, 0.0, ramp().rise) > 0.99);
    }

    fn enemy(pawn: usize, at: [f32; 3]) -> Player {
        Player {
            controller: pawn,
            pawn,
            team: Team::CounterTerrorist,
            health: 100,
            origin: Some(at),
        }
    }

    fn us() -> LocalPlayer {
        LocalPlayer {
            pawn: 0x1000,
            health: 100,
            team: Team::Terrorist,
        }
    }

    /// Us at the origin looking east, with whoever else is passed in.
    fn facing_east(players: &[Player]) -> Vec<Player> {
        let mut all = vec![Player {
            controller: us().pawn,
            pawn: us().pawn,
            team: us().team,
            health: us().health,
            origin: Some([0.0, 0.0, 0.0]),
        }];
        all.extend_from_slice(players);
        all
    }

    fn situation(players: &[Player]) -> Situation<'_> {
        Situation {
            held: true,
            in_front: true,
            hand: Some([0, 0]),
            me: Some(us()),
            players,
            angles: ViewAngles {
                pitch: 0.0,
                yaw: 0.0,
            },
            delivered: false,
        }
    }

    /// One way of breaking an otherwise workable pass, and what it should be
    /// refused as.
    struct Breakage {
        expected: Refusal,
        break_it: Break,
    }

    /// What breaking one thing about a pass looks like.
    type Break = fn(&mut Situation<'_>);

    /// Everything the assist refuses over, in the order it refuses.
    ///
    /// Each entry breaks one thing about an otherwise workable pass. Kept as
    /// a list so a new refusal has somewhere obvious to go, and so the order
    /// is a fact with a test rather than whatever the code happens to do.
    fn each_way_it_can_refuse() -> Vec<Breakage> {
        let ways: [(Refusal, Break); 7] = [
            (Refusal::NotHeld, |now| now.held = false),
            (Refusal::NotInFront, |now| now.in_front = false),
            (Refusal::HandUnreadable, |now| now.hand = None),
            (Refusal::NoLocalPlayer, |now| now.me = None),
            (Refusal::NotAlive, |now| {
                now.me = Some(LocalPlayer { health: 0, ..us() });
            }),
            (Refusal::ViewImplausible, |now| now.angles.pitch = 400.0),
            (Refusal::NoPositionForUs, |now| now.players = &[]),
        ];
        ways.into_iter()
            .map(|(expected, break_it)| Breakage { expected, break_it })
            .collect()
    }

    #[test]
    fn a_workable_pass_steers_so_that_breaking_one_thing_means_something() {
        let players = facing_east(&[enemy(0x2000, [1000.0, 200.0, 0.0])]);
        let choice = STEERING.choose(&situation(&players), 1.0, 0.0);
        assert_eq!(choice.target, Some(0x2000));
        assert!(choice.counts.is_ok(), "{:?}", choice.counts);
    }

    #[test]
    fn every_refusal_that_can_be_provoked_names_itself() {
        let players = facing_east(&[enemy(0x2000, [1000.0, 200.0, 0.0])]);
        for way in each_way_it_can_refuse() {
            let mut now = situation(&players);
            (way.break_it)(&mut now);
            assert_eq!(
                STEERING.choose(&now, 1.0, 0.0).counts,
                Err(way.expected),
                "breaking the thing {:?} is about gave something else",
                way.expected
            );
        }
    }

    #[test]
    fn a_refusal_is_reached_before_anything_it_would_have_to_trust() {
        // Order is the point, not merely coverage. Reading our own position
        // before checking we are alive would take a corpse's; choosing a
        // target before knowing the hand can be read would steer blind. Break
        // two things at once and the earlier reason must be the one given.
        let players = facing_east(&[enemy(0x2000, [1000.0, 200.0, 0.0])]);
        let ways = each_way_it_can_refuse();
        for (earlier, index) in ways.iter().zip(1..) {
            for later in &ways[index..] {
                let mut now = situation(&players);
                (later.break_it)(&mut now);
                (earlier.break_it)(&mut now);
                assert_eq!(
                    STEERING.choose(&now, 1.0, 0.0).counts,
                    Err(earlier.expected),
                    "{:?} should come before {:?}",
                    earlier.expected,
                    later.expected
                );
            }
        }
    }

    #[test]
    fn nobody_worth_shooting_is_told_apart_from_nobody_at_all() {
        let mine = Player {
            team: Team::Terrorist,
            ..enemy(0x2000, [1000.0, 200.0, 0.0])
        };
        let dead = Player {
            health: 0,
            ..enemy(0x3000, [1000.0, 0.0, 0.0])
        };
        let behind = enemy(0x4000, [-1000.0, 0.0, 0.0]);
        let unreadable = Player {
            health: 900,
            ..enemy(0x5000, [1000.0, 0.0, 0.0])
        };
        for player in [mine, dead, behind, unreadable] {
            let players = facing_east(&[player]);
            assert_eq!(
                STEERING.choose(&situation(&players), 1.0, 0.0).counts,
                Err(Refusal::NoEnemyInTheCone),
                "{player:?}"
            );
        }
    }

    #[test]
    fn the_nearest_by_angle_is_chosen_and_not_the_nearest_by_distance() {
        // Close but wide, against far but nearly straight ahead. The movement
        // to reach the second is smaller, which is what is being minimised.
        let close_and_wide = enemy(0x2000, [300.0, 120.0, 0.0]);
        let far_and_ahead = enemy(0x3000, [4000.0, 60.0, 0.0]);
        let players = facing_east(&[close_and_wide, far_and_ahead]);
        let choice = STEERING.choose(&situation(&players), 1.0, 0.0);
        assert_eq!(choice.target, Some(0x3000));
    }

    #[test]
    fn the_pull_ends_the_moment_the_view_is_anywhere_inside_the_target() {
        // Level with us, so adding the same eye height to both leaves the
        // view exactly on them.
        let dead_ahead = facing_east(&[enemy(0x2000, [1000.0, 0.0, 0.0])]);
        let choice = STEERING.choose(&situation(&dead_ahead), 1.0, 0.0);
        assert!(choice.arrived);
        assert_eq!(choice.counts, Err(Refusal::Delivered));

        // A third of a degree off at a thousand units is still inside a
        // body, which is what the handover is measured against — not the
        // fraction of a degree that stops a tremble.
        let nearly = facing_east(&[enemy(0x2000, [1000.0, 5.0, 0.0])]);
        let choice = STEERING.choose(&situation(&nearly), 1.0, 0.0);
        assert!(choice.arrived, "off by {:.2} deg", choice.offset.size());
    }

    #[test]
    fn what_counts_as_inside_the_target_shrinks_with_the_distance_to_it() {
        // A fixed angle would hand over short of a target up close and well
        // past one at range. These are the widths a player subtends.
        let close = STEERING.handover(500.0);
        let far = STEERING.handover(2000.0);
        assert!((close - 1.38).abs() < 0.05, "{close}");
        assert!((far - 0.34).abs() < 0.05, "{far}");
        assert!(close > far);

        // And never under the width that stops a tremble, however far away.
        assert!(STEERING.handover(1e9) >= STEERING.deadzone);
        assert!(STEERING.handover(f32::NAN) >= STEERING.deadzone);
        assert!(STEERING.handover(0.0) >= STEERING.deadzone);
    }

    #[test]
    fn once_it_has_been_handed_over_it_stays_handed_over() {
        // The target walks away afterwards. Nothing follows: this press has
        // closed the angle it was pressed to close.
        let walked_off = facing_east(&[enemy(0x2000, [1000.0, 300.0, 0.0])]);
        let mut now = situation(&walked_off);
        assert!(now.hand.is_some());
        assert!(
            STEERING.choose(&now, 1.0, 0.0).counts.is_ok(),
            "far enough off to be worth a pull"
        );

        now.delivered = true;
        let choice = STEERING.choose(&now, 1.0, 0.0);
        assert_eq!(choice.counts, Err(Refusal::Delivered));
        // Still says who, so a line about it can be read afterwards.
        assert_eq!(choice.target, Some(0x2000));
        assert!(!choice.arrived, "and does not claim to have just arrived");
    }

    #[test]
    fn nothing_to_send_says_which_of_the_two_it_was() {
        let off_to_one_side = facing_east(&[enemy(0x2000, [1000.0, 200.0, 0.0])]);
        assert_eq!(
            STEERING
                .choose(&situation(&off_to_one_side), 0.0, 1.0)
                .counts,
            Err(Refusal::HandWins),
            "the player has taken it"
        );
        assert_eq!(
            STEERING
                .choose(&situation(&off_to_one_side), 0.0, 0.0)
                .counts,
            Err(Refusal::Easing),
            "the grip is only on its way up"
        );
    }

    #[test]
    fn the_tuned_values_still_mean_what_they_are_written_down_as_meaning() {
        // None of these is worth pinning on its own — they are meant to be
        // turned. What is worth pinning is the arithmetic between them, since
        // each figure is described in prose by a number none of them holds.
        let degrees_a_second = RAMP.full_push * STEERING.counts_per_degree.recip();
        assert!(
            (10.0..20.0).contains(&degrees_a_second),
            "a full push is {degrees_a_second:.1} degrees a second, not the \
             fifteen it is written as"
        );

        let holding = RAMP.holding_push() * degrees_a_second;
        assert!(
            (9.0..13.0).contains(&holding),
            "the grip holds at {holding:.1} degrees a second, not the eleven \
             it is written as"
        );

        // A pass at the target rate must not be able to cross the whole ramp,
        // or the curve is a switch again on a machine like this one.
        let pass = Duration::from_millis(8).as_secs_f32();
        assert!(pass < RAMP.rise.as_secs_f32() / 4.0);
        assert!(pass < RAMP.fall.as_secs_f32() / 4.0);
    }

    #[test]
    fn wherever_the_pull_stops_is_somewhere_a_bullet_would_land() {
        // The defect this is here for: aiming at the head and handing over at
        // the width of a body stops half the time in the sky above the
        // shoulders. The circle the pull may stop anywhere inside has to lie
        // inside the body, so the point it is centred on and the width of it
        // have to describe the same shape.
        //
        // A player stands seventy-two units tall and the pull aims at the
        // chest, so the circle must not reach the top of the head or the
        // ground, nor further out than the shoulders.
        let reach = STEERING.body_half_width;
        assert!(
            STEERING.aim_height + reach < 72.0,
            "the circle reaches over the head"
        );
        assert!(STEERING.aim_height - reach > 0.0, "and into the ground");
        assert!(
            STEERING.aim_height < 64.0,
            "aiming at or above the eyes is aiming at the head"
        );
    }

    #[test]
    fn the_pull_aims_at_a_target_lower_than_it_measures_from() {
        // Two heights that were one figure while the pull aimed at heads. One
        // is where our own camera is, which the game decides; the other is
        // where we aim on someone else, which we do.
        assert_ne!(STEERING.eye_height, STEERING.aim_height);

        // Level ground, a thousand units away: the pull is downwards, because
        // it aims below its own eye.
        let ahead = facing_east(&[enemy(0x2000, [1000.0, 0.0, 0.0])]);
        let choice = STEERING.choose(&situation(&ahead), 1.0, 0.0);
        assert!(choice.offset.pitch > 0.0, "{:?}", choice.offset);
    }

    #[test]
    fn the_roll_call_of_refusals_holds_every_one_of_them_exactly_once() {
        // A tally is indexed by the refusal itself, so a reason left out of
        // the roll call is a reason whose count is kept and never printed —
        // which is the silence the tally exists to break, reappearing inside
        // the thing meant to break it.
        let mut seen = [false; Refusal::COUNT];
        for refusal in Refusal::ALL {
            let index = refusal.slot();
            assert!(index < Refusal::COUNT, "{refusal:?} indexes past the tally");
            assert!(!seen[index], "{refusal:?} listed twice");
            seen[index] = true;
        }
        assert!(seen.iter().all(|seen| *seen), "a refusal has no slot");
    }

    #[test]
    fn every_refusal_says_something_different() {
        for (index, refusal) in Refusal::ALL.iter().enumerate() {
            for other in &Refusal::ALL[index + 1..] {
                assert_ne!(refusal.key(), other.key(), "{refusal:?} and {other:?}");
                assert_ne!(refusal.label(), other.label(), "{refusal:?} and {other:?}");
            }
        }
    }

    #[test]
    fn someone_outside_the_cone_is_not_who_the_player_meant() {
        let steering = steering();
        assert!(steering.within_cone(Offset {
            yaw: 20.0,
            pitch: 10.0
        }));
        assert!(!steering.within_cone(Offset {
            yaw: 170.0,
            pitch: 0.0
        }));
    }
}
