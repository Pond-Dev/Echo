//! Which parts of the surface a frame actually touched.
//!
//! Clearing and copying the whole window to draw a few outlines was measured
//! at three quarters of the drawing time, for content covering under two per
//! cent of the screen. Tracking what was touched lets both be done on that
//! part alone.
//!
//! Two rules keep it honest:
//!
//! * A region has to cover *last* frame's marks as well as this one's. Marks
//!   that are not copied over stay on screen after the thing that made them
//!   has moved, and the overlay smears.
//! * When the marks are spread far enough that the regions cover most of the
//!   screen anyway, one whole-screen region is cheaper than many small ones —
//!   each region costs a call, and calls are not free.
//!
//! Pure rectangle arithmetic, so it is tested without a window.

/// Regions to merge into before falling back to a single bounding box.
///
/// A box is four lines that merge into one region, so this is a few dozen
/// boxes' worth. Past it the per-region cost outweighs the pixels saved.
const MAX_REGIONS: usize = 24;

/// Share of the surface past which one whole-screen region wins.
const FULL_SCREEN_SHARE: i64 = 60;

/// A rectangle in surface coordinates. Right and bottom are exclusive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    /// A rectangle around two points, in any order, grown by `margin`.
    ///
    /// The margin is what keeps a thick line's own width inside the region it
    /// reports: a line drawn with a four-pixel pen paints two pixels either
    /// side of the path between its endpoints.
    pub fn around(a: (i32, i32), b: (i32, i32), margin: i32) -> Self {
        Self {
            left: a.0.min(b.0) - margin,
            top: a.1.min(b.1) - margin,
            right: a.0.max(b.0) + margin,
            bottom: a.1.max(b.1) + margin,
        }
    }

    pub const fn is_empty(self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// Widened to `i64` before subtracting, not after.
    ///
    /// Coordinates can reach both ends of `i32` at once, and `right - left`
    /// on those overflows before the cast has a chance to help — a panic in
    /// debug, and in release a negative area that would silently switch off
    /// the whole-screen fallback below.
    pub const fn area(self) -> i64 {
        if self.is_empty() {
            0
        } else {
            (self.right as i64 - self.left as i64) * (self.bottom as i64 - self.top as i64)
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// Whether the two overlap, or sit close enough that merging them costs
    /// less than treating them separately.
    fn near(self, other: Self, slack: i32) -> bool {
        self.left - slack < other.right
            && other.left - slack < self.right
            && self.top - slack < other.bottom
            && other.top - slack < self.bottom
    }

    fn clamp(self, width: i32, height: i32) -> Self {
        Self {
            left: self.left.max(0),
            top: self.top.max(0),
            right: self.right.min(width),
            bottom: self.bottom.min(height),
        }
    }
}

/// The regions one frame marked.
#[derive(Clone, Debug, Default)]
pub struct Dirty {
    regions: Vec<Rect>,
    /// Set once the regions stop being worth tracking separately.
    everything: bool,
    /// The surface marks are clamped to. Zero until the size is known, which
    /// discards everything — correct, since there is no surface to draw on.
    width: i32,
    height: i32,
}

impl Dirty {
    /// Start a new frame on a surface of this size.
    pub fn clear(&mut self, width: i32, height: i32) {
        self.regions.clear();
        self.everything = false;
        self.width = width;
        self.height = height;
    }

    /// Mark the whole surface, for when none of it can be trusted.
    ///
    /// A freshly built bitmap holds uninitialised memory and a window that has
    /// never been painted holds whatever was behind it. Neither is the key
    /// colour, so both are opaque: until every pixel has been painted once,
    /// the parts never drawn on cover the game instead of showing it.
    pub fn mark_everything(&mut self) {
        self.regions.clear();
        self.everything = true;
    }

    pub const fn is_everything(&self) -> bool {
        self.everything
    }

    /// Record that `rect` was drawn on.
    pub fn touch(&mut self, rect: Rect) {
        // Clamped on the way in, not on the way out. A mark entirely past the
        // edge — an enemy just off screen projects to one — has an enormous
        // area, and measuring that against the surface would trip the
        // whole-screen fallback for something that paints no pixels at all.
        let rect = rect.clamp(self.width, self.height);
        if self.everything || rect.is_empty() {
            return;
        }
        // Merging into the first region it meets is enough in practice: the
        // marks that belong together arrive together, because they are the
        // four sides of one box or the lines of one block of text.
        if let Some(existing) = self
            .regions
            .iter_mut()
            .find(|existing| existing.near(rect, 4))
        {
            *existing = existing.union(rect);
            return;
        }
        if self.regions.len() >= MAX_REGIONS {
            self.everything = true;
            self.regions.clear();
            return;
        }
        self.regions.push(rect);
    }

    /// The regions to clear and copy, given what the previous frame marked.
    ///
    /// Both frames' marks are included: this frame's so the new drawing
    /// appears, and the previous frame's so the old drawing is taken away.
    /// Leaving the second out is what makes an overlay smear.
    pub fn combined(&self, previous: &Self, width: i32, height: i32) -> Vec<Rect> {
        let whole = Rect::new(0, 0, width, height);
        if self.everything || previous.everything {
            return vec![whole];
        }

        let mut merged = Self::default();
        merged.clear(width, height);
        for region in self.regions.iter().chain(previous.regions.iter()) {
            merged.touch(*region);
        }
        if merged.everything {
            return vec![whole];
        }

        let covered: i64 = merged.regions.iter().map(|region| region.area()).sum();
        if covered * 100 >= whole.area() * FULL_SCREEN_SHARE {
            return vec![whole];
        }

        merged.regions.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: i32 = 2048;
    const H: i32 = 1152;

    fn dirty(rects: &[Rect]) -> Dirty {
        let mut dirty = Dirty::default();
        dirty.clear(W, H);
        for rect in rects {
            dirty.touch(*rect);
        }
        dirty
    }

    fn empty() -> Dirty {
        let mut dirty = Dirty::default();
        dirty.clear(W, H);
        dirty
    }

    #[test]
    fn the_sides_of_one_box_become_one_region() {
        // Four lines, drawn as a box would draw them.
        let box_ = dirty(&[
            Rect::around((100, 100), (300, 100), 1),
            Rect::around((300, 100), (300, 500), 1),
            Rect::around((300, 500), (100, 500), 1),
            Rect::around((100, 500), (100, 100), 1),
        ]);
        let regions = box_.combined(&empty(), W, H);
        assert_eq!(regions, vec![Rect::new(99, 99, 301, 501)]);
    }

    #[test]
    fn marks_far_apart_stay_apart_rather_than_spanning_the_gap() {
        let corners = dirty(&[Rect::new(0, 0, 100, 100), Rect::new(W - 100, H - 100, W, H)]);
        let regions = corners.combined(&empty(), W, H);
        assert_eq!(regions.len(), 2, "a bounding box would cover the screen");
    }

    #[test]
    fn the_previous_frames_marks_are_included_so_old_drawing_is_taken_away() {
        let now = dirty(&[Rect::new(500, 500, 600, 600)]);
        let before = dirty(&[Rect::new(100, 100, 200, 200)]);

        let regions = now.combined(&before, W, H);
        assert!(regions.contains(&Rect::new(500, 500, 600, 600)), "the new");
        assert!(
            regions.contains(&Rect::new(100, 100, 200, 200)),
            "the old, without which it would stay on screen"
        );
    }

    #[test]
    fn regions_are_clamped_to_the_surface() {
        // A player at the edge of the screen reports marks past it.
        let edge = dirty(&[
            Rect::new(-50, -50, 50, 50),
            Rect::new(W - 10, H - 10, W + 90, H + 90),
        ]);
        for region in edge.combined(&empty(), W, H) {
            assert!(region.left >= 0 && region.top >= 0, "{region:?}");
            assert!(region.right <= W && region.bottom <= H, "{region:?}");
        }
    }

    #[test]
    fn enough_scattered_marks_fall_back_to_one_whole_screen_region() {
        // More separate marks than are worth tracking.
        let mut many = empty();
        for index in 0..(MAX_REGIONS as i32 + 5) {
            let x = index * 60;
            many.touch(Rect::new(x, 0, x + 10, 10));
        }
        assert_eq!(many.combined(&empty(), W, H), vec![Rect::new(0, 0, W, H)]);
    }

    #[test]
    fn marks_covering_most_of_the_screen_fall_back_rather_than_paying_per_region() {
        let wide = dirty(&[
            Rect::new(0, 0, W, H / 2 - 100),
            Rect::new(0, H / 2 + 100, W, H),
        ]);
        assert_eq!(wide.combined(&empty(), W, H), vec![Rect::new(0, 0, W, H)]);
    }

    #[test]
    fn a_surface_that_cannot_be_trusted_is_repainted_whole() {
        // The state after the bitmap is built: nothing on it is the key
        // colour yet, so anything left alone would cover the game.
        let mut untrusted = empty();
        untrusted.mark_everything();

        let small = dirty(&[Rect::new(10, 10, 20, 20)]);
        assert_eq!(
            small.combined(&untrusted, W, H),
            vec![Rect::new(0, 0, W, H)],
            "one small mark this frame must not leave the rest untouched"
        );
        assert_eq!(
            untrusted.combined(&empty(), W, H),
            vec![Rect::new(0, 0, W, H)]
        );
    }

    #[test]
    fn a_frame_that_drew_nothing_after_a_frame_that_did_still_clears_the_old_marks() {
        let nothing = empty();
        let before = dirty(&[Rect::new(100, 100, 200, 200)]);
        assert_eq!(
            nothing.combined(&before, W, H),
            vec![Rect::new(100, 100, 200, 200)]
        );
    }
}
