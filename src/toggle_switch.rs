//! WinUI `ToggleSwitch` (`DefaultToggleSwitchStyle`) for raw Win32.
//!
//! Geometry, brushes and animations follow `ToggleSwitch_themeresources.xaml`;
//! input follows `ToggleSwitch_Partial.cpp` and `ToggleSwitchKeyProcess.h`.
//! Shares [`Input`], [`Response`] and [`Visual`] with [`crate::button`].

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::DirectWrite::{DWRITE_PARAGRAPH_ALIGNMENT_CENTER, IDWriteTextFormat};
use windows::core::Result;

use crate::button::{FocusKind, Input, Key, Response, Visual};
use crate::easing::{self, Tween};
use crate::gfx::{Surface, rect};
use crate::theme::{self, Theme};

/// `OuterBorder`: 40 x 20, radius 10.
const TRACK_W: f32 = 40.0;
const TRACK_H: f32 = 20.0;
/// `KnobTranslateTransform.X` in the On state.
const TRAVEL: f32 = 20.0;
/// `ToggleSwitchPreContentMargin` / `ToggleSwitchPostContentMargin`.
const PRE: f32 = 10.0;
/// Height of the switch row: pre margin + track + post margin.
const ROW: f32 = PRE + TRACK_H + PRE;
/// `SwitchAreaGrid` `Margin="0,5"`: the focus target inside the row.
const AREA_INSET: f32 = 5.0;
/// Column between the track and the On/Off content.
const GAP: f32 = 12.0;
/// `ToggleSwitchThemeMinWidth`.
const MIN_WIDTH: f32 = 154.0;
/// `ToggleSwitchTopHeaderMargin` bottom.
const HEADER_GAP: f32 = 4.0;
/// `FocusVisualMargin="-7,-3,-7,-3"`.
const FOCUS_MARGIN: (f32, f32) = (7.0, 3.0);
/// Movement before a press becomes a drag.
/// shortcut: fixed 4 DIPs (the Windows default drag threshold, SM_CXDRAG = 4 px at 100%); read the system metric if it matters.
const DRAG_THRESHOLD: f32 = 4.0;

/// Knob size (width, height) per visual state.
fn knob_size(visual: Visual) -> (f32, f32) {
    match visual {
        Visual::Rest | Visual::Disabled => (12.0, 12.0),
        Visual::PointerOver => (14.0, 14.0),
        Visual::Pressed => (17.0, 14.0),
    }
}

/// Brushes of one state (`ToggleSwitch*` resources).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SwitchStyle {
    /// `ToggleSwitchFillOff*` (`OuterBorder` fill).
    pub fill_off: D2D1_COLOR_F,
    /// `ToggleSwitchStrokeOff*` (`OuterBorder` 1-DIP stroke).
    pub stroke_off: D2D1_COLOR_F,
    /// `ToggleSwitchKnobFillOff*`.
    pub knob_off: D2D1_COLOR_F,
    /// `ToggleSwitchFillOn*` (`SwitchKnobBounds`).
    pub fill_on: D2D1_COLOR_F,
    /// `ToggleSwitchStrokeOn*` when `ToggleSwitchOnStrokeThickness` is 1 (high
    /// contrast); `None` where it is 0 (Default, Light).
    pub stroke_on: Option<D2D1_COLOR_F>,
    /// `ToggleSwitchKnobFillOn*`.
    pub knob_on: D2D1_COLOR_F,
    /// `ToggleSwitchContentForeground*` (header and On/Off content).
    pub content: D2D1_COLOR_F,
}

/// An on/off switch with optional header and On/Off content text.
#[derive(Clone, Debug)]
pub struct ToggleSwitch {
    /// Bounds in DIPs: header block (if any) on top, then the 40-DIP switch row.
    pub rect: D2D_RECT_F,
    /// `Header` (also the UI Automation name); empty for none.
    pub header: String,
    /// `OnContent`.
    pub on_content: String,
    /// `OffContent`.
    pub off_content: String,
    on: bool,
    enabled: bool,
    pointer_over: bool,
    /// `m_isDragging`: the Thumb drag runs from press to release (Pressed visual).
    dragging: bool,
    /// Pointer x at press.
    press_x: f32,
    /// Knob translation at press.
    press_knob: f32,
    /// The press moved past the drag threshold (`m_wasDragged`).
    moved: bool,
    /// `m_handledKeyDown` for Space.
    space_down: bool,
    focus: Option<FocusKind>,
    /// `KnobTranslateTransform.X`, 0 (Off) to [`TRAVEL`] (On).
    knob_x: Tween,
    knob_w: Tween,
    knob_h: Tween,
    /// Opacity of `SwitchKnobBounds` / `SwitchKnobOn`; `OuterBorder` shows `1 - it`.
    on_opacity: Tween,
    /// Off-track color transition: from this visual's brushes, 0 to 1.
    off_from: Visual,
    off_t: Tween,
}

