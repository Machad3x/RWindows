//! WinUI `Slider` (`DefaultSliderStyle`, horizontal) for raw Win32.
//!
//! Geometry and brushes: `Slider_themeresources.xaml`; input: `Slider_Partial.cpp`
//! (`OnPointerPressed`, `MoveThumbToPoint`) and `SliderKeyProcess.h`.

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::core::Result;

use crate::button::{FocusKind, Input, Key, Response, Visual};
use crate::easing::{self, Tween};
use crate::gfx::{Edge, Surface, rect};
use crate::theme::{self, Theme};

/// `SliderPreContentMargin` / `SliderPostContentMargin`.
const PRE: f32 = 14.0;
/// `SliderHorizontalHeight` (MinHeight).
pub const HEIGHT: f32 = 32.0;
/// `SliderHorizontalThumbWidth` / `Height`.
const THUMB: f32 = 18.0;
/// Thumb template `Border Margin="-2"`: the visible ring is 22 DIPs.
const THUMB_OUTSET: f32 = 2.0;
/// `SliderThumbCornerRadius`.
const THUMB_RADIUS: f32 = 10.0;
/// `SliderInnerThumbWidth` / `Height`.
const INNER: f32 = 12.0;
/// `SliderTrackCornerRadius`.
const TRACK_RADIUS: f32 = 2.0;
/// `FocusVisualMargin="-7,0,-7,0"`.
const FOCUS_MARGIN_X: f32 = 7.0;

/// Inner thumb `CompositeTransform` scale per Thumb visual state.
fn inner_scale(visual: Visual) -> f32 {
    match visual {
        Visual::Rest => 0.86,
        Visual::PointerOver | Visual::Disabled => 1.167,
        Visual::Pressed => 0.71,
    }
}

/// Brushes of one Slider visual state (`Slider*` resources).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SliderStyle {
    /// `SliderTrackFill*` (`HorizontalTrackRect`).
    pub track: D2D1_COLOR_F,
    /// `SliderTrackValueFill*` (`HorizontalDecreaseRect`).
    pub value: D2D1_COLOR_F,
    /// `SliderThumbBackground*` (inner ellipse).
    pub thumb: D2D1_COLOR_F,
    /// `SliderOuterThumbBackground` (22-DIP ring fill).
    pub thumb_ring: D2D1_COLOR_F,
    /// `SliderThumbBorderBrush`: solid in high contrast, else `None` for the
    /// `ControlElevationBorderBrush` gradient.
    pub thumb_border: Option<D2D1_COLOR_F>,
}

/// A horizontal slider from `minimum` to `maximum`.
#[derive(Clone, Debug)]
pub struct Slider {
    /// Bounds in DIPs (height [`HEIGHT`]).
    pub rect: D2D_RECT_F,
    /// UI Automation name (no header is drawn).
    pub name: String,
    value: f64,
    minimum: f64,
    maximum: f64,
    /// `StepFrequency` (and `SmallChange`): 1 by default.
    step: f64,
    enabled: bool,
    pointer_over: bool,
    /// `m_isPressed`: a press on the track (the thumb follows the pointer).
    pressed: bool,
    /// Drag started on the thumb: pointer x and value at press.
    thumb_drag: Option<(f32, f64)>,
    /// Pointer over the thumb (its own PointerOver state).
    thumb_over: bool,
    /// `IntermediateValue` while pressing or dragging: unsnapped, it places the
    /// thumb under the pointer while `value` follows the steps.
    intermediate: Option<f64>,
    focus: Option<FocusKind>,
    scale: Tween,
}

impl Slider {
    /// Enabled slider at `rect`, `value` clamped to `minimum..=maximum`, step 1.
    pub fn new(name: &str, rect: D2D_RECT_F, minimum: f64, maximum: f64, value: f64) -> Self {
        Self {
            rect,
            name: name.to_owned(),
            value: value.clamp(minimum, maximum),
            minimum,
            maximum,
            step: 1.0,
            enabled: true,
            pointer_over: false,
            pressed: false,
            thumb_drag: None,
            thumb_over: false,
            intermediate: None,
            focus: None,
            scale: Tween::idle(inner_scale(Visual::Rest)),
        }
    }

