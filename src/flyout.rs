//! WinUI `Flyout` pieces for a popup window: placement next to a target
//! (`FlyoutBase::CalculatePlacementPrivate`), the open/close animation
//! (`PopupThemeTransition` = uxtheme `TAS_SHOWPOPUP` / `TAS_HIDEPOPUP`) and the
//! `FlyoutPresenter` chrome. The window itself (a `WS_POPUP` with the
//! `DWMSBT_TRANSIENTWINDOW` backdrop, see [`crate::backdrop`]) belongs to the app.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};

use crate::easing::{self, CubicBezier};
use crate::gfx::{Surface, rect};
use crate::theme::{self, Theme};

/// `FlyoutBase::FlyoutMargin`: gap to the target and to the container edges.
pub const MARGIN: f32 = 4.0;
/// `FlyoutBase::g_entranceThemeOffset`: the open slide starts this far away.
pub const ENTRANCE_OFFSET: f32 = 50.0;
/// `FlyoutContentPadding` = 16,15,16,17 (left, top, right, bottom).
pub const PADDING: (f32, f32, f32, f32) = (16.0, 15.0, 16.0, 17.0);
/// `FlyoutThemeMinWidth` / `FlyoutThemeMaxWidth`.
pub const WIDTH_RANGE: (f32, f32) = (96.0, 456.0);
/// `FlyoutThemeMinHeight` / `FlyoutThemeMaxHeight`.
pub const HEIGHT_RANGE: (f32, f32) = (40.0, 758.0);

/// `FlyoutPlacementMode` (major side; justification centered).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Above the target (the WinUI default).
    Top,
    /// Below the target.
    Bottom,
    /// Left of the target.
    Left,
    /// Right of the target.
    Right,
}

impl Placement {
    /// Fallback order of `CalculatePlacementPrivate` starting from `self`.
    fn order(self) -> [Placement; 4] {
        use Placement::{Bottom, Left, Right, Top};
        match self {
            Top => [Top, Bottom, Left, Right],
            Bottom => [Bottom, Top, Left, Right],
            Left => [Left, Right, Top, Bottom],
            Right => [Right, Left, Top, Bottom],
        }
    }

    fn vertical(self) -> bool {
        matches!(self, Placement::Top | Placement::Bottom)
    }
}

/// Places a `width` x `height` flyout next to `target` inside `container` (the
/// monitor work area), trying `preferred` then its fallbacks. Returns the
/// flyout rectangle and the placement used. Same coordinate space for all.
pub fn place(target: D2D_RECT_F, width: f32, height: f32, container: D2D_RECT_F, preferred: Placement) -> (D2D_RECT_F, Placement) {
    // shortcut: no ResizeToFit (fb.cpp L2635); a flyout larger than the container is just clamped to its size here.
    let (width, height) = (width.min(container.right - container.left), height.min(container.bottom - container.top));
    // PerformPlacementWithFallback: the first placement that fits, else the first choice.
    let order = preferred.order();
    let first = try_placement(target, width, height, container, order[0]);
    let fallback = order[1..].iter()
        .map(|&p| (try_placement(target, width, height, container, p), p))
        .find(|((fits, _), _)| *fits)
        .map(|((_, pos), p)| (pos, p));
    let fits = first.0 || fallback.is_some();
    let (mut x, mut y) = if first.0 { first.1 } else { fallback.unwrap_or((first.1, order[0])).0 };
    let used = if first.0 { order[0] } else { fallback.map(|(_, p)| p).unwrap_or(order[0]) };
    if !fits {
        // Nothing fit: keep the (clamped) flyout inside the container.
        x = x.clamp(container.left, container.right - width);
        y = y.clamp(container.top, container.bottom - height);
    }
    (rect(x, y, x + width, y + height), used)
}

/// `TryPlacement`: position on `placement`'s side, centered on the cross axis.
fn try_placement(target: D2D_RECT_F, width: f32, height: f32, container: D2D_RECT_F, placement: Placement) -> (bool, (f32, f32)) {
    if placement.vertical() {
        let (fits_y, y) = place_along(target.top, target.bottom, placement == Placement::Bottom, height, container.top, container.bottom);
        let (fits_x, x) = center_within(target.left, target.right, width, container.left, container.right);
        (fits_y && fits_x, (x, y))
    } else {
        let (fits_x, x) = place_along(target.left, target.right, placement == Placement::Right, width, container.left, container.right);
        let (fits_y, y) = center_within(target.top, target.bottom, height, container.top, container.bottom);
        (fits_x && fits_y, (x, y))
    }
}

