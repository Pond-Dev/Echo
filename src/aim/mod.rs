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
    /// The most counts one pass may send on one axis.
    ///
    /// The ceiling on what a wrong reading can do. Without it, a position read
    /// mid-write asks for a movement of any size at all, and the view is
    /// somewhere else before the next pass can disagree.
    pub cap: i32,
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
    /// The mouse movement that closes `offset`, or nothing when it is already
    /// close enough.
    pub fn counts(self, offset: Offset) -> Option<[i32; 2]> {
        if !offset.size().is_finite() || offset.size() < self.deadzone {
            return None;
        }
        let scale = self.counts_per_degree * self.gain;
        // Yaw is negated: a mouse count to the right turns the view right,
        // which the engine records as yaw falling. Pitch is not: a count
        // downwards looks downwards, which it records as pitch rising.
        let movement = [
            clamp_counts(-offset.yaw * scale, self.cap),
            clamp_counts(offset.pitch * scale, self.cap),
        ];
        (movement != [0, 0]).then_some(movement)
    }

    /// Whether something this far off is worth steering towards at all.
    pub fn within_cone(self, offset: Offset) -> bool {
        offset.size() <= self.cone
    }
}

/// Rounded, then held inside the cap — and never turned into a movement of
/// nothing by the rounding, since a fraction of a count still means the view
/// is not there yet.
fn clamp_counts(counts: f32, cap: i32) -> i32 {
    if !counts.is_finite() {
        return 0;
    }
    let rounded = if counts.abs() < 1.0 {
        counts.signum() * counts.abs().ceil()
    } else {
        counts.round()
    };
    (rounded as i32).clamp(-cap, cap)
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
            cap: 200,
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
            .counts(Offset {
                yaw: 10.0,
                pitch: 0.0,
            })
            .expect("a movement");
        assert!(left[0] < 0, "{left:?}");

        let right = steering()
            .counts(Offset {
                yaw: -10.0,
                pitch: 0.0,
            })
            .expect("a movement");
        assert!(right[0] > 0, "{right:?}");
    }

    #[test]
    fn a_target_below_is_reached_by_moving_the_mouse_down() {
        // Pitch counts downwards, so a target at a higher pitch is below, and
        // reaching it means a positive mouse movement.
        let below = steering()
            .counts(Offset {
                yaw: 0.0,
                pitch: 10.0,
            })
            .expect("a movement");
        assert!(below[1] > 0, "{below:?}");
    }

    #[test]
    fn each_pass_covers_its_share_of_the_distance_and_leaves_the_rest() {
        // Ten degrees at fifty counts a degree is five hundred counts to go;
        // half of that is two hundred and fifty, which the cap trims to two
        // hundred. Below the cap the share is exact.
        let close = steering()
            .counts(Offset {
                yaw: -4.0,
                pitch: 0.0,
            })
            .expect("a movement");
        assert_eq!(close, [100, 0], "half of four degrees at fifty a degree");
    }

    #[test]
    fn no_single_pass_may_move_further_than_the_cap() {
        // What a position read mid-write looks like: an enormous distance.
        let wild = steering()
            .counts(Offset {
                yaw: -170.0,
                pitch: 80.0,
            })
            .expect("a movement");
        assert_eq!(wild, [200, 200]);
    }

    #[test]
    fn a_view_already_on_target_is_left_alone_rather_than_trembling() {
        assert_eq!(
            steering().counts(Offset {
                yaw: 0.1,
                pitch: 0.1
            }),
            None,
            "inside the deadzone"
        );
        // And just outside it, a movement that rounding must not swallow: a
        // third of a count is still the view not being there.
        let tiny = Steering {
            counts_per_degree: 1.0,
            gain: 0.001,
            ..steering()
        };
        assert_eq!(
            tiny.counts(Offset {
                yaw: -1.0,
                pitch: 0.0
            }),
            Some([1, 0])
        );
    }

    #[test]
    fn a_reading_that_was_not_a_number_moves_nothing() {
        assert_eq!(
            steering().counts(Offset {
                yaw: f32::NAN,
                pitch: 0.0
            }),
            None
        );
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