    /// `Value`.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// `Minimum`, `Maximum`, `SmallChange` (= `StepFrequency`).
    pub fn range(&self) -> (f64, f64, f64) {
        (self.minimum, self.maximum, self.step)
    }

    /// Slider visual state (`CommonStates`): Pressed while the pointer is down.
    pub fn visual(&self) -> Visual {
        if !self.enabled {
            Visual::Disabled
        } else if self.pressed || self.thumb_drag.is_some() {
            Visual::Pressed
        } else if self.pointer_over {
            Visual::PointerOver
        } else {
            Visual::Rest
        }
    }

    /// The Thumb's own visual state (drives the inner ellipse scale).
    fn thumb_visual(&self) -> Visual {
        if !self.enabled {
            Visual::Disabled
        } else if self.thumb_drag.is_some() {
            Visual::Pressed
        } else if self.thumb_over {
            Visual::PointerOver
        } else {
            Visual::Rest
        }
    }

    /// True while the slider holds the mouse capture.
    pub fn is_captured(&self) -> bool {
        self.pressed || self.thumb_drag.is_some()
    }

    /// True when the focus rectangle must be drawn (keyboard focus only).
    pub fn focus_visible(&self) -> bool {
        self.enabled && self.focus == Some(FocusKind::Keyboard)
    }

    /// Value as a 0..1 fraction of the range.
    fn fraction(&self) -> f64 {
        let range = self.maximum - self.minimum;
        let v = self.intermediate.unwrap_or(self.value);
        if range <= 0.0 { 0.0 } else { (v - self.minimum) / range }
    }

    /// `HorizontalThumb` bounds (18 x 18): left = fraction x (width - 18).
    pub fn thumb(&self) -> D2D_RECT_F {
        let r = self.rect;
        let left = r.left + (self.fraction() as f32) * self.travel();
        // Centered on the 4-DIP track row (the thumb spans all three rows).
        let cy = r.top + PRE + 2.0;
        rect(left, cy - THUMB / 2.0, left + THUMB, cy + THUMB / 2.0)
    }

    /// Horizontal distance the thumb can move (`trackClickableRegionLength`).
    fn travel(&self) -> f32 {
        (self.rect.right - self.rect.left - THUMB).max(1.0)
    }

    /// `Slider::GetClosestStep`: the nearest multiple of the step counted from 0
    /// (not from Minimum), bounded by the range; a tie keeps the lower step.
    fn snap(&self, value: f64) -> f64 {
        let n = value / self.step;
        let next = self.maximum.min(n.ceil() * self.step);
        let prev = self.minimum.max(n.floor() * self.step);
        if next - value < value - prev { next } else { prev }
    }

    /// `MoveThumbToPoint`'s `IntermediateValue`: the thumb center under the pointer.
    fn value_at(&self, x: f32) -> f64 {
        let fraction = f64::from((x - self.rect.left - THUMB / 2.0) / self.travel()).clamp(0.0, 1.0);
        self.minimum + fraction * (self.maximum - self.minimum)
    }

    /// Pointer input moved the thumb to `raw`: draw it there, step the value.
    fn track(&mut self, raw: f64) {
        let raw = raw.clamp(self.minimum, self.maximum);
        self.intermediate = Some(raw);
        self.value = self.snap(raw);
    }

    /// Sets `Value` clamped to the range, without step snapping (`RangeBase::put_Value`);
    /// UIA `SetValue`. Ignored while disabled.
    pub fn set_value(&mut self, value: f64) -> Response {
        if !self.enabled {
            return Response::default();
        }
        let before = self.snapshot();
        self.value = value.clamp(self.minimum, self.maximum);
        self.changed(before)
    }