/// `TestAgainstLimitsAndPlace`: before or after the anchor, clamped into the limits.
/// The flyout margin is part of the space it needs on that side.
fn place_along(start: f32, end: f32, increasing: bool, size: f32, low: f32, high: f32) -> (bool, f32) {
    let need = size + MARGIN;
    let (extra, pos) = if increasing {
        (high - end - need, end + MARGIN)
    } else {
        (start - low - need, start - MARGIN - size)
    };
    let pos = if pos < low { low } else if pos + size > high { high - size } else { pos };
    (extra >= 0.0, pos)
}

/// `TestAndCenterAlignWithinLimits` (Center justification). A flyout that
/// exactly fills the axis fits, the same rule `place_along` uses.
fn center_within(start: f32, end: f32, size: f32, low: f32, high: f32) -> (bool, f32) {
    if high - low < size {
        return (false, low);
    }
    let pos = start + 0.5 * ((end - start) - size);
    (true, if pos < low { low } else if pos + size > high { high - size } else { pos })
}

/// The open/close animation state of a flyout.
#[derive(Clone, Copy, Debug)]
pub struct Transition {
    start: Instant,
    opening: bool,
    /// Unit vector of the open slide's starting offset (toward/over the target).
    from: (f32, f32),
    /// Closing: opacity and offset the fade starts from.
    start_opacity: f32,
    at: (f32, f32),
}

/// One frame of a [`Transition`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Offset of the window from its final position, in DIPs.
    pub offset: (f32, f32),
    /// Content opacity.
    pub opacity: f32,
    /// False once the transition is over.
    pub running: bool,
}

/// `TAS_SHOWPOPUP` opacity: linear 0 -> 1, 83 ms, after an 83 ms delay.
const SHOW_FADE: (Duration, Duration) = (Duration::from_millis(83), Duration::from_millis(83));
/// `TAS_SHOWPOPUP` translation: 367 ms, `cubic-bezier(0.1, 0.9, 0.2, 1)`.
const SHOW_SLIDE: (Duration, CubicBezier) = (easing::REPOSITION_DURATION, easing::REPOSITION);
/// `TAS_HIDEPOPUP` opacity: linear 1 -> 0, 83 ms.
const HIDE_FADE: Duration = Duration::from_millis(83);

impl Transition {
    /// Opening at `placement`: the flyout starts 50 DIPs toward (over) the target and
    /// slides away from it into place.
    pub fn open(placement: Placement) -> Self {
        // PopupThemeTransition offsets (FlyoutBase): Top starts lower, Bottom higher, etc.
        let from = match placement {
            Placement::Top => (0.0, 1.0),
            Placement::Bottom => (0.0, -1.0),
            Placement::Left => (1.0, 0.0),
            Placement::Right => (-1.0, 0.0),
        };
        Self { start: Instant::now(), opening: true, from, start_opacity: 0.0, at: (0.0, 0.0) }
    }

    /// Opening with no slide, for a `ToolTip` (`FadeInThemeAnimation`).
    pub fn fade() -> Self {
        Self { start: Instant::now(), opening: true, from: (0.0, 0.0), start_opacity: 0.0, at: (0.0, 0.0) }
    }

    /// Closing: a plain fade out.
    pub fn close() -> Self {
        Self::close_from(Frame { offset: (0.0, 0.0), opacity: 1.0, running: false })
    }

    /// Closing from `frame` (e.g. mid-open): stays at its offset, fades from its opacity.
    pub fn close_from(frame: Frame) -> Self {
        Self { start: Instant::now(), opening: false, from: (0.0, 0.0), start_opacity: frame.opacity, at: frame.offset }
    }

