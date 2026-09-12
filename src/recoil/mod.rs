//! Taking back what the gun did to the aim.
//!
//! Where a shot goes is the view angle plus the weapon's punch. The player
//! points the first and the gun adds the second, so keeping the shots where
//! they were pointed means moving the view by the opposite of the punch, for
//! as long as the punch lasts — including while it decays back, or the view
//! is left below where it started.
//!
//! Nothing in here knows about a target, and that is the point. It cancels a
//! force the game applies; it has no opinion about who is being shot at. So
//! it can run with the aim assist off, and when the assist is on, the two are
//! about different things and neither has to ask the other's permission.
//!
//! # What the player has already done counts first
//!
//! A player spraying pulls down. That pull is not resistance and must not be
//! treated as any: it is part of the same compensation, it simply arrives
//! from their hand instead of from here. So their movement pays what is owed
//! before this does, and what goes out is the remainder. Both pulling at once
//! is how a spray ends up in the floor.
//!
//! Movement the other way — aiming up mid-spray — does not *add* to what is
//! owed either. It is the player deciding where to point, and answering it by
//! pulling harder is the fight this is built not to have.
//!
//! # Order, which was expensive to learn
//!
//! The compensation is worked out before anything that aims, and whatever
//! aims must look at the view as it will be *after* the compensation lands.
//! Otherwise both cancel the same kick — the aim sees the punch as error and
//! corrects it, and this corrects it too. The product before this one folded
//! recoil in after the controller and got exactly that: twice the correction,
//! and a shake at the firing rate.
//!
//! # Nothing knows what a weapon is
//!
//! Two sprays in one log, read shot by shot, matched to three decimal places
//! — the punch is a function of how many rounds have gone out, and the game
//! keeps the answer. So there is no table of patterns here, nothing to update
//! when a weapon is tuned, and nothing to get wrong for weapons nobody
//! thought to test.

use std::time::Duration;

use crate::game::Punch;

/// How much of the gun's kick to take back, and how quickly.
#[derive(Clone, Copy, Debug)]
pub struct Control {
    /// Share of the upward kick to take back.
    ///
    /// A whole one is not the same as doing all the work: what the player's
    /// own pull covers is taken off first, so at a whole one a player who
    /// compensates perfectly gets nothing added and one who does not gets the
    /// difference. Anything less is a deliberate choice to leave some of the
    /// climb in.
    pub vertical: f32,
    /// Share of the sideways wander to take back.
    ///
    /// Kept separate because the two are not the same problem. The climb is
    /// steady and a hand can learn it; the wander reverses several times
    /// through a spray and is the part hands are worst at — and also the part
    /// that looks least like a person when it is cancelled exactly.
    pub sideways: f32,
    /// Share of what is still owed to pay each pass.
    ///
    /// Under one, so a shot's kick is paid across the gap before the next
    /// shot rather than in the single pass it arrives on. A whole one would
    /// put the entire step into one pass, which is a jolt at the firing rate
    /// — and the same jolt whether or not it is in the right direction.
    pub pace: f32,
    /// Mouse counts that turn the view one degree.
    pub counts_per_degree: f32,
    /// The most one pass may send on one axis.
    ///
    /// The same ceiling the aim assist works under, and for the same reason:
    /// a movement faster than the player has ever made is not theirs however
    /// correct it is.
    pub cap: f32,
}

impl Control {
    /// Whether this is switched on in any sense that costs anything.
    ///
    /// Both shares at nothing is not a quiet setting, it is off — and saying
    /// so here keeps a lane that compensates nothing from paying for itself
    /// every pass and moving the view by the rounding.
    pub fn wanted(self) -> bool {
        self.vertical > 0.0 || self.sideways > 0.0
    }
}

/// What to send, and what it will do.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Correction {
    /// Counts to send this pass.
    pub counts: [i32; 2],
    /// What those counts move the view by, in degrees, signed the way view
    /// angles are.
    ///
    /// Whatever aims adds this to where the view points before working out
    /// its own error. That is the whole of why the two do not cancel the same
    /// kick twice.
    pub moves: Punch,
}

impl Correction {
    pub fn sends_nothing(self) -> bool {
        self.counts == [0, 0]
    }
}