impl ToggleSwitch {
    /// Enabled switch at `rect` (height from [`ToggleSwitch::desired_size`]).
    pub fn new(header: &str, on_content: &str, off_content: &str, rect: D2D_RECT_F, on: bool) -> Self {
        let (w, h) = knob_size(Visual::Rest);
        Self {
            rect,
            header: header.to_owned(),
            on_content: on_content.to_owned(),
            off_content: off_content.to_owned(),
            on,
            enabled: true,
            pointer_over: false,
            dragging: false,
            press_x: 0.0,
            press_knob: 0.0,
            moved: false,
            space_down: false,
            focus: None,
            knob_x: Tween::idle(if on { TRAVEL } else { 0.0 }),
            knob_w: Tween::idle(w),
            knob_h: Tween::idle(h),
            on_opacity: Tween::idle(if on { 1.0 } else { 0.0 }),
            off_from: Visual::Rest,
            off_t: Tween::idle(1.0),
        }
    }

    /// Body text format for header and content: left-aligned
    /// (`HorizontalContentAlignment=Left`), vertically centered.
    pub fn text_format(surface: &Surface) -> Result<IDWriteTextFormat> {
        let format = surface.format(&theme::BODY)?;
        unsafe { format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)? };
        Ok(format)
    }

    /// Auto size: header line + 4 (when present) over a 40-DIP row; width at
    /// least `ToggleSwitchThemeMinWidth`, rounded up to whole DIPs.
    pub fn desired_size(
        surface: &Surface, format: &IDWriteTextFormat, header: &str, on_content: &str, off_content: &str,
    ) -> Result<(f32, f32)> {
        let (on_w, _) = surface.measure(on_content, format)?;
        let (off_w, _) = surface.measure(off_content, format)?;
        let mut width = TRACK_W + GAP + on_w.max(off_w);
        let mut height = ROW;
        if !header.is_empty() {
            let (hw, hh) = surface.measure(header, format)?;
            width = width.max(hw);
            height += hh + HEADER_GAP;
        }
        Ok((width.max(MIN_WIDTH).ceil(), height.ceil()))
    }

    /// `IsOn`.
    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Current visual state (`Pressed` while the Thumb drag runs).
    pub fn visual(&self) -> Visual {
        if !self.enabled {
            Visual::Disabled
        } else if self.dragging {
            Visual::Pressed
        } else if self.pointer_over {
            Visual::PointerOver
        } else {
            Visual::Rest
        }
    }

    /// True while this switch holds the mouse capture.
    pub fn is_captured(&self) -> bool {
        self.dragging
    }

    /// True when the focus rectangle must be drawn (keyboard focus only).
    pub fn focus_visible(&self) -> bool {
        self.enabled && self.focus == Some(FocusKind::Keyboard)
    }

    /// The 40-DIP switch row (pointer target: the `Thumb` spans it).
    fn row(&self) -> D2D_RECT_F {
        let top = self.rect.bottom - ROW;
        rect(self.rect.left, top, self.rect.right, self.rect.bottom)
    }

    /// `OuterBorder` / `SwitchKnobBounds`.
    pub fn track(&self) -> D2D_RECT_F {
        let top = self.row().top + PRE;
        rect(self.rect.left, top, self.rect.left + TRACK_W, top + TRACK_H)
    }

    /// `SwitchAreaGrid`: the focus target (row inset 5 DIPs vertically).
    pub fn focus_target(&self) -> D2D_RECT_F {
        let r = self.row();
        rect(r.left, r.top + AREA_INSET, r.right, r.bottom - AREA_INSET)
    }

    /// Current knob bounds (animated size and translation; Pressed anchors it).
    pub fn knob(&self) -> D2D_RECT_F {
        let track = self.track();
        // `SwitchKnob`: a 20 x 20 cell at the track's left, moved by KnobTranslateTransform.
        let cell = track.left + self.knob_x.value();
        let (w, h) = (self.knob_w.value(), self.knob_h.value());
        let (left, right) = if self.visual() == Visual::Pressed {
            // Pressed: the Off knob is anchored left (Margin 3,0,0,0), the On knob right (0,0,3,0).
            if self.on { (cell + TRAVEL - 3.0 - w, cell + TRAVEL - 3.0) } else { (cell + 3.0, cell + 3.0 + w) }
        } else {
            // Centered with Margin -1,0,0,0 (Off) or 0,0,1,0 (On): both centers land at 9.5.
            (cell + 9.5 - w / 2.0, cell + 9.5 + w / 2.0)
        };
        let cy = (track.top + track.bottom) / 2.0;
        rect(left, cy - h / 2.0, right, cy + h / 2.0)
    }

    /// Enables or disables the switch (`OnIsEnabledChanged`).
    pub fn set_enabled(&mut self, enabled: bool) -> Response {
        let before = self.snapshot();
        let capture = (self.dragging && !enabled).then_some(false);
        self.enabled = enabled;
        self.pointer_over = false;
        let lost_focus = !enabled && self.focus.take().is_some();
        if !enabled {
            self.dragging = false;
            self.moved = false;
            self.space_down = false;
            self.knob_x = Tween::idle(self.rest_x());
        }
        Response { capture, lost_focus, ..self.changed(before) }
    }

    /// `Toggle()` (UI Automation Toggle pattern). Does nothing while disabled.
    pub fn invoke(&mut self) -> Response {
        if !self.enabled {
            return Response::default();
        }
        let before = self.snapshot();
        self.toggle();
        Response { clicked: true, ..self.changed(before) }
    }

    /// Feeds one input event; `Response::clicked` means `IsOn` changed (`Toggled`).
    pub fn handle(&mut self, input: Input) -> Response {
        let before = self.snapshot();
        let was_on = self.on;
        let mut r = Response::default();
        match input {
            Input::PointerMove(x, y) => {
                self.pointer_over = contains(self.row(), x, y);
                if self.dragging {
                    let dx = x - self.press_x;
                    self.moved |= dx.abs() >= DRAG_THRESHOLD;
                    if self.moved {
                        // MoveDelta: the knob follows the pointer, within the track.
                        self.knob_x = Tween::idle((self.press_knob + dx).clamp(0.0, TRAVEL));
                    }
                }
            }
            // The Thumb starts its drag on press (DragStarted): Pressed + pointer focus.
            Input::PointerDown(x, y) if self.enabled && contains(self.row(), x, y) => {
                self.dragging = true;
                self.moved = false;
                self.press_x = x;
                self.press_knob = self.knob_x.value();
                self.focus = Some(FocusKind::Pointer);
                r.capture = Some(true);
                r.take_focus = true;
            }
            Input::PointerUp(x, y) if self.dragging => {
                self.dragging = false;
                self.pointer_over = contains(self.row(), x, y);
                r.capture = Some(false);
                if self.moved {
                    // MoveCompleted: toggles when the knob crossed half the travel.
                    let k = self.knob_x.value();
                    let crossed = if self.on { k <= TRAVEL / 2.0 } else { k >= TRAVEL / 2.0 };
                    self.on ^= crossed;
                    // Dragging -> On/Off: the state sets X with Duration 0 (snap) and
                    // DraggingToOn/OffTransition fade the On visuals over 83 ms.
                    self.knob_x = Tween::idle(self.rest_x());
                    self.on_opacity.retarget(if self.on { 1.0 } else { 0.0 }, easing::FASTER, easing::LINEAR);
                } else if self.pointer_over {
                    // Tapped.
                    self.toggle();
                }
            }
            Input::PointerLeave if !self.dragging => self.pointer_over = false,
            // OnKeyDown only marks Space as handled; KeyUp toggles (HandlesKey: Space only).
            Input::KeyDown(Key::Space) if self.enabled && self.focus.is_some() && !self.dragging => self.space_down = true,
            Input::KeyUp(Key::Space) if self.space_down => {
                self.space_down = false;
                if !self.dragging {
                    self.toggle();
                }
            }
            Input::Focus(kind) => self.focus = Some(kind),
            Input::Blur => {
                self.focus = None;
                self.space_down = false;
            }
            Input::CaptureLost if self.dragging => {
                // The drag is abandoned: no toggle, the knob returns to its end.
                self.dragging = false;
                self.moved = false;
                self.knob_x = Tween::idle(self.rest_x());
            }
            _ => {}
        }
        let clicked = self.on != was_on;
        Response { clicked, capture: r.capture, take_focus: r.take_focus, ..self.changed(before) }
    }

    /// Knob translation at rest for the current `IsOn`.
    fn rest_x(&self) -> f32 {
        if self.on { TRAVEL } else { 0.0 }
    }

    /// `Toggle()`, then the Off/On transition: RepositionThemeAnimation slides the
    /// knob (367 ms); OffToOn fades the On visuals in over 83 ms, OnToOff snaps them.
    fn toggle(&mut self) {
        self.on = !self.on;
        self.knob_x.retarget(self.rest_x(), easing::REPOSITION_DURATION, easing::REPOSITION);
        if self.on {
            self.on_opacity.retarget(1.0, easing::FASTER, easing::LINEAR);
        } else {
            self.on_opacity = Tween::idle(0.0);
        }
    }

    fn snapshot(&self) -> (Visual, bool, bool, f32) {
        (self.visual(), self.focus_visible(), self.on, self.knob_x.value())
    }

    fn changed(&mut self, before: (Visual, bool, bool, f32)) -> Response {
        let visual = self.visual();
        if visual != before.0 {
            // Knob Width/Height: SplineDoubleKeyFrame 0,0,0,1; 83 ms (250 ms into Disabled).
            let (w, h) = knob_size(visual);
            let duration = if visual == Visual::Disabled { easing::NORMAL } else { easing::FASTER };
            self.knob_w.retarget(w, duration, easing::FAST_OUT_SLOW_IN);
            self.knob_h.retarget(h, duration, easing::FAST_OUT_SLOW_IN);
            // OuterBorder colors: ColorAnimation 83 ms into PointerOver/Pressed/Disabled,
            // discrete into Normal.
            // shortcut: restarts from the previous state colors, not the blended ones; track the drawn color if it shows.
            self.off_from = before.0;
            self.off_t = Tween::idle(if visual == Visual::Rest { 1.0 } else { 0.0 });
            self.off_t.retarget(1.0, easing::FASTER, easing::LINEAR);
        }
        Response { redraw: self.snapshot() != before, ..Response::default() }
    }

    /// True while any transition runs.
    pub fn animating(&self) -> bool {
        [self.knob_x, self.knob_w, self.knob_h, self.on_opacity, self.off_t].iter().any(Tween::running)
    }

    /// Target brushes for the current visual state.
    pub fn style(&self, t: &Theme) -> SwitchStyle {
        style_of(self.visual(), t)
    }

    /// Draws header, track, knob, On/Off content and the focus rectangle.
    pub fn draw(&self, s: &Surface, t: &Theme, format: &IDWriteTextFormat) -> Result<()> {
        let style = self.style(t);
        let from = style_of(self.off_from, t);
        let k = self.off_t.value();
        let fill_off = lerp(from.fill_off, style.fill_off, k);
        let stroke_off = lerp(from.stroke_off, style.stroke_off, k);
        let on = self.on_opacity.value();
        let track = self.track();
        let radius = TRACK_H / 2.0;
        if !self.header.is_empty() {
            let r = self.rect;
            s.text(&self.header, format, rect(r.left, r.top, r.right, self.row().top - HEADER_GAP), style.content);
        }
        if on < 1.0 {
            s.fill_round(track, radius, theme::with_opacity(fill_off, 1.0 - on));
            s.stroke_round(track, radius, 1.0, theme::with_opacity(stroke_off, 1.0 - on));
        }
        if on > 0.0 {
            s.fill_round(track, radius, theme::with_opacity(style.fill_on, on));
            if let Some(stroke) = style.stroke_on {
                s.stroke_round(track, radius, 1.0, theme::with_opacity(stroke, on));
            }
        }
        let knob = self.knob();
        let knob_radius = 7.0_f32.min((knob.bottom - knob.top) / 2.0);
        if on < 1.0 {
            s.fill_round(knob, knob_radius, theme::with_opacity(style.knob_off, 1.0 - on));
        }
        if on > 0.0 {
            s.fill_round(knob, knob_radius, theme::with_opacity(style.knob_on, on));
        }
        let content = if self.on { &self.on_content } else { &self.off_content };
        let row = self.row();
        s.text(content, format, rect(track.right + GAP, row.top, row.right, row.bottom), style.content);
        if self.focus_visible() {
            let target = self.focus_target();
            let outer = rect(target.left - FOCUS_MARGIN.0, target.top - FOCUS_MARGIN.1,
                target.right + FOCUS_MARGIN.0, target.bottom + FOCUS_MARGIN.1);
            let r = theme::CONTROL_RADIUS + FOCUS_MARGIN.1;
            s.stroke_round(outer, r, 2.0, t.focus_outer);
            let ring = s.snap_len(2.0);
            let inner = rect(outer.left + ring, outer.top + ring, outer.right - ring, outer.bottom - ring);
            s.stroke_round(inner, r - 2.0, 1.0, t.focus_inner);
        }
        Ok(())
    }
}