    /// The frame `elapsed` after the start.
    pub fn at(&self, elapsed: Duration) -> Frame {
        let ratio = |t: Duration, d: Duration| (t.as_secs_f32() / d.as_secs_f32()).clamp(0.0, 1.0);
        if !self.opening {
            let k = ratio(elapsed, HIDE_FADE);
            return Frame { offset: self.at, opacity: self.start_opacity * (1.0 - k), running: elapsed < HIDE_FADE };
        }
        let (delay, fade) = SHOW_FADE;
        let opacity = ratio(elapsed.saturating_sub(delay), fade);
        let (slide, curve) = SHOW_SLIDE;
        let left = ENTRANCE_OFFSET * (1.0 - curve.ease(ratio(elapsed, slide)));
        let offset = (self.from.0 * left, self.from.1 * left);
        let end = if self.from == (0.0, 0.0) { delay + fade } else { slide.max(delay + fade) };
        Frame { offset, opacity, running: elapsed < end }
    }

    /// The current frame.
    pub fn now(&self) -> Frame {
        self.at(self.start.elapsed())
    }
}

/// `FlyoutPresenter` brushes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyoutStyle {
    /// Solid fill drawn over the backdrop: `None` when the acrylic backdrop shows
    /// (`AcrylicInAppFillColorDefault`), the acrylic fallback color without it,
    /// `SystemColorWindowColor` in high contrast.
    pub background: Option<D2D1_COLOR_F>,
    /// `FlyoutBorderThemeBrush`: `SurfaceStrokeColorFlyout`, WindowText in high contrast.
    pub border: D2D1_COLOR_F,
    /// `FlyoutBorderThemeThickness`: 1, or 2 in high contrast.
    pub border_width: f32,
}

/// Brushes for `t`; `backdrop` is whether the DWM acrylic backdrop is active.
pub fn style(t: &Theme, backdrop: bool) -> FlyoutStyle {
    if let Some(c) = t.high_contrast {
        return FlyoutStyle { background: Some(c.window), border: c.window_text, border_width: 2.0 };
    }
    // AcrylicInAppFillColorDefault FallbackColor; SurfaceStrokeColorFlyout.
    let (fallback, border) = if t.dark { (0xFF2C2C2C, 0x33000000) } else { (0xFFF9F9F9, 0x0F000000) };
    FlyoutStyle { background: (!backdrop).then(|| theme::argb(fallback)), border: theme::argb(border), border_width: 1.0 }
}

/// Draws the presenter chrome over the whole `size` (DIPs): optional fill and
/// the border, with `OverlayCornerRadius`.
pub fn draw_chrome(s: &Surface, style: &FlyoutStyle, size: (f32, f32)) {
    draw_chrome_r(s, style, size, theme::OVERLAY_RADIUS);
}