/// The compensation, carried between passes.
///
/// Holds one thing: how far the view has been moved on the punch's account so
/// far. Everything else is read fresh, which is what lets a reading that goes
/// missing for a pass — a pawn freed at the end of a round — be handled by
/// forgetting rather than by unwinding.
#[derive(Clone, Copy, Debug, Default)]
pub struct Recoil {
    /// Degrees of view movement already spent on the punch, signed as view
    /// angles: pitch positive downwards.
    paid: Punch,
}

impl Recoil {
    /// Work out this pass's movement and record it as sent.
    ///
    /// `hand` is the player's own movement this pass, in counts. What it
    /// covers of the debt is taken off before anything is sent, so the two
    /// never pull down at once.
    pub fn settle(&mut self, punch: Punch, hand: [i64; 2], control: Control) -> Correction {
        if !punch.plausible() || !control.wanted() {
            // Nothing is owed and nothing has been paid. Not a pause — a
            // reading that cannot be trusted must not leave a debt behind to
            // be unwound against the next one that can.
            self.paid = Punch::default();
            return Correction::default();
        }

        // The view has to move by the opposite of the punch: the gun throwing
        // the aim up is pitch going negative, and answering it is pitch going
        // positive, which is downwards.
        let wanted = Punch {
            pitch: -punch.pitch * control.vertical,
            yaw: -punch.yaw * control.sideways,
        };

        // Their movement, as degrees of view. A count downwards raises pitch;
        // a count rightwards lowers yaw.
        let hand = Punch {
            pitch: hand[1] as f32 / control.counts_per_degree,
            yaw: -hand[0] as f32 / control.counts_per_degree,
        };
        self.paid = Punch {
            pitch: self.paid.pitch + towards(hand.pitch, wanted.pitch - self.paid.pitch),
            yaw: self.paid.yaw + towards(hand.yaw, wanted.yaw - self.paid.yaw),
        };

        let pay = Punch {
            pitch: (wanted.pitch - self.paid.pitch) * control.pace,
            yaw: (wanted.yaw - self.paid.yaw) * control.pace,
        };
        // A count to the right lowers yaw, so closing a gap in yaw means
        // sending the opposite sign. Pitch does not: a count downwards raises
        // it, which is the direction an upward kick is answered in.
        let counts = [
            capped(-pay.yaw * control.counts_per_degree, control.cap),
            capped(pay.pitch * control.counts_per_degree, control.cap),
        ];
        let moves = Punch {
            pitch: counts[1] as f32 / control.counts_per_degree,
            yaw: -counts[0] as f32 / control.counts_per_degree,
        };
        self.paid = Punch {
            pitch: self.paid.pitch + moves.pitch,
            yaw: self.paid.yaw + moves.yaw,
        };
        Correction { counts, moves }
    }

    /// Take back a correction that never went out.
    ///
    /// A send Windows refused moved nothing, and a debt recorded as paid when
    /// it was not is a debt that disappears — the compensation would be short
    /// by exactly that much for the rest of the spray, silently.
    pub fn refused(&mut self, correction: Correction) {
        self.paid = Punch {
            pitch: self.paid.pitch - correction.moves.pitch,
            yaw: self.paid.yaw - correction.moves.yaw,
        };
    }

    /// Forget everything, for when there is no longer a gun to compensate.
    pub fn forget(&mut self) {
        self.paid = Punch::default();
    }

    /// What has been spent on the punch so far, for the log to show.
    pub fn paid(self) -> Punch {
        self.paid
    }
}

/// As much of `hand` as goes towards `owed`, and no further than it.
///
/// Movement the same way as the debt pays it, up to the whole of it.
/// Movement the other way is worth nothing — it is the player choosing where
/// to point, and charging them for it would mean pulling back against them.
fn towards(hand: f32, owed: f32) -> f32 {
    if !hand.is_finite() || !owed.is_finite() {
        return 0.0;
    }
    if owed >= 0.0 {
        hand.clamp(0.0, owed)
    } else {
        hand.clamp(owed, 0.0)
    }
}

/// Rounded and held inside the ceiling.
fn capped(counts: f32, cap: f32) -> i32 {
    if !counts.is_finite() || cap <= 0.0 {
        return 0;
    }
    counts.clamp(-cap, cap).round() as i32
}

