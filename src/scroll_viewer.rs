//! Win11 vertical `ScrollBar` for a `ScrollViewer`: 12 DIP track, 8–30 DIP
//! thumb with corner radius 3 and `ControlStrongFillColorDefault`. The app owns
//! the offset; this module only does geometry, hit testing and drawing.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::gfx::{Surface, rect};
use crate::theme::Theme;

/// `ScrollBarSize`.
pub const BAR_WIDTH: f32 = 12.0;
/// `ScrollBarVerticalThumbMinHeight`.
pub const THUMB_MIN: f32 = 30.0;
/// `ScrollBarVerticalThumbMinWidth` / `ScrollBarCornerRadius`.
pub const THUMB_WIDTH: f32 = 8.0;
/// `ScrollBarCornerRadius`.
pub const RADIUS: f32 = 3.0;
/// `ScrollBarThumbOffset`: thumb inset from the track edge.
pub const THUMB_OFFSET: f32 = 2.0;

/// Where a pointer press landed on the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// On the thumb; start dragging.
    Thumb,
    /// On the track above the thumb: page up.
    Above,
    /// On the track below the thumb: page down.
    Below,
    /// Outside the bar.
    None,
}

/// Geometry and drag state of one vertical scroll bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollBar {
    dragging: bool,
    grab: f32,
}

impl Default for ScrollBar {
    fn default() -> Self {
        Self::new()
    }
}

impl ScrollBar {
    /// A bar at rest.
    pub fn new() -> Self {
        Self { dragging: false, grab: 0.0 }
    }

    /// True while the thumb is being dragged.
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    /// Thumb rectangle inside `track` for `content`/`viewport`/`offset` (DIPs).
    pub fn thumb(&self, track: D2D_RECT_F, content: f32, viewport: f32, offset: f32) -> D2D_RECT_F {
        let height = (track.bottom - track.top).max(0.0);
        if content <= viewport || height <= 0.0 {
            let y = track.top;
            return rect(track.left + THUMB_OFFSET, y, track.left + THUMB_OFFSET + THUMB_WIDTH, y + height.min(THUMB_MIN));
        }
        let len = (height * viewport / content).max(THUMB_MIN).min(height);
        let travel = height - len;
        let y = track.top + travel * (offset / (content - viewport)).clamp(0.0, 1.0);
        rect(track.left + THUMB_OFFSET, y, track.left + THUMB_OFFSET + THUMB_WIDTH, y + len)
    }

    /// Classifies a point in the same coordinate space as `track`.
    pub fn hit(&self, x: f32, y: f32, track: D2D_RECT_F, content: f32, viewport: f32, offset: f32) -> Hit {
        if x < track.left || x > track.right || y < track.top || y > track.bottom {
            return Hit::None;
        }
        if content <= viewport {
            return Hit::None;
        }
        let thumb = self.thumb(track, content, viewport, offset);
        if y >= thumb.top && y <= thumb.bottom {
            Hit::Thumb
        } else if y < thumb.top {
            Hit::Above
        } else {
            Hit::Below
        }
    }

    /// Starts a drag at `y`, remembering the grab point inside the thumb.
    pub fn press(&mut self, y: f32, thumb: D2D_RECT_F) {
        self.dragging = true;
        self.grab = y - thumb.top;
    }

    /// New scroll offset while dragging, or `None` when not dragging.
    pub fn drag(&self, y: f32, track: D2D_RECT_F, content: f32, viewport: f32) -> Option<f32> {
        if !self.dragging || content <= viewport {
            return None;
        }
        let thumb = self.thumb(track, content, viewport, 0.0);
        let travel = (track.bottom - track.top) - (thumb.bottom - thumb.top);
        if travel <= 0.0 {
            return Some(0.0);
        }
        Some(((y - self.grab - track.top) / travel).clamp(0.0, 1.0) * (content - viewport))
    }

    /// Ends a drag.
    pub fn release(&mut self) {
        self.dragging = false;
    }

    /// Draws the thumb; the track stays transparent like `ScrollBarBackground`.
    pub fn draw(&self, s: &Surface, t: &Theme, track: D2D_RECT_F, content: f32, viewport: f32, offset: f32) {
        if content <= viewport {
            return;
        }
        let thumb = self.thumb(track, content, viewport, offset);
        let color = if let Some(c) = t.high_contrast {
            if self.dragging { c.highlight } else { c.button_text }
        } else {
            t.control_strong_fill
        };
        s.fill_round(thumb, RADIUS, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> D2D_RECT_F {
        rect(0.0, 0.0, 12.0, 100.0)
    }

    #[test]
    fn thumb_should_scale_with_the_visible_fraction() {
        let bar = ScrollBar::new();
        let t = bar.thumb(track(), 400.0, 100.0, 0.0);
        assert_eq!((t.left, t.top, t.right, t.bottom), (2.0, 0.0, 10.0, 30.0));
        let t = bar.thumb(track(), 400.0, 100.0, 300.0);
        assert_eq!((t.top, t.bottom), (70.0, 100.0));
    }

    #[test]
    fn thumb_should_never_be_shorter_than_30() {
        let t = ScrollBar::new().thumb(track(), 10_000.0, 100.0, 0.0);
        assert_eq!(t.bottom - t.top, THUMB_MIN);
    }

    #[test]
    fn a_content_that_fits_should_have_no_bar() {
        let bar = ScrollBar::new();
        assert_eq!(bar.hit(5.0, 10.0, track(), 100.0, 100.0, 0.0), Hit::None);
    }

    #[test]
    fn hit_should_classify_thumb_and_track() {
        let bar = ScrollBar::new();
        assert_eq!(bar.hit(5.0, 10.0, track(), 400.0, 100.0, 0.0), Hit::Thumb);
        assert_eq!(bar.hit(5.0, 50.0, track(), 400.0, 100.0, 0.0), Hit::Below);
        assert_eq!(bar.hit(20.0, 10.0, track(), 400.0, 100.0, 0.0), Hit::None);
    }

    #[test]
    fn dragging_should_follow_the_pointer_and_clamp() {
        let mut bar = ScrollBar::new();
        bar.press(10.0, bar.thumb(track(), 400.0, 100.0, 0.0));
        assert_eq!(bar.drag(10.0, track(), 400.0, 100.0), Some(0.0));
        assert_eq!(bar.drag(85.0, track(), 400.0, 100.0), Some(300.0));
        bar.release();
        assert_eq!(bar.drag(85.0, track(), 400.0, 100.0), None);
    }
}