/// Same chrome with a different radius (ToolTip uses `ControlCornerRadius`).
pub fn draw_chrome_r(s: &Surface, style: &FlyoutStyle, size: (f32, f32), radius: f32) {
    let r = rect(0.0, 0.0, size.0, size.1);
    if let Some(bg) = style.background {
        s.fill_round(r, radius, bg);
    }
    s.stroke_round(r, radius, style.border_width, style.border);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tests::HC;

    const SCREEN: D2D_RECT_F = D2D_RECT_F { left: 0.0, top: 0.0, right: 1000.0, bottom: 800.0 };

    fn target(x: f32, y: f32) -> D2D_RECT_F {
        rect(x, y, x + 100.0, y + 32.0)
    }

    #[test]
    fn fade_should_have_no_offset_and_stop_after_the_opacity_fade() {
        let t = Transition::fade();
        assert_eq!(t.at(Duration::ZERO).offset, (0.0, 0.0));
        assert_eq!(t.at(Duration::from_millis(100)).offset, (0.0, 0.0));
        assert!(!t.at(Duration::from_millis(166)).running);
        assert!(!t.at(Duration::from_millis(200)).running);
    }

    #[test]
    fn top_placement_should_center_above_with_the_margin() {
        let (r, p) = place(target(450.0, 400.0), 200.0, 100.0, SCREEN, Placement::Top);
        assert_eq!((r.left, r.top, r.right, r.bottom, p), (400.0, 296.0, 600.0, 396.0, Placement::Top));
    }

    #[test]
    fn top_without_room_should_fall_back_to_bottom() {
        let (r, p) = place(target(450.0, 50.0), 200.0, 100.0, SCREEN, Placement::Top);
        assert_eq!((r.top, p), (86.0, Placement::Bottom));
    }

    #[test]
    fn centered_flyout_should_be_clamped_inside_the_container() {
        let (r, _) = place(target(950.0, 400.0), 200.0, 100.0, SCREEN, Placement::Top);
        assert_eq!((r.left, r.right), (800.0, 1000.0));
    }

    #[test]
    fn right_placement_should_sit_beside_with_the_margin() {
        let (r, p) = place(target(100.0, 400.0), 200.0, 100.0, SCREEN, Placement::Right);
        assert_eq!((r.left, r.top, p), (204.0, 366.0, Placement::Right));
    }

    #[test]
    fn the_margin_should_count_when_testing_whether_the_flyout_fits() {
        // Room for the flyout but not for the gap: must fall back, not overflow.
        let (r, p) = place(target(400.0, 670.0), 200.0, 96.0, SCREEN, Placement::Bottom);
        assert_eq!(p, Placement::Top);
        assert!(r.bottom <= SCREEN.bottom, "{r:?}");
    }

    #[test]
    fn nothing_fitting_should_keep_the_first_choice() {
        let (_, p) = place(target(450.0, 400.0), 2000.0, 2000.0, SCREEN, Placement::Bottom);
        assert_eq!(p, Placement::Bottom);
    }

    #[test]
    fn opening_above_should_start_50_dips_lower_and_invisible() {
        let f = Transition::open(Placement::Top).at(Duration::ZERO);
        assert_eq!((f.offset, f.opacity, f.running), ((0.0, ENTRANCE_OFFSET), 0.0, true));
    }

    #[test]
    fn opening_below_should_start_50_dips_higher() {
        assert_eq!(Transition::open(Placement::Bottom).at(Duration::ZERO).offset, (0.0, -ENTRANCE_OFFSET));
    }

    #[test]
    fn opacity_should_wait_83_ms_then_fade_in_over_83_ms() {
        let t = Transition::open(Placement::Top);
        let at = |ms| t.at(Duration::from_millis(ms)).opacity;
        assert_eq!((at(80), (at(124) * 100.0).round(), at(170)), (0.0, 49.0, 1.0));
    }

    #[test]
    fn slide_should_end_at_367_ms() {
        let t = Transition::open(Placement::Left);
        let (mid, end) = (t.at(Duration::from_millis(100)), t.at(Duration::from_millis(367)));
        assert!(mid.offset.0 > 0.0 && mid.offset.0 < ENTRANCE_OFFSET);
        assert_eq!((end.offset, end.running), ((0.0, 0.0), false));
    }

    #[test]
    fn closing_should_fade_out_in_83_ms() {
        let t = Transition::close();
        let (start, end) = (t.at(Duration::ZERO), t.at(Duration::from_millis(83)));
        assert_eq!((start.opacity, end.opacity, end.running), (1.0, 0.0, false));
    }

    #[test]
    fn closing_mid_open_should_start_from_the_current_frame() {
        let mid = Frame { offset: (0.0, 20.0), opacity: 0.4, running: true };
        let t = Transition::close_from(mid);
        let (start, half) = (t.at(Duration::ZERO), t.at(Duration::from_millis(41)));
        assert_eq!((start.offset, start.opacity), ((0.0, 20.0), 0.4));
        assert!(half.opacity < 0.4 && half.offset == (0.0, 20.0));
    }

    #[test]
    fn a_flyout_bigger_than_the_container_should_shrink_to_fit() {
        let (r, _) = place(target(450.0, 400.0), 300.0, 2000.0, SCREEN, Placement::Bottom);
        assert!(r.top >= SCREEN.top && r.bottom <= SCREEN.bottom, "{r:?}");
    }

    #[test]
    fn backdrop_style_should_draw_no_fill_and_a_1_dip_flyout_stroke() {
        let s = style(&Theme::DARK, true);
        assert_eq!((s.background, s.border, s.border_width), (None, theme::argb(0x33000000), 1.0));
    }

    #[test]
    fn without_backdrop_the_acrylic_fallback_color_should_fill() {
        assert_eq!(style(&Theme::LIGHT, false).background, Some(theme::argb(0xFFF9F9F9)));
    }

    #[test]
    fn high_contrast_should_use_window_and_a_2_dip_window_text_border() {
        let s = style(&Theme::high_contrast(HC), true);
        assert_eq!((s.background, s.border, s.border_width), (Some(HC.window), HC.window_text, 2.0));
    }
}