/// How long after the last round a spray is treated as over.
///
/// Long enough to cover the gap between rounds at any firing rate the game
/// has, short enough that putting the gun away is noticed.
pub const SPRAY_ENDS_AFTER: Duration = Duration::from_millis(400);

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> Control {
        Control {
            vertical: 1.0,
            sideways: 1.0,
            pace: 0.3,
            counts_per_degree: 51.0,
            cap: 60.0,
        }
    }

    fn kicked_up(degrees: f32) -> Punch {
        Punch {
            pitch: -degrees,
            yaw: 0.0,
        }
    }

    /// Run a still punch until the compensation has settled, and give back
    /// what was sent in total.
    fn settled(recoil: &mut Recoil, punch: Punch, passes: usize) -> [i64; 2] {
        let mut sent = [0i64; 2];
        for _ in 0..passes {
            let correction = recoil.settle(punch, [0, 0], control());
            sent[0] += i64::from(correction.counts[0]);
            sent[1] += i64::from(correction.counts[1]);
        }
        sent
    }

    #[test]
    fn a_gun_that_has_not_fired_is_not_compensated() {
        let mut recoil = Recoil::default();
        assert!(
            recoil
                .settle(Punch::default(), [0, 0], control())
                .sends_nothing()
        );
    }

    #[test]
    fn a_kick_upwards_is_answered_downwards() {
        let mut recoil = Recoil::default();
        let correction = recoil.settle(kicked_up(5.0), [0, 0], control());
        assert!(correction.counts[1] > 0, "{correction:?}");
        assert_eq!(correction.counts[0], 0, "nothing sideways was asked for");
        assert!(correction.moves.pitch > 0.0, "downwards, as view angles go");
    }

    #[test]
    fn a_kick_sideways_is_answered_the_other_way() {
        let mut recoil = Recoil::default();
        // Thrown left, which is yaw going up. Answering it is a count to the
        // right, which is a positive one.
        let left = recoil.settle(
            Punch {
                pitch: 0.0,
                yaw: 2.0,
            },
            [0, 0],
            control(),
        );
        assert!(left.counts[0] > 0, "{left:?}");

        let mut recoil = Recoil::default();
        let right = recoil.settle(
            Punch {
                pitch: 0.0,
                yaw: -2.0,
            },
            [0, 0],
            control(),
        );
        assert!(right.counts[0] < 0, "{right:?}");
    }

    #[test]
    fn a_shot_is_paid_across_the_gap_and_not_in_the_pass_it_lands_on() {
        // One pass taking the whole step is a jolt at the firing rate, and a
        // jolt in the right direction is still a jolt.
        let mut recoil = Recoil::default();
        let punch = kicked_up(5.0);
        let first = recoil.settle(punch, [0, 0], control());
        let whole = 5.0 * control().counts_per_degree;
        assert!(
            f32::from(i16::try_from(first.counts[1]).expect("a small number")) < whole * 0.5,
            "{first:?} of {whole}"
        );
    }

    #[test]
    fn what_is_owed_is_paid_in_full_and_not_past_it() {
        let mut recoil = Recoil::default();
        let sent = settled(&mut recoil, kicked_up(5.0), 80);
        let whole = (5.0 * control().counts_per_degree) as i64;
        assert!((sent[1] - whole).abs() <= 2, "{} of {whole}", sent[1]);

        // And it stops. Another eighty passes at the same punch add nothing.
        let after = settled(&mut recoil, kicked_up(5.0), 80);
        assert_eq!(after, [0, 0]);
    }

    #[test]
    fn the_players_own_pull_is_what_pays_first() {
        // Both pulling down at once is how a spray ends up in the floor.
        let mut theirs = Recoil::default();
        let mut ours = Recoil::default();
        let punch = kicked_up(1.0);
        let whole = (1.0 * control().counts_per_degree) as i64;

        // They pull the whole degree themselves on the first pass.
        let covered = theirs.settle(punch, [0, whole], control());
        assert!(covered.sends_nothing(), "{covered:?}");

        // Nobody pulls, and the lane pays all of it.
        let alone = settled(&mut ours, punch, 80);
        assert!((alone[1] - whole).abs() <= 2, "{}", alone[1]);
    }

    #[test]
    fn pulling_further_than_the_debt_does_not_turn_into_a_push_back() {
        let mut recoil = Recoil::default();
        let punch = kicked_up(1.0);
        let far_more = (10.0 * control().counts_per_degree) as i64;
        let correction = recoil.settle(punch, [0, far_more], control());
        assert!(correction.sends_nothing(), "{correction:?}");
        assert!(settled(&mut recoil, punch, 40) == [0, 0], "and stays quiet");
    }

    #[test]
    fn aiming_up_during_a_spray_is_not_charged_for() {
        // Deciding where to point is theirs. Answering it by pulling harder
        // is the fight this exists not to have.
        let mut moved = Recoil::default();
        let mut still = Recoil::default();
        let punch = kicked_up(3.0);
        let upwards = -(2.0 * control().counts_per_degree) as i64;

        let with_hand = moved.settle(punch, [0, upwards], control());
        let without = still.settle(punch, [0, 0], control());
        assert_eq!(with_hand.counts, without.counts, "{with_hand:?}");
    }

    #[test]
    fn the_compensation_unwinds_as_the_punch_decays() {
        // The punch falls back to nothing after a spray. A view held down
        // against a punch that is no longer there is a view aiming at the
        // floor.
        let mut recoil = Recoil::default();
        let down = settled(&mut recoil, kicked_up(5.0), 80);
        assert!(down[1] > 0);

        let back = settled(&mut recoil, Punch::default(), 80);
        assert!(back[1] < 0, "it never came back up: {back:?}");
        assert!((down[1] + back[1]).abs() <= 3, "{down:?} then {back:?}");
    }

    #[test]
    fn a_send_that_never_went_out_is_owed_again() {
        let mut refused = Recoil::default();
        let mut sent = Recoil::default();
        let punch = kicked_up(5.0);

        let correction = refused.settle(punch, [0, 0], control());
        refused.refused(correction);
        let _ = sent.settle(punch, [0, 0], control());

        // The one whose movement was thrown away still owes it; the other
        // does not.
        let next_refused = refused.settle(punch, [0, 0], control());
        let next_sent = sent.settle(punch, [0, 0], control());
        assert!(
            next_refused.counts[1] > next_sent.counts[1],
            "{next_refused:?} against {next_sent:?}"
        );
    }

    #[test]
    fn shares_at_nothing_are_off_rather_than_quiet() {
        // A lane compensating nothing would otherwise pay for itself every
        // pass and move the view by the rounding.
        let off = Control {
            vertical: 0.0,
            sideways: 0.0,
            ..control()
        };
        let mut recoil = Recoil::default();
        assert!(!off.wanted());
        assert!(recoil.settle(kicked_up(20.0), [0, 0], off).sends_nothing());

        // And nothing accumulated while it was off, so turning it on does not
        // pay out a debt from a spray it was not watching.
        assert_eq!(recoil.paid(), Punch::default());
    }

    #[test]
    fn half_a_share_takes_back_half_the_climb() {
        let mut whole = Recoil::default();
        let mut half = Recoil::default();
        let punch = kicked_up(5.0);
        let all = settled(&mut whole, punch, 80);
        let some = i64::from(
            (0..80)
                .map(|_| {
                    half.settle(
                        punch,
                        [0, 0],
                        Control {
                            vertical: 0.5,
                            ..control()
                        },
                    )
                    .counts[1]
                })
                .sum::<i32>(),
        );
        assert!((some * 2 - all[1]).abs() <= 3, "{some} against {all:?}");
    }

    #[test]
    fn a_reading_that_could_not_be_real_leaves_no_debt_behind() {
        let mut recoil = Recoil::default();
        settled(&mut recoil, kicked_up(5.0), 10);
        assert_ne!(recoil.paid(), Punch::default());

        let wrong = Punch {
            pitch: f32::NAN,
            yaw: 0.0,
        };
        assert!(recoil.settle(wrong, [0, 0], control()).sends_nothing());
        assert_eq!(
            recoil.paid(),
            Punch::default(),
            "a debt against a reading nobody believes would be unwound against \
             the next one that is believed"
        );
    }

    #[test]
    fn no_pass_moves_further_than_the_ceiling() {
        let mut recoil = Recoil::default();
        let huge = recoil.settle(kicked_up(40.0), [0, 0], control());
        assert!(huge.counts[1] <= control().cap as i32, "{huge:?}");
    }
}