    /// Enables or disables the slider.
    pub fn set_enabled(&mut self, enabled: bool) -> Response {
        let before = self.snapshot();
        let capture = (self.is_captured() && !enabled).then_some(false);
        self.enabled = enabled;
        self.pointer_over = false;
        self.thumb_over = false;
        let lost_focus = !enabled && self.focus.take().is_some();
        if !enabled {
            self.end_press();
        }
        Response { capture, lost_focus, ..self.changed(before) }
    }

    /// Feeds one input event. `Response::redraw` covers value changes; read
    /// [`Slider::value`] to see the new value.
    pub fn handle(&mut self, input: Input) -> Response {
        let before = self.snapshot();
        let mut r = Response::default();
        match input {
            Input::PointerMove(x, y) => {
                self.pointer_over = contains(self.rect, x, y);
                self.thumb_over = contains(self.thumb(), x, y);
                if let Some((px, v0)) = self.thumb_drag {
                    // Thumb DragDelta: the value moves with the pointer, relative to the press.
                    let range = self.maximum - self.minimum;
                    self.track(v0 + f64::from((x - px) / self.travel()) * range);
                } else if self.pressed {
                    self.track(self.value_at(x));
                }
            }
            Input::PointerDown(x, y) if self.enabled && contains(self.rect, x, y) => {
                self.focus = Some(FocusKind::Pointer);
                r.capture = Some(true);
                r.take_focus = true;
                if contains(self.thumb(), x, y) {
                    // The Thumb handles its own press: drag without a jump.
                    self.thumb_drag = Some((x, self.value));
                } else {
                    self.pressed = true;
                    self.track(self.value_at(x));
                }
                self.thumb_over = contains(self.thumb(), x, y);
            }
            Input::PointerUp(x, y) if self.is_captured() => {
                // PerformPointerUpAction / OnThumbDragCompleted: the thumb snaps to Value.
                self.end_press();
                self.pointer_over = contains(self.rect, x, y);
                self.thumb_over = contains(self.thumb(), x, y);
                r.capture = Some(false);
            }
            Input::PointerLeave if !self.is_captured() => {
                self.pointer_over = false;
                self.thumb_over = false;
            }
            Input::KeyDown(key) if self.enabled && self.focus.is_some() => {
                let target = match key {
                    Key::Left | Key::Down => Some(self.value - self.step),
                    Key::Right | Key::Up => Some(self.value + self.step),
                    Key::Home => Some(self.minimum),
                    Key::End => Some(self.maximum),
                    _ => None,
                };
                // RangeBase Step -> put_Value: coerced into the range, not step-snapped.
                if let Some(v) = target {
                    self.value = v.clamp(self.minimum, self.maximum);
                }
            }
            Input::Focus(kind) => self.focus = Some(kind),
            Input::Blur => {
                self.focus = None;
                // shortcut: losing focus also ends a pointer press (keeps Panel's Tab-while-pressed rule); XAML keeps the capture.
                if self.is_captured() {
                    self.end_press();
                    r.capture = Some(false);
                }
            }
            Input::CaptureLost => self.end_press(),
            _ => {}
        }
        Response { capture: r.capture, take_focus: r.take_focus, ..self.changed(before) }
    }

    /// Ends a track press or thumb drag; the thumb returns to the stepped value.
    fn end_press(&mut self) {
        self.pressed = false;
        self.thumb_drag = None;
        self.intermediate = None;
    }

    fn snapshot(&self) -> (Visual, Visual, bool, f64, Option<f64>) {
        (self.visual(), self.thumb_visual(), self.focus_visible(), self.value, self.intermediate)
    }

    fn changed(&mut self, before: (Visual, Visual, bool, f64, Option<f64>)) -> Response {
        let thumb = self.thumb_visual();
        if thumb != before.1 {
            // Inner thumb ScaleX/Y: 0,0,0,1 spline, 167 ms into Normal/Disabled, 250 ms otherwise.
            let duration = match thumb {
                Visual::Rest | Visual::Disabled => easing::FAST,
                Visual::PointerOver | Visual::Pressed => easing::NORMAL,
            };
            self.scale.retarget(inner_scale(thumb), duration, easing::FAST_OUT_SLOW_IN);
        }
        Response { redraw: self.snapshot() != before, ..Response::default() }
    }

