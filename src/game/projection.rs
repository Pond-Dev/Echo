//! Turning a place in the world into a place on the screen.
//!
//! The game keeps the matrix it renders with, so the same transform it used
//! to draw a player is available to us. Multiply a world point by it, divide
//! by the resulting `w`, and the result is in normalised device coordinates —
//! `-1` to `1` across the screen — which scale to pixels.
//!
//! The `w` divide is also the only thing standing between us and drawing
//! boxes behind our own head. A point behind the camera comes out with `w`
//! zero or negative, and dividing by it mirrors the point back into view at a
//! plausible-looking position. Rejecting those is not an optimisation.
//!
//! Pure arithmetic, so all of it is tested without a game running.

/// The 4×4 matrix the game renders with, in the order it stores it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewMatrix([f32; 16]);

/// Anything closer to the camera plane than this is treated as behind it.
/// Exactly zero would divide by zero; a small positive margin also throws
/// away points grazing the plane, where the projection explodes.
const NEAR: f32 = 0.01;

/// Pixels beyond which a projection is not kept.
///
/// A point just in front of the camera plane projects to coordinates in the
/// billions, and `as i32` saturates them to the extremes. Two saturated
/// coordinates subtracted from one another then overflow — a panic in debug
/// and a wrapped, meaningless size in release. Nothing that far outside the
/// screen can be drawn usefully, so it is rejected while it is still a float.
const LIMIT: f32 = 32_000.0;

impl ViewMatrix {
    pub const fn from_raw(values: [f32; 16]) -> Self {
        Self(values)
    }

    /// Where a world point lands on a screen of this size.
    ///
    /// `None` means it does not land anywhere: behind the camera, or the
    /// matrix was not finite because it was read mid-write.
    pub fn project(&self, world: [f32; 3], width: i32, height: i32) -> Option<(i32, i32)> {
        let m = &self.0;
        let [x, y, z] = world;

        let clip_x = m[0] * x + m[1] * y + m[2] * z + m[3];
        let clip_y = m[4] * x + m[5] * y + m[6] * z + m[7];
        let w = m[12] * x + m[13] * y + m[14] * z + m[15];

        // `is_finite` first, so a matrix read mid-write is rejected before the
        // comparison — a NaN compares false against everything, including the
        // test that is meant to catch it.
        if !w.is_finite() || w <= NEAR || !clip_x.is_finite() || !clip_y.is_finite() {
            return None;
        }

        // Screen y counts downwards while the device coordinate counts up.
        let screen_x = (width as f32 / 2.0) * (1.0 + clip_x / w);
        let screen_y = (height as f32 / 2.0) * (1.0 - clip_y / w);

        (screen_x.abs() <= LIMIT && screen_y.abs() <= LIMIT)
            .then_some((screen_x.round() as i32, screen_y.round() as i32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A matrix that passes x and y straight through with `w = 1`, so a world
    /// point is already in device coordinates. Makes the screen mapping the
    /// only thing under test.
    fn passthrough() -> ViewMatrix {
        ViewMatrix::from_raw([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 0.0, 1.0,
        ])
    }

    /// A matrix whose `w` is the world's z, so a point's depth decides whether
    /// it is in front of the camera.
    fn depth_is_z() -> ViewMatrix {
        ViewMatrix::from_raw([
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.0, 0.0, 1.0, 0.0, //
            0.0, 0.0, 1.0, 0.0,
        ])
    }

    #[test]
    fn the_centre_of_the_device_is_the_centre_of_the_screen() {
        assert_eq!(
            passthrough().project([0.0, 0.0, 0.0], 800, 600),
            Some((400, 300))
        );
    }

    #[test]
    fn the_corners_map_to_the_corners_and_y_is_flipped() {
        let m = passthrough();
        // Device (-1, 1) is the top left; (1, -1) is the bottom right.
        assert_eq!(m.project([-1.0, 1.0, 0.0], 800, 600), Some((0, 0)));
        assert_eq!(m.project([1.0, -1.0, 0.0], 800, 600), Some((800, 600)));
    }

    #[test]
    fn a_point_behind_the_camera_lands_nowhere_rather_than_being_mirrored_in_front() {
        let m = depth_is_z();
        assert!(m.project([0.5, 0.5, 1.0], 800, 600).is_some(), "in front");

        // Without the check, negative w flips the sign of both coordinates and
        // this would draw a box behind our own head as if it were ahead.
        assert_eq!(m.project([0.5, 0.5, -1.0], 800, 600), None);
        assert_eq!(
            m.project([0.5, 0.5, 0.0], 800, 600),
            None,
            "exactly on the plane"
        );
        assert_eq!(
            m.project([0.5, 0.5, NEAR / 2.0], 800, 600),
            None,
            "grazing the plane, where the projection explodes"
        );
    }

    #[test]
    fn a_point_grazing_the_camera_plane_is_rejected_before_it_can_overflow() {
        // Just past NEAR, so the w check lets it through and the divide
        // explodes. Saturating two of these into i32 and subtracting them
        // overflows, which is a panic in debug and nonsense in release.
        let m = depth_is_z();
        let grazing = m.project([5_000.0, 5_000.0, NEAR * 1.001], 800, 600);
        assert_eq!(grazing, None);

        // A box straddling the screen edge is still worth keeping.
        assert!(m.project([1.5, -1.2, 1.0], 800, 600).is_some());
    }

    #[test]
    fn a_matrix_caught_mid_write_produces_nothing_instead_of_a_box_at_a_wild_place() {
        let broken = ViewMatrix::from_raw([f32::NAN; 16]);
        assert_eq!(broken.project([1.0, 2.0, 3.0], 800, 600), None);

        let infinite = ViewMatrix::from_raw([f32::INFINITY; 16]);
        assert_eq!(infinite.project([1.0, 2.0, 3.0], 800, 600), None);
    }
}
