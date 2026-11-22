//! WinUI `ProgressBar` and `ProgressRing`, drawn with the tokens from their
//! `*_themeresources.xaml`. Animation time comes from the app, so idle frames
//! stay free.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

use crate::gfx::{Surface, rect};
use crate::theme::Theme;

/// `ProgressBarMinHeight` / `ProgressBarTrackHeight`.
pub const BAR_HEIGHT: f32 = 3.0;
/// `ProgressBarTrackHeight`.
pub const TRACK_HEIGHT: f32 = 1.0;
/// `ProgressBarCornerRadius` / `ProgressBarTrackCornerRadius`.
pub const BAR_RADIUS: f32 = 1.5;
/// `ProgressBarTrackCornerRadius`.
pub const TRACK_RADIUS: f32 = 0.5;
/// `ProgressRingStrokeThickness`.
pub const RING_STROKE: f32 = 4.0;

/// `ProgressBar` with a clamped fraction of `Maximum`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressBar {
    /// `0..Maximum`; values outside are clamped when drawing.
    pub value: f32,
    /// `Maximum`, must be positive for meaningful output.
    pub maximum: f32,
}

impl ProgressBar {
    /// A bar at `value` of `maximum`.
    pub fn new(value: f32, maximum: f32) -> Self {
        Self { value, maximum }
    }

    /// Fraction in `0..=1`.
    pub fn fraction(&self) -> f32 {
        if self.maximum <= 0.0 { 0.0 } else { (self.value / self.maximum).clamp(0.0, 1.0) }
    }

    /// Geometry of the filled part inside `bounds`.
    pub fn fill(&self, bounds: D2D_RECT_F) -> D2D_RECT_F {
        let mid = (bounds.top + bounds.bottom) / 2.0;
        rect(bounds.left, mid - TRACK_HEIGHT / 2.0,
            bounds.left + (bounds.right - bounds.left) * self.fraction(),
            mid + TRACK_HEIGHT / 2.0)
    }

    /// Draws the 1 DIP track and the accent fill.
    pub fn draw(&self, s: &Surface, t: &Theme, bounds: D2D_RECT_F) {
        let mid = (bounds.top + bounds.bottom) / 2.0;
        let track = rect(bounds.left, mid - TRACK_HEIGHT / 2.0, bounds.right, mid + TRACK_HEIGHT / 2.0);
        let (track_color, fill_color) = if let Some(c) = t.high_contrast {
            (c.window, c.highlight)
        } else {
            (t.control_strong_stroke, t.accent)
        };
        s.fill_round(track, TRACK_RADIUS, track_color);
        let fill = self.fill(bounds);
        if fill.right > fill.left {
            s.fill_round(fill, BAR_RADIUS, fill_color);
        }
    }
}

/// `ProgressRing`. `Some(fraction)` is determinate, `None` is indeterminate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProgressRing {
    /// Center in client DIPs.
    pub center: (f32, f32),
    /// Radius of the stroke's centerline.
    pub radius: f32,
    /// Determinate value, or `None` for the looping arc.
    pub value: Option<f32>,
}

impl ProgressRing {
    /// Draws the ring. `time` (seconds) rotates the indeterminate arc.
    pub fn draw(&self, s: &Surface, t: &Theme, time: f32) {
        let color = if let Some(c) = t.high_contrast { c.highlight } else { t.accent };
        let (cx, cy) = self.center;
        match self.value {
            Some(v) => {
                let sweep = v.clamp(0.0, 1.0) * std::f32::consts::TAU;
                s.stroke_arc(cx, cy, self.radius, -std::f32::consts::FRAC_PI_2,
                    -std::f32::consts::FRAC_PI_2 + sweep, RING_STROKE, color);
            }
            None => {
                // WinUI's indeterminate ring sweeps a ~300 degree arc; the
                // leading edge accelerates and eases near the end of each loop.
                let phase = time.rem_euclid(2.0) / 2.0;
                let eased = phase * phase * (3.0 - 2.0 * phase);
                let start = -std::f32::consts::FRAC_PI_2 + eased * std::f32::consts::TAU;
                s.stroke_arc(cx, cy, self.radius, start,
                    start + std::f32::consts::TAU * 5.0 / 6.0, RING_STROKE, color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_should_clamp_and_handle_a_zero_maximum() {
        assert_eq!(ProgressBar::new(5.0, 10.0).fraction(), 0.5);
        assert_eq!(ProgressBar::new(-1.0, 10.0).fraction(), 0.0);
        assert_eq!(ProgressBar::new(20.0, 10.0).fraction(), 1.0);
        assert_eq!(ProgressBar::new(5.0, 0.0).fraction(), 0.0);
    }

    #[test]
    fn fill_should_be_a_one_dip_strip_scaled_by_the_fraction() {
        let b = rect(10.0, 20.0, 110.0, 23.0);
        let f = ProgressBar::new(25.0, 100.0).fill(b);
        assert_eq!((f.left, f.top, f.right, f.bottom), (10.0, 21.0, 35.0, 22.0));
    }

    #[test]
    fn an_empty_bar_should_have_no_fill_width() {
        let b = rect(0.0, 0.0, 100.0, 3.0);
        let f = ProgressBar::new(0.0, 10.0).fill(b);
        assert!(f.right <= f.left);
    }
}