    /// True while the inner thumb scale animates.
    pub fn animating(&self) -> bool {
        self.scale.running()
    }

    /// Target brushes for the current visual state.
    pub fn style(&self, t: &Theme) -> SliderStyle {
        style_of(self.visual(), t)
    }

    /// Track height: `SliderTrackThemeHeight` (2 in high contrast, else 4).
    fn track_height(t: &Theme) -> f32 {
        if t.high_contrast.is_some() { 2.0 } else { 4.0 }
    }

    /// Draws track, value fill, thumb and the focus rectangle.
    pub fn draw(&self, s: &Surface, t: &Theme) -> Result<()> {
        let style = self.style(t);
        let r = self.rect;
        let h = Self::track_height(t);
        // Centered on the thumb line (high contrast's 2-DIP track sits 1 DIP lower than in XAML).
        let top = r.top + PRE + (4.0 - h) / 2.0;
        s.fill_round(rect(r.left, top, r.right, top + h), TRACK_RADIUS, style.track);
        let thumb = self.thumb();
        if thumb.left > r.left {
            s.fill_round(rect(r.left, top, thumb.left, top + h), TRACK_RADIUS.min((thumb.left - r.left) / 2.0), style.value);
        }
        let ring = rect(thumb.left - THUMB_OUTSET, thumb.top - THUMB_OUTSET, thumb.right + THUMB_OUTSET, thumb.bottom + THUMB_OUTSET);
        s.fill_round(ring, THUMB_RADIUS, style.thumb_ring);
        match style.thumb_border {
            Some(color) => s.stroke_round(ring, THUMB_RADIUS, 1.0, color),
            None => {
                let edge = if t.dark { Edge::Top } else { Edge::Bottom };
                s.stroke_elevation(ring, THUMB_RADIUS, edge, t.stroke_secondary, t.stroke)?;
            }
        }
        let d = INNER * self.scale.value();
        let (cx, cy) = ((thumb.left + thumb.right) / 2.0, (thumb.top + thumb.bottom) / 2.0);
        s.fill_round(rect(cx - d / 2.0, cy - d / 2.0, cx + d / 2.0, cy + d / 2.0), d / 2.0, style.thumb);
        if self.focus_visible() {
            let outer = rect(r.left - FOCUS_MARGIN_X, r.top, r.right + FOCUS_MARGIN_X, r.bottom);
            let rr = theme::CONTROL_RADIUS;
            s.stroke_round(outer, rr + 3.0, 2.0, t.focus_outer);
            let ring = s.snap_len(2.0);
            s.stroke_round(rect(outer.left + ring, outer.top + ring, outer.right - ring, outer.bottom - ring), rr + 1.0, 1.0, t.focus_inner);
        }
        Ok(())
    }
}

