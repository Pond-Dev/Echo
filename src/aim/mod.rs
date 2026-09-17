//! Head AimLock: current camera to current head bone.

use crate::game::{LocalPlayer, Player, ViewAngles};

pub const CONE: f32 = 1.5;
pub const GAIN: f32 = 0.7;
pub const DEADZONE: f32 = 0.03;
// Calibration for the player's mouse sensitivity.
pub const COUNTS_PER_DEGREE: f32 = 51.0;
const CAP: f32 = 60.0;
pub const CROSSHAIR_WEIGHT: f32 = 0.5;
pub const SWITCH_MARGIN: f32 = 0.05;
const DISTANCE_SCALE: f32 = 1000.0;

pub fn distance(eye: [f32; 3], head: [f32; 3]) -> f32 {
    (head[0] - eye[0])
        .hypot(head[1] - eye[1])
        .hypot(head[2] - eye[2])
}

/// Lower is better. Distance is bounded so it cannot overwhelm crosshair proximity.
pub fn score(at: Offset, distance: f32) -> f32 {
    CROSSHAIR_WEIGHT * at.size() / CONE
        + (1.0 - CROSSHAIR_WEIGHT) * (distance / (distance + DISTANCE_SCALE))
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Offset {
    pub yaw: f32,
    pub pitch: f32,
}

impl Offset {
    pub fn size(self) -> f32 {
        self.yaw.hypot(self.pitch)
    }
}

pub fn look_at(eye: [f32; 3], head: [f32; 3]) -> Option<ViewAngles> {
    let [dx, dy, dz] = [head[0] - eye[0], head[1] - eye[1], head[2] - eye[2]];
    let flat = dx.hypot(dy);
    if !flat.is_finite() || !dz.is_finite() || (flat == 0.0 && dz == 0.0) {
        return None;
    }
    Some(ViewAngles {
        pitch: -dz.atan2(flat).to_degrees(),
        yaw: dy.atan2(dx).to_degrees(),
    })
}

pub fn offset(view: ViewAngles, desired: ViewAngles) -> Offset {
    Offset {
        yaw: desired.turn_from(view),
        pitch: desired.pitch - view.pitch,
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Choice {
    pub pawn: usize,
    pub offset: Offset,
    pub distance: f32,
    pub score: f32,
    /// How far up the target the aim point sits, from its own feet. This is
    /// the one number that says head or body: a standing player's eyes are at
    /// 64, so anything well under that is the neck and chest.
    pub aim_above_origin: Option<f32>,
    pub counts: [i32; 2],
}

pub fn choose(
    me: Option<LocalPlayer>,
    players: &[Player],
    view: ViewAngles,
    locked: Option<usize>,
) -> Result<Choice, &'static str> {
    let me = me
        .filter(|me| me.plausible() && me.team.plays())
        .ok_or("no-local-player")?;
    if !me.alive() {
        return Err("not-alive");
    }
    if !view.plausible() {
        return Err("invalid-view");
    }
    let eye = players
        .iter()
        .find(|p| p.pawn == me.pawn)
        .and_then(|p| p.eye)
        .filter(|eye| eye.iter().all(|v| v.is_finite()))
        .ok_or("missing-eye")?;
    let mut choice = players
        .iter()
        .filter(|p| p.pawn != me.pawn && p.plausible() && p.alive() && me.team.opposes(p.team))
        .filter_map(|p| {
            let head = p.head?;
            let at = offset(view, look_at(eye, head)?);
            let distance = distance(eye, head);
            (at.size() <= CONE && distance.is_finite()).then_some(Choice {
                pawn: p.pawn,
                offset: at,
                distance,
                score: score(at, distance),
                aim_above_origin: p.origin.map(|origin| head[2] - origin[2]),
                counts: [0, 0],
            })
        })
        .min_by(|a, b| {
            // A small advantage is not enough to take the current target away.
            let a_locked = Some(a.pawn) == locked;
            let b_locked = Some(b.pawn) == locked;
            let a_score = a.score - if a_locked { SWITCH_MARGIN } else { 0.0 };
            let b_score = b.score - if b_locked { SWITCH_MARGIN } else { 0.0 };
            a_score
                .total_cmp(&b_score)
                .then_with(|| b_locked.cmp(&a_locked))
                .then_with(|| a.pawn.cmp(&b.pawn))
        })
        .ok_or("no-head-in-cone")?;
    let at = choice.offset;
    choice.counts = if at.size() <= DEADZONE {
        [0, 0]
    } else {
        let scale = COUNTS_PER_DEGREE * GAIN;
        [
            (-at.yaw * scale / CAP).tanh(),
            (at.pitch * scale / CAP).tanh(),
        ]
        .map(|v| (CAP * v).round() as i32)
    };
    Ok(choice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Team;

    fn scene() -> (LocalPlayer, Vec<Player>, ViewAngles) {
        let me = LocalPlayer {
            pawn: 1,
            health: 100,
            team: Team::Terrorist,
        };
        let player = Player {
            pawn: 1,
            health: 100,
            team: me.team,
            origin: Some([0.0; 3]),
            eye: Some([0.0, 0.0, 46.0]),
            head: None,
        };
        let enemy = Player {
            pawn: 2,
            team: Team::CounterTerrorist,
            origin: Some([1000.0, 0.0, 0.0]),
            head: Some([1000.0, 0.0, 46.0]),
            ..player
        };
        (
            me,
            vec![player, enemy],
            ViewAngles {
                pitch: 0.0,
                yaw: 0.0,
            },
        )
    }

    #[test]
    fn crouched_head_is_measured_from_the_current_camera_only() {
        let (me, mut players, mut view) = scene();
        assert_eq!(
            choose(Some(me), &players, view, None).unwrap().counts,
            [0, 0]
        );
        view.pitch = -0.67;
        let choice = choose(Some(me), &players, view, None).unwrap();
        assert!((choice.offset.pitch - 0.67).abs() < 0.001);
        assert!(choice.counts[1] > 0);
        players[1].head = Some([1000.0, 10.0, 48.0]);
        view = look_at(players[0].eye.unwrap(), players[1].head.unwrap()).unwrap();
        assert_eq!(
            choose(Some(me), &players, view, None).unwrap().counts,
            [0, 0]
        );
    }

    #[test]
    fn switches_to_a_better_crosshair_target_and_replaces_invalid_owners() {
        let (me, mut players, view) = scene();
        players.push(Player {
            pawn: 3,
            ..players[1]
        });
        players[1].head = Some([1000.0, 20.0, 46.0]);
        assert_eq!(choose(Some(me), &players, view, Some(2)).unwrap().pawn, 3);
        players[1].head = Some([1000.0, 30.0, 46.0]);
        assert_eq!(choose(Some(me), &players, view, Some(2)).unwrap().pawn, 3);
        players[1].head = Some([1000.0, 0.0, 46.0]);
        players[1].health = 0;
        assert_eq!(choose(Some(me), &players, view, Some(2)).unwrap().pawn, 3);
    }

    #[test]
    fn equally_weighted_distance_can_win_with_a_slightly_worse_crosshair_angle() {
        let (me, mut players, view) = scene();
        players[1].head = Some([3000.0, 0.0, 46.0]);
        players.push(Player {
            pawn: 3,
            head: Some([500.0, 4.0, 46.0]),
            ..players[1]
        });
        let choice = choose(Some(me), &players, view, Some(2)).unwrap();
        assert_eq!(choice.pawn, 3);
        assert!(choice.distance < 501.0);
        assert!(choice.offset.size() > 0.4);

        // A nearby head still cannot be selected outside the cone.
        players[2].head = Some([500.0, 20.0, 46.0]);
        assert_eq!(choose(Some(me), &players, view, Some(3)).unwrap().pawn, 2);
    }

    #[test]
    fn small_score_changes_keep_the_lock_regardless_of_roster_order() {
        let (me, mut players, view) = scene();
        players[1].head = Some([1000.0, 10.0, 46.0]);
        players.push(Player {
            pawn: 3,
            head: Some([1000.0, 9.0, 46.0]),
            ..players[1]
        });
        assert_eq!(choose(Some(me), &players, view, None).unwrap().pawn, 3);
        assert_eq!(choose(Some(me), &players, view, Some(2)).unwrap().pawn, 2);
        players.swap(1, 2);
        assert_eq!(choose(Some(me), &players, view, Some(2)).unwrap().pawn, 2);
        assert_eq!(choose(Some(me), &players, view, Some(3)).unwrap().pawn, 3);

        // Once the challenger is clearly better, it can take over.
        players[1].head = Some([1000.0, 0.0, 46.0]);
        assert_eq!(choose(Some(me), &players, view, Some(2)).unwrap().pawn, 3);
    }

    #[test]
    fn missing_invalid_and_friendly_geometry_never_moves_the_mouse() {
        let (me, mut players, view) = scene();
        players[1].head = None;
        assert!(choose(Some(me), &players, view, None).is_err());
        players[1].head = Some([f32::NAN, 0.0, 46.0]);
        assert!(choose(Some(me), &players, view, None).is_err());
        players[1].head = Some([1000.0, 0.0, 46.0]);
        players[1].team = me.team;
        assert!(choose(Some(me), &players, view, None).is_err());
        players[1].team = Team::CounterTerrorist;
        players[0].eye = None;
        assert!(choose(Some(me), &players, view, None).is_err());
        assert!(choose(Some(LocalPlayer { health: 0, ..me }), &players, view, None).is_err());
    }

    #[test]
    fn angular_seam_and_cone_limits_keep_the_short_direction() {
        let delta = offset(
            ViewAngles {
                pitch: 0.0,
                yaw: 179.5,
            },
            ViewAngles {
                pitch: 0.0,
                yaw: -179.5,
            },
        );
        assert!((delta.yaw - 1.0).abs() < 0.001);
        let (me, players, mut view) = scene();
        view.yaw = CONE;
        assert!(choose(Some(me), &players, view, None).is_ok());
        view.yaw = CONE + 0.01;
        assert!(choose(Some(me), &players, view, None).is_err());
    }
}
