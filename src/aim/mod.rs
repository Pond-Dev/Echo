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
//! Nothing in here switches on or off. Every edge is a curve, in both of the
//! places an edge would otherwise be felt. A movement asked for approaches
//! its ceiling instead of striking it, so there is no distance at which the
//! assist stops accelerating and starts coasting. And the strength it steers
//! with rises and falls over time rather than between two passes, so pushing
//! against it meets a grip that eases off rather than one that lets go.

use std::time::Duration;

use crate::game::ViewAngles;

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
        let push = if engaged { push.clamp(0.0, 1.0) } else { 1.0 };
        self.along += if push > 0.0 {
            -Self::step(elapsed, ramp.fall) * push
        } else {
            Self::step(elapsed, ramp.rise)
        };
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
    AlreadyOnTarget,
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
        Self::AlreadyOnTarget,
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
            Self::AlreadyOnTarget => 10,
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
            Self::AlreadyOnTarget => "on-target",
            Self::WindowsRefused => "windows-refused",
            Self::Steering => "steering",
        }
    }

    /// Whether the view is being held on a target. Moving onto one and
    /// sitting on one are the same thing from outside.
    pub const fn engaged(self) -> bool {
        matches!(self, Self::Steering | Self::AlreadyOnTarget)
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
            Self::AlreadyOnTarget => "on target",
            Self::WindowsRefused => "Windows refused the movement — not elevated?",
            Self::Steering => "steering",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn angles(pitch: f32, yaw: f32) -> ViewAngles {
        ViewAngles { pitch, yaw }
    }

    fn steering() -> Steering {
        Steering {
            counts_per_degree: 50.0,
            gain: 0.5,
            cap: 200.0,
            deadzone: 0.2,
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
    fn a_hand_pushing_gently_still_ends_with_the_view_rather_than_never_winning() {
        let mut grip = Grip::default();
        assert!(ramped(&mut grip, true, 0.0, ramp().rise) > 0.99);
        // A tenth of a full push, for ten falls. A threshold would have left
        // this hand pinned under the assist for as long as it cared to push.
        assert_eq!(ramped(&mut grip, true, 0.1, ramp().fall * 11), 0.0);
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