fn contains(r: D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// `Slider*` brushes for the Slider's CommonStates (they also set the thumb fill).
fn style_of(visual: Visual, t: &Theme) -> SliderStyle {
    if let Some(c) = t.high_contrast {
        let (track, value, thumb) = match visual {
            Visual::Rest => (c.button_text, c.highlight, c.button_text),
            Visual::PointerOver => (c.button_text, c.highlight, c.highlight),
            Visual::Pressed => (c.button_text, c.highlight, c.hotlight),
            Visual::Disabled => (c.gray_text, c.gray_text, c.gray_text),
        };
        return SliderStyle { track, value, thumb, thumb_ring: c.button_text, thumb_border: Some(c.button_text) };
    }
    let (track, accent) = match visual {
        Visual::Rest => (t.control_strong_fill, t.accent),
        Visual::PointerOver => (t.control_strong_fill, theme::with_opacity(t.accent, 0.9)),
        Visual::Pressed => (t.control_strong_fill, theme::with_opacity(t.accent, 0.8)),
        Visual::Disabled => (t.control_strong_fill_disabled, t.accent_disabled),
    };
    SliderStyle { track, value: accent, thumb: accent, thumb_ring: t.control_solid_fill, thumb_border: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tests::HC;

    /// 0..100 slider, 218 DIPs wide (200 DIPs of thumb travel), at value 50.
    fn slider(value: f64) -> Slider {
        Slider::new("Volume", rect(0.0, 0.0, 218.0, HEIGHT), 0.0, 100.0, value)
    }

    fn focused(value: f64) -> Slider {
        let mut s = slider(value);
        s.handle(Input::Focus(FocusKind::Keyboard));
        s
    }

    #[test]
    fn thumb_should_sit_at_the_value_fraction_of_the_travel() {
        let t = slider(25.0).thumb();
        assert_eq!((t.left, t.top, t.right, t.bottom), (50.0, 7.0, 68.0, 25.0));
    }

    #[test]
    fn press_on_the_track_should_move_the_thumb_to_the_pointer_and_capture() {
        let mut s = slider(0.0);
        // Pointer at x = 9 + 0.75 * 200 = 159 -> 75.
        let r = s.handle(Input::PointerDown(159.0, 16.0));
        assert_eq!((s.value(), r.capture, r.take_focus, s.visual()), (75.0, Some(true), true, Visual::Pressed));
    }

    #[test]
    fn track_press_should_snap_to_the_step() {
        let mut s = slider(0.0);
        s.handle(Input::PointerDown(9.0 + 0.333 * 200.0, 16.0));
        assert_eq!(s.value(), 33.0);
    }

    #[test]
    fn a_tie_between_two_steps_should_go_to_the_lower_one() {
        let mut s = slider(0.0);
        // 1 DIP into the 200-DIP travel = 0.5: GetClosestStep keeps the previous step.
        s.handle(Input::PointerDown(10.0, 16.0));
        assert_eq!(s.value(), 0.0);
    }

    #[test]
    fn steps_should_count_from_zero_not_from_minimum() {
        let mut s = Slider::new("x", rect(0.0, 0.0, 218.0, HEIGHT), 0.5, 10.5, 0.5);
        // 19% of 0.5..10.5 = 2.4 -> nearest multiple of 1 from 0 is 2 (a grid from
        // Minimum would give 2.5).
        s.handle(Input::PointerDown(9.0 + 0.19 * 200.0, 16.0));
        assert_eq!(s.value(), 2.0);
    }

    #[test]
    fn programmatic_value_should_not_snap() {
        let mut s = slider(0.0);
        s.set_value(62.5);
        assert_eq!(s.value(), 62.5);
    }

    #[test]
    fn thumb_should_stay_under_the_pointer_during_a_press_and_snap_on_release() {
        let mut s = Slider::new("x", rect(0.0, 0.0, 218.0, HEIGHT), 0.0, 10.0, 0.0);
        s.handle(Input::PointerDown(9.0 + 0.33 * 200.0, 16.0));
        let during = (s.value(), s.thumb().left.round());
        s.handle(Input::PointerUp(9.0 + 0.33 * 200.0, 16.0));
        assert_eq!((during, s.thumb().left.round()), ((3.0, 66.0), 60.0));
    }

    #[test]
    fn moving_after_a_track_press_should_follow_the_pointer() {
        let mut s = slider(0.0);
        s.handle(Input::PointerDown(109.0, 16.0));
        s.handle(Input::PointerMove(500.0, 16.0));
        assert_eq!(s.value(), 100.0);
    }

    #[test]
    fn dragging_the_thumb_should_move_by_the_pointer_delta() {
        let mut s = slider(50.0);
        // Thumb spans 100..118; press at its right part, move 20 DIPs = 10 units.
        s.handle(Input::PointerDown(115.0, 16.0));
        let pressed_on_thumb = s.value();
        s.handle(Input::PointerMove(135.0, 16.0));
        assert_eq!((pressed_on_thumb, s.value()), (50.0, 60.0));
    }

    #[test]
    fn release_should_end_the_press_and_free_the_capture() {
        let mut s = slider(0.0);
        s.handle(Input::PointerDown(109.0, 16.0));
        let r = s.handle(Input::PointerUp(109.0, 16.0));
        assert_eq!((r.capture, s.is_captured()), (Some(false), false));
    }

    #[test]
    fn capture_lost_should_end_the_press_keeping_the_value() {
        let mut s = slider(0.0);
        s.handle(Input::PointerDown(109.0, 16.0));
        s.handle(Input::CaptureLost);
        assert_eq!((s.is_captured(), s.value()), (false, 50.0));
    }

    #[test]
    fn arrows_should_step_by_one_and_clamp() {
        let mut s = focused(99.0);
        s.handle(Input::KeyDown(Key::Right));
        s.handle(Input::KeyDown(Key::Up));
        let top = s.value();
        s.handle(Input::KeyDown(Key::Left));
        s.handle(Input::KeyDown(Key::Down));
        assert_eq!((top, s.value()), (100.0, 98.0));
    }

    #[test]
    fn home_and_end_should_jump_to_the_ends() {
        let mut s = focused(40.0);
        s.handle(Input::KeyDown(Key::End));
        let end = s.value();
        s.handle(Input::KeyDown(Key::Home));
        assert_eq!((end, s.value()), (100.0, 0.0));
    }

    #[test]
    fn keys_should_be_ignored_without_focus_or_while_disabled() {
        let mut a = slider(40.0);
        a.handle(Input::KeyDown(Key::Right));
        let mut b = focused(40.0);
        b.set_enabled(false);
        b.handle(Input::KeyDown(Key::Right));
        assert_eq!((a.value(), b.value()), (40.0, 40.0));
    }

    #[test]
    fn value_change_should_request_a_redraw() {
        let mut s = focused(40.0);
        assert!(s.handle(Input::KeyDown(Key::Right)).redraw);
    }

    #[test]
    fn set_value_should_snap_and_clamp_and_ignore_when_disabled() {
        let mut s = slider(0.0);
        s.set_value(12.4);
        let kept = s.value();
        s.set_value(500.0);
        let clamped = s.value();
        s.set_enabled(false);
        s.set_value(3.0);
        assert_eq!((kept, clamped, s.value()), (12.4, 100.0, 100.0));
    }

    #[test]
    fn hovering_the_thumb_should_grow_the_inner_ellipse() {
        let mut s = slider(50.0);
        s.handle(Input::PointerMove(109.0, 16.0));
        assert_eq!(s.scale.target(), 1.167);
    }

    #[test]
    fn rest_style_should_use_strong_fill_track_and_accent_value() {
        let (st, t) = (slider(50.0).style(&Theme::DARK), Theme::DARK);
        assert_eq!((st.track, st.value, st.thumb, st.thumb_ring, st.thumb_border),
            (t.control_strong_fill, t.accent, t.accent, t.control_solid_fill, None));
    }

    #[test]
    fn pointer_over_style_should_use_accent_secondary() {
        let mut s = slider(50.0);
        s.handle(Input::PointerMove(10.0, 16.0));
        let (st, t) = (s.style(&Theme::LIGHT), Theme::LIGHT);
        let accent = theme::with_opacity(t.accent, 0.9);
        assert_eq!((st.value, st.thumb), (accent, accent));
    }

    #[test]
    fn disabled_style_should_use_disabled_tokens() {
        let mut s = slider(50.0);
        s.set_enabled(false);
        let (st, t) = (s.style(&Theme::DARK), Theme::DARK);
        assert_eq!((st.track, st.value, st.thumb), (t.control_strong_fill_disabled, t.accent_disabled, t.accent_disabled));
    }

    #[test]
    fn high_contrast_pressed_thumb_should_use_hotlight() {
        let mut s = slider(50.0);
        s.handle(Input::PointerDown(109.0, 16.0));
        let st = s.style(&Theme::high_contrast(HC));
        assert_eq!((st.track, st.value, st.thumb, st.thumb_ring, st.thumb_border),
            (HC.button_text, HC.highlight, HC.hotlight, HC.button_text, Some(HC.button_text)));
    }
}