fn contains(r: D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

/// `ToggleSwitch_themeresources.xaml`; high contrast resolves `SystemColor*` directly.
fn style_of(visual: Visual, t: &Theme) -> SwitchStyle {
    if let Some(c) = t.high_contrast {
        let (fill_off, stroke_off, knob_off, fill_on, stroke_on, knob_on, content) = match visual {
            Visual::Rest => (c.button_face, c.button_text, c.button_text, c.highlight, c.highlight, c.highlight_text, c.window_text),
            Visual::PointerOver | Visual::Pressed => {
                (c.highlight_text, c.highlight, c.highlight, c.button_face, c.button_text, c.button_text, c.window_text)
            }
            Visual::Disabled => (c.window, c.gray_text, c.gray_text, c.window, c.gray_text, c.gray_text, c.gray_text),
        };
        return SwitchStyle { fill_off, stroke_off, knob_off, fill_on, stroke_on: Some(stroke_on), knob_on, content };
    }
    let base = SwitchStyle {
        fill_off: t.control_alt_fill_secondary,
        stroke_off: t.control_strong_stroke,
        knob_off: t.text_secondary,
        fill_on: t.accent,
        stroke_on: None,
        knob_on: t.text_on_accent,
        content: t.text_primary,
    };
    match visual {
        Visual::Rest => base,
        Visual::PointerOver => {
            SwitchStyle { fill_off: t.control_alt_fill_tertiary, fill_on: theme::with_opacity(t.accent, 0.9), ..base }
        }
        Visual::Pressed => {
            SwitchStyle { fill_off: t.control_alt_fill_quarternary, fill_on: theme::with_opacity(t.accent, 0.8), ..base }
        }
        Visual::Disabled => SwitchStyle {
            fill_off: t.control_alt_fill_disabled,
            stroke_off: t.control_strong_stroke_disabled,
            knob_off: t.text_disabled,
            fill_on: t.accent_disabled,
            stroke_on: None,
            knob_on: t.text_on_accent_disabled,
            content: t.text_disabled,
        },
    }
}

fn lerp(a: D2D1_COLOR_F, b: D2D1_COLOR_F, k: f32) -> D2D1_COLOR_F {
    let m = |x: f32, y: f32| x + (y - x) * k;
    D2D1_COLOR_F { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: m(a.a, b.a) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tests::HC;

    /// Header-less switch at (0, 0), 154 x 40, Off.
    fn switch(on: bool) -> ToggleSwitch {
        ToggleSwitch::new("", "Ligado", "Desligado", rect(0.0, 0.0, 154.0, 40.0), on)
    }

    /// Point on the track.
    const TRACK: (f32, f32) = (20.0, 20.0);

    fn tap(s: &mut ToggleSwitch) -> Response {
        s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        s.handle(Input::PointerUp(TRACK.0, TRACK.1))
    }

    fn drag(s: &mut ToggleSwitch, dx: f32) -> Response {
        s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        s.handle(Input::PointerMove(TRACK.0 + dx, TRACK.1));
        s.handle(Input::PointerUp(TRACK.0 + dx, TRACK.1))
    }

    #[test]
    fn track_should_be_40_by_20_after_the_pre_margin() {
        let t = switch(false).track();
        assert_eq!((t.left, t.top, t.right, t.bottom), (0.0, 10.0, 40.0, 30.0));
    }

    #[test]
    fn header_should_push_the_row_down() {
        let s = ToggleSwitch::new("Wi-Fi", "Ligado", "Desligado", rect(0.0, 0.0, 154.0, 64.0), false);
        assert_eq!((s.track().top, s.focus_target().top), (34.0, 29.0));
    }

    #[test]
    fn tap_should_toggle_on_and_report_it() {
        let mut s = switch(false);
        let r = tap(&mut s);
        assert_eq!((s.is_on(), r.clicked), (true, true));
    }

    #[test]
    fn press_should_capture_take_focus_and_show_pressed() {
        let mut s = switch(false);
        let r = s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        assert_eq!((r.capture, r.take_focus, s.visual(), s.focus_visible()), (Some(true), true, Visual::Pressed, false));
    }

    #[test]
    fn press_outside_the_row_should_be_ignored() {
        let mut s = ToggleSwitch::new("Wi-Fi", "Ligado", "Desligado", rect(0.0, 0.0, 154.0, 64.0), false);
        assert_eq!(s.handle(Input::PointerDown(20.0, 5.0)), Response::default());
    }

    #[test]
    fn release_should_free_the_capture() {
        let mut s = switch(false);
        assert_eq!(tap(&mut s).capture, Some(false));
    }

    #[test]
    fn drag_past_half_should_toggle() {
        let mut s = switch(false);
        let r = drag(&mut s, 12.0);
        assert_eq!((s.is_on(), r.clicked), (true, true));
    }

    #[test]
    fn short_drag_should_not_toggle() {
        let mut s = switch(false);
        let r = drag(&mut s, 6.0);
        assert_eq!((s.is_on(), r.clicked), (false, false));
    }

    #[test]
    fn drag_from_on_toward_off_should_toggle_off() {
        let mut s = switch(true);
        drag(&mut s, -12.0);
        assert!(!s.is_on());
    }

    #[test]
    fn drag_should_move_the_knob_with_the_pointer_within_the_track() {
        let mut s = switch(false);
        s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        s.handle(Input::PointerMove(TRACK.0 + 50.0, TRACK.1));
        // Clamped to the On end; the pressed Off knob is anchored left at 3 in its 20-DIP cell.
        assert_eq!((s.knob_x.value(), s.knob().left), (TRAVEL, 23.0));
    }

    #[test]
    fn tiny_jitter_should_still_count_as_a_tap() {
        let mut s = switch(false);
        s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        s.handle(Input::PointerMove(TRACK.0 + 1.0, TRACK.1));
        let r = s.handle(Input::PointerUp(TRACK.0 + 1.0, TRACK.1));
        assert_eq!((s.is_on(), r.clicked), (true, true));
    }

    #[test]
    fn capture_lost_should_abandon_the_drag_without_toggling() {
        let mut s = switch(false);
        s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        s.handle(Input::PointerMove(TRACK.0 + 15.0, TRACK.1));
        s.handle(Input::CaptureLost);
        let up = s.handle(Input::PointerUp(TRACK.0 + 15.0, TRACK.1));
        assert_eq!((s.is_on(), s.visual() == Visual::Pressed, up.clicked), (false, false, false));
    }

    #[test]
    fn space_up_after_space_down_should_toggle() {
        let mut s = switch(false);
        s.handle(Input::Focus(FocusKind::Keyboard));
        s.handle(Input::KeyDown(Key::Space));
        let r = s.handle(Input::KeyUp(Key::Space));
        assert_eq!((s.is_on(), r.clicked), (true, true));
    }

    #[test]
    fn enter_should_not_toggle() {
        let mut s = switch(false);
        s.handle(Input::Focus(FocusKind::Keyboard));
        s.handle(Input::KeyDown(Key::Enter));
        s.handle(Input::KeyUp(Key::Enter));
        assert!(!s.is_on());
    }

    #[test]
    fn space_up_without_its_key_down_should_not_toggle() {
        let mut s = switch(false);
        s.handle(Input::Focus(FocusKind::Keyboard));
        s.handle(Input::KeyUp(Key::Space));
        assert!(!s.is_on());
    }

    #[test]
    fn invoke_should_toggle_and_disabled_should_ignore_it() {
        let mut s = switch(false);
        let first = s.invoke();
        s.set_enabled(false);
        let second = s.invoke();
        assert_eq!((first.clicked, second, s.is_on()), (true, Response::default(), true));
    }

    #[test]
    fn disabling_should_drop_focus_and_release_a_drag() {
        let mut s = switch(false);
        s.handle(Input::PointerDown(TRACK.0, TRACK.1));
        let r = s.set_enabled(false);
        assert_eq!((r.capture, r.lost_focus, s.visual()), (Some(false), true, Visual::Disabled));
    }

    #[test]
    fn toggling_on_should_slide_the_knob_with_the_reposition_curve() {
        let mut s = switch(false);
        s.invoke();
        assert_eq!((s.knob_x.target(), s.knob_x.running()), (TRAVEL, true));
    }

    #[test]
    fn toggling_off_should_snap_the_on_visuals_but_slide_the_knob() {
        let mut s = switch(true);
        s.invoke();
        // OnToOffTransition has only the reposition: opacities jump, the knob slides.
        assert_eq!((s.on_opacity.value(), s.on_opacity.running(), s.knob_x.running()), (0.0, false, true));
    }

    #[test]
    fn toggling_on_should_fade_the_on_visuals_in() {
        let mut s = switch(false);
        s.invoke();
        assert_eq!((s.on_opacity.target(), s.on_opacity.running()), (1.0, true));
    }

    #[test]
    fn hover_should_grow_the_knob_to_14() {
        let mut s = switch(false);
        s.handle(Input::PointerMove(TRACK.0, TRACK.1));
        assert_eq!((s.knob_w.target(), s.knob_h.target()), (14.0, 14.0));
    }

    #[test]
    fn rest_knob_should_be_12_centered_in_the_off_cell() {
        let k = switch(false).knob();
        // SwitchKnob cell 20 x 20 at the track's left; knob centered at x 9.5 (Margin -1,0,0,0).
        assert_eq!((k.left, k.top, k.right, k.bottom), (3.5, 14.0, 15.5, 26.0));
    }

    #[test]
    fn rest_knob_on_should_sit_in_the_on_cell() {
        let k = switch(true).knob();
        assert_eq!((k.left, k.right), (23.5, 35.5));
    }

    #[test]
    fn off_rest_style_should_use_control_alt_fill_and_strong_stroke() {
        let (s, t) = (switch(false).style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.fill_off, s.stroke_off, s.knob_off, s.content),
            (t.control_alt_fill_secondary, t.control_strong_stroke, t.text_secondary, t.text_primary));
    }

    #[test]
    fn on_pointer_over_style_should_use_accent_secondary() {
        let mut s = switch(true);
        s.handle(Input::PointerMove(TRACK.0, TRACK.1));
        let (st, t) = (s.style(&Theme::LIGHT), Theme::LIGHT);
        assert_eq!((st.fill_on, st.knob_on), (theme::with_opacity(t.accent, 0.9), t.text_on_accent));
    }

    #[test]
    fn disabled_style_should_use_disabled_tokens() {
        let mut s = switch(false);
        s.set_enabled(false);
        let (st, t) = (s.style(&Theme::DARK), Theme::DARK);
        assert_eq!((st.fill_off, st.stroke_off, st.knob_off, st.fill_on, st.knob_on, st.content),
            (t.control_alt_fill_disabled, t.control_strong_stroke_disabled, t.text_disabled, t.accent_disabled,
             t.text_on_accent_disabled, t.text_disabled));
    }

    #[test]
    fn high_contrast_pointer_over_style_should_use_highlight() {
        let mut s = switch(false);
        s.handle(Input::PointerMove(TRACK.0, TRACK.1));
        let st = s.style(&Theme::high_contrast(HC));
        assert_eq!((st.fill_off, st.stroke_off, st.knob_off, st.fill_on, st.knob_on, st.content),
            (HC.highlight_text, HC.highlight, HC.highlight, HC.button_face, HC.button_text, HC.window_text));
    }
}
