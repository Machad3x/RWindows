//! WinUI `Button` (`DefaultButtonStyle` / `AccentButtonStyle`) for raw Win32.
//!
//! Input rules follow WinUI's `ButtonBase` with the default `ClickMode.Release`
//! (`ButtonBase_Partial.cpp`, `ButtonBaseKeyProcess.h`).

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER, IDWriteTextFormat,
};
use windows::core::Result;

use crate::easing::{self, Tween};
use crate::gfx::{Edge, Surface, rect};
use crate::theme::{self, SystemColors, Theme, with_opacity};

/// `ButtonPadding` = 11,5,11,6 (left, top, right, bottom).
const PADDING: (f32, f32, f32, f32) = (11.0, 5.0, 11.0, 6.0);
/// `ButtonBorderThemeThickness`.
const BORDER: f32 = 1.0;
/// `FocusVisualMargin` = -3 on `DefaultButtonStyle`.
const FOCUS_MARGIN: f32 = 3.0;

/// Keys a button reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Space bar.
    Space,
    /// Enter.
    Enter,
    /// Numpad `+` (`VirtualKey_Add`: checks a CheckBox).
    Add,
    /// Numpad `-` (`VirtualKey_Subtract`: unchecks a CheckBox).
    Subtract,
    /// Any other key (cancels a held Space/Enter press).
    Other,
}

/// How focus arrived; only keyboard focus shows the focus rectangle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusKind {
    /// Tab / arrow navigation.
    Keyboard,
    /// Pointer press.
    Pointer,
}

/// Input event, pointer coordinates in DIPs in the same space as `rect`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Input {
    /// Pointer moved to (x, y).
    PointerMove(f32, f32),
    /// Left button pressed at (x, y).
    PointerDown(f32, f32),
    /// Left button released at (x, y).
    PointerUp(f32, f32),
    /// Pointer left the window.
    PointerLeave,
    /// Key pressed (only handled while focused).
    KeyDown(Key),
    /// Key released (only handled while focused).
    KeyUp(Key),
    /// The button received focus.
    Focus(FocusKind),
    /// The button lost focus.
    Blur,
    /// Someone else took the mouse capture (`WM_CAPTURECHANGED`), like
    /// `OnPointerCaptureLost`: the press is abandoned without a click.
    CaptureLost,
}

/// Visual state, in WinUI priority order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visual {
    /// `Normal`.
    Rest,
    /// `PointerOver`.
    PointerOver,
    /// `Pressed`.
    Pressed,
    /// `Disabled`.
    Disabled,
}

/// What the caller must do after [`Button::handle`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Response {
    /// Visual or focus ring changed: repaint.
    pub redraw: bool,
    /// The button was invoked.
    pub clicked: bool,
    /// `Some(true)`: call `SetCapture`; `Some(false)`: call `ReleaseCapture`.
    pub capture: Option<bool>,
    /// The button wants keyboard focus (pointer press): blur the others.
    pub take_focus: bool,
    /// The button dropped focus (it was disabled): move focus elsewhere.
    pub lost_focus: bool,
}

/// Border of one visual state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Border {
    /// `ControlElevationBorderBrush`-style gradient: lit/shaded `edge`, then `rest`.
    Elevation {
        /// Edge carrying `edge_color`.
        edge: Edge,
        /// Color at the edge (stop 0.33).
        edge_color: D2D1_COLOR_F,
        /// Color everywhere else (stop 1.0).
        rest: D2D1_COLOR_F,
    },
    /// Solid 1-DIP border.
    Solid(D2D1_COLOR_F),
    /// No border (`ControlFillColorTransparentBrush`).
    None,
}

/// Brushes of one visual state (`Button_themeresources.xaml`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    /// `ButtonBackground*` / `AccentButtonBackground*`.
    pub background: D2D1_COLOR_F,
    /// `ButtonForeground*` / `AccentButtonForeground*`.
    pub foreground: D2D1_COLOR_F,
    /// `ButtonBorderBrush*` / `AccentButtonBorderBrush*`.
    pub border: Border,
}

/// A push button.
#[derive(Clone, Debug)]
pub struct Button {
    /// Bounds in DIPs.
    pub rect: D2D_RECT_F,
    /// Text content.
    pub label: String,
    /// `AccentButtonStyle` instead of `DefaultButtonStyle`.
    pub accent: bool,
    enabled: bool,
    pointer_over: bool,
    pressed: bool,
    /// `m_bIsPointerLeftButtonDown` (implies pointer capture here).
    left_down: bool,
    /// `m_bIsSpaceOrEnterKeyDown`.
    key_down: bool,
    focus: Option<FocusKind>,
    /// `ToggleButton.IsChecked`; `None` for a plain button.
    checked: Option<bool>,
    /// `SubtleButtonStyle`.
    subtle: bool,
    /// `m_bKeyboardNavigationAcceptsReturn`: Enter presses like Space (false on CheckBox).
    accepts_return: bool,
    /// `IsThreeState`: Checked toggles to Indeterminate instead of Unchecked.
    three_state: bool,
    /// `IsChecked == null` (`checked` then holds `Some(false)`).
    indeterminate: bool,
    /// State the background is transitioning from (`BackgroundTransition`).
    bg_from: (Visual, Look),
    bg: Tween,
}

/// Which brush table applies: `Checked` is the `ToggleButton` checked states.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Look {
    Default,
    Accent,
    Checked,
    Subtle,
}

/// `ButtonBase::IsValidPointerPosition` tolerance, in DIPs.
const TOLERANCE: f32 = 0.05;

impl Button {
    /// Enabled button at `rect`.
    pub fn new(label: &str, rect: D2D_RECT_F, accent: bool) -> Self {
        Self {
            rect,
            label: label.to_owned(),
            accent,
            enabled: true,
            pointer_over: false,
            pressed: false,
            left_down: false,
            key_down: false,
            focus: None,
            checked: None,
            subtle: false,
            accepts_return: true,
            three_state: false,
            indeterminate: false,
            bg_from: (Visual::Rest, if accent { Look::Accent } else { Look::Default }),
            bg: Tween::idle(1.0),
        }
    }

    /// WinUI `ToggleButton` (`DefaultToggleButtonStyle`), two-state.
    pub fn toggle(label: &str, rect: D2D_RECT_F, checked: bool) -> Self {
        Self { checked: Some(checked), ..Self::new(label, rect, false) }
    }

    /// WinUI `SubtleButtonStyle`: no fill or border at rest (taskbar, Start, title bar buttons).
    pub fn subtle(label: &str, rect: D2D_RECT_F) -> Self {
        Self { subtle: true, ..Self::new(label, rect, false) }
    }

    /// `SetAcceptsReturn` (CheckBox and RadioButton ignore Enter).
    pub(crate) fn set_accepts_return(&mut self, accepts: bool) {
        self.accepts_return = accepts;
    }

    /// `IsThreeState` (CheckBox only for now).
    pub(crate) fn set_three_state(&mut self, three_state: bool) {
        self.three_state = three_state;
    }

    /// `IsThreeState`.
    pub(crate) fn is_three_state(&self) -> bool {
        self.three_state
    }

    /// True when `IsChecked` is null (Indeterminate).
    pub(crate) fn is_indeterminate(&self) -> bool {
        self.indeterminate
    }

    /// Sets `IsChecked` to null (Indeterminate) without a click.
    pub(crate) fn set_indeterminate(&mut self) -> Response {
        let before = self.snapshot();
        if self.checked.is_some() {
            self.checked = Some(false);
            self.indeterminate = true;
        }
        self.changed(before)
    }

    /// `put_IsPressed(FALSE)`: drops the pressed state (and a held Space/Enter) but
    /// keeps the pointer capture; a later move back inside presses again.
    pub(crate) fn cancel_press(&mut self) -> Response {
        let before = self.snapshot();
        self.pressed = false;
        self.key_down = false;
        self.changed(before)
    }

    /// Sets `IsChecked` of a toggle button without a click.
    pub(crate) fn set_checked(&mut self, checked: bool) -> Response {
        let before = self.snapshot();
        if self.checked.is_some() {
            self.checked = Some(checked);
            self.indeterminate = false;
        }
        self.changed(before)
    }

    /// `IsChecked` of a toggle button; `None` for a plain button.
    pub fn is_checked(&self) -> Option<bool> {
        self.checked
    }

    /// Programmatic click (`OnClick`, used by UI Automation Invoke/Toggle): a toggle
    /// button flips `IsChecked` first. Does nothing while disabled.
    pub fn invoke(&mut self) -> Response {
        if !self.enabled {
            return Response::default();
        }
        let before = self.snapshot();
        self.on_click();
        Response { clicked: true, ..self.changed(before) }
    }

    /// `ToggleButton::OnClick`: `OnToggle` before the click. `OnToggleImpl`:
    /// Indeterminate -> Unchecked, Checked -> Indeterminate (three-state) or
    /// Unchecked, Unchecked -> Checked.
    fn on_click(&mut self) {
        let Some(checked) = self.checked else { return };
        let (next, indeterminate) = match (self.indeterminate, checked) {
            (true, _) => (false, false),
            (false, true) => (false, self.three_state),
            (false, false) => (true, false),
        };
        self.checked = Some(next);
        self.indeterminate = indeterminate;
    }

    /// Body text format centered like `HorizontalContentAlignment`/`VerticalContentAlignment=Center`.
    pub fn text_format(surface: &Surface) -> Result<IDWriteTextFormat> {
        let format = surface.format(&theme::BODY)?;
        unsafe {
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        Ok(format)
    }

    /// Auto size for `label`: text + padding + border, rounded to whole DIPs
    /// like XAML layout rounding at 100% scale.
    pub fn desired_size(surface: &Surface, format: &IDWriteTextFormat, label: &str) -> Result<(f32, f32)> {
        let (w, h) = surface.measure(label, format)?;
        let width = w + PADDING.0 + PADDING.2 + 2.0 * BORDER;
        let height = h + PADDING.1 + PADDING.3 + 2.0 * BORDER;
        Ok((width.ceil(), height.round()))
    }

    /// True while this button holds the mouse capture (left button pressed on it).
    pub fn is_captured(&self) -> bool {
        self.left_down
    }

    /// Current visual state.
    pub fn visual(&self) -> Visual {
        if !self.enabled {
            Visual::Disabled
        } else if self.pressed {
            Visual::Pressed
        } else if self.pointer_over {
            Visual::PointerOver
        } else {
            Visual::Rest
        }
    }

    /// True while the button has focus of any kind.
    pub(crate) fn has_focus(&self) -> bool {
        self.focus.is_some()
    }

    /// True when the focus rectangle must be drawn (keyboard focus only).
    pub fn focus_visible(&self) -> bool {
        self.enabled && self.focus == Some(FocusKind::Keyboard)
    }

    /// Enables or disables the button (`OnIsEnabledChanged` clears pressed and pointer-over).
    pub fn set_enabled(&mut self, enabled: bool) -> Response {
        let before = self.snapshot();
        let capture = (self.left_down && !enabled).then_some(false);
        self.enabled = enabled;
        self.pressed = false;
        self.pointer_over = false;
        // A disabled control takes no input, so a keyboard ring on it would be a dead end: drop focus, the caller moves it.
        let lost_focus = !enabled && self.focus.take().is_some();
        if !enabled {
            self.left_down = false;
            self.key_down = false;
        }
        Response { capture, lost_focus, ..self.changed(before) }
    }

    /// Feeds one input event.
    pub fn handle(&mut self, input: Input) -> Response {
        // IsPress: Enter counts only when the control accepts return.
        let input = match input {
            Input::KeyDown(Key::Enter) if !self.accepts_return => Input::KeyDown(Key::Other),
            Input::KeyUp(Key::Enter) if !self.accepts_return => Input::KeyUp(Key::Other),
            other => other,
        };
        let before = self.snapshot();
        let mut r = Response::default();
        match input {
            Input::PointerMove(x, y) => {
                self.pointer_over = self.contains(x, y, 0.0);
                if self.enabled && self.left_down && !self.key_down {
                    self.pressed = self.contains(x, y, TOLERANCE);
                }
            }
            Input::PointerDown(x, y) if self.enabled && self.contains(x, y, 0.0) => {
                self.left_down = true;
                self.pressed = true;
                self.focus = Some(FocusKind::Pointer);
                r.capture = Some(true);
                r.take_focus = true;
            }
            Input::PointerUp(x, y) if self.left_down => {
                self.left_down = false;
                r.clicked = self.pressed && !self.key_down;
                self.pressed = false;
                self.pointer_over = self.contains(x, y, 0.0);
                r.capture = Some(false);
            }
            Input::PointerLeave if !self.left_down => self.pointer_over = false,
            Input::KeyDown(key) if self.enabled && self.focus.is_some() => match key {
                Key::Space | Key::Enter if !self.left_down && !self.key_down => {
                    self.key_down = true;
                    self.pressed = true;
                }
                Key::Other | Key::Add | Key::Subtract if self.key_down => {
                    self.key_down = false;
                    self.pressed = false;
                }
                _ => {}
            },
            Input::KeyUp(Key::Space | Key::Enter) if self.enabled && self.focus.is_some() => {
                self.key_down = false;
                if !self.left_down {
                    r.clicked = self.pressed;
                    self.pressed = false;
                }
            }
            Input::Focus(kind) => self.focus = Some(kind),
            Input::CaptureLost => {
                // ReleasePointerCaptureInternal clears left-down; then IsPressed = false.
                self.left_down = false;
                self.pressed = false;
            }
            Input::Blur => {
                self.focus = None;
                self.pressed = false;
                self.key_down = false;
                if self.left_down {
                    self.left_down = false;
                    r.capture = Some(false);
                }
            }
            _ => {}
        }
        if r.clicked {
            self.on_click();
        }
        Response { clicked: r.clicked, capture: r.capture, take_focus: r.take_focus, ..self.changed(before) }
    }

    /// Target brushes for the current visual state.
    pub fn style(&self, t: &Theme) -> Style {
        style_of(self.visual(), self.look(), t)
    }

    /// `BackgroundSizing=OuterBorderEdge`: a Setter on `AccentButtonStyle`; on a
    /// ToggleButton only the Checked, CheckedPointerOver and CheckedPressed states
    /// set it (`CheckedDisabled` keeps the template's `InnerBorderEdge`).
    fn outer_background(&self) -> bool {
        match self.look() {
            Look::Accent => true,
            Look::Checked => self.visual() != Visual::Disabled,
            Look::Default | Look::Subtle => false,
        }
    }

    fn look(&self) -> Look {
        match (self.checked, self.accent, self.subtle) {
            (Some(true), ..) => Look::Checked,
            (_, true, _) => Look::Accent,
            (.., true) => Look::Subtle,
            _ => Look::Default,
        }
    }

    /// True while the 83 ms background transition runs.
    pub fn animating(&self) -> bool {
        self.bg.running()
    }

    /// Draws the button (and the focus rectangle when keyboard-focused).
    pub fn draw(&self, s: &Surface, t: &Theme, format: &IDWriteTextFormat) -> Result<()> {
        let style = self.style(t);
        let from = style_of(self.bg_from.0, self.bg_from.1, t).background;
        let background = lerp(from, style.background, self.bg.value());
        let radius = theme::CONTROL_RADIUS;
        let r = self.rect;
        if self.outer_background() {
            s.fill_round(r, radius, background);
        } else {
            // DefaultButtonStyle: BackgroundSizing=InnerBorderEdge. Inset by the border as
            // actually drawn (whole device pixels) so fill and stroke meet at 125%/150%.
            let border = s.snap_len(BORDER);
            s.fill_round(inset(r, border), radius - border, background);
        }
        match style.border {
            Border::Elevation { edge, edge_color, rest } => s.stroke_elevation(r, radius, edge, edge_color, rest)?,
            Border::Solid(color) => s.stroke_round(r, radius, BORDER, color),
            Border::None => {}
        }
        let content = rect(
            r.left + BORDER + PADDING.0,
            r.top + BORDER + PADDING.1,
            r.right - BORDER - PADDING.2,
            r.bottom - BORDER - PADDING.3,
        );
        s.text(&self.label, format, content, style.foreground);
        if self.focus_visible() {
            // FocusRectManager: outer ring at -FocusVisualMargin, 2 DIPs, radius + 3;
            // inner ring inset by the outer thickness, 1 DIP, radius reduced by 2.
            let outer = inset(r, -FOCUS_MARGIN);
            s.stroke_round(outer, radius + FOCUS_MARGIN, 2.0, t.focus_outer);
            let ring = s.snap_len(2.0);
            s.stroke_round(inset(outer, ring), radius + FOCUS_MARGIN - 2.0, 1.0, t.focus_inner);
        }
        Ok(())
    }

    fn contains(&self, x: f32, y: f32, tolerance: f32) -> bool {
        let r = self.rect;
        x >= r.left - tolerance && x <= r.right + tolerance && y >= r.top - tolerance && y <= r.bottom + tolerance
    }

    fn snapshot(&self) -> (Visual, bool, Look, bool) {
        (self.visual(), self.focus_visible(), self.look(), self.indeterminate)
    }

    fn changed(&mut self, before: (Visual, bool, Look, bool)) -> Response {
        if (self.visual(), self.look()) != (before.0, before.2) {
            // BrushTransition Duration=0:0:0.083 on the background only.
            // shortcut: a change mid-transition restarts from the previous state's color, not the blended one (<83 ms); track the drawn color if it shows.
            self.bg_from = (before.0, before.2);
            self.bg = Tween::idle(0.0);
            self.bg.retarget(1.0, easing::FASTER, easing::LINEAR);
        }
        Response { redraw: self.snapshot() != before, ..Response::default() }
    }
}

fn style_of(visual: Visual, look: Look, t: &Theme) -> Style {
    if let Some(c) = t.high_contrast {
        return high_contrast_style(visual, look, &c);
    }
    if look == Look::Subtle {
        return subtle_style(visual, t);
    }
    // ToggleButton Checked* states use the accent brushes outside high contrast.
    let accent = matches!(look, Look::Accent | Look::Checked);
    let elevation = |edge, edge_color, rest| Border::Elevation { edge, edge_color, rest };
    let default_edge = if t.dark { Edge::Top } else { Edge::Bottom };
    match (accent, visual) {
        (false, Visual::Rest | Visual::PointerOver) => Style {
            background: if visual == Visual::Rest { t.control_fill } else { t.control_fill_hover },
            foreground: t.text_primary,
            border: elevation(default_edge, t.stroke_secondary, t.stroke),
        },
        (false, Visual::Pressed) => Style { background: t.control_fill_pressed, foreground: t.text_secondary, border: Border::Solid(t.stroke) },
        (false, Visual::Disabled) => Style { background: t.control_fill_disabled, foreground: t.text_disabled, border: Border::Solid(t.stroke) },
        (true, Visual::Rest | Visual::PointerOver) => Style {
            background: if visual == Visual::Rest { t.accent } else { with_opacity(t.accent, 0.9) },
            foreground: t.text_on_accent,
            border: elevation(Edge::Bottom, t.stroke_on_accent_secondary, t.stroke_on_accent),
        },
        (true, Visual::Pressed) => Style { background: with_opacity(t.accent, 0.8), foreground: t.text_on_accent_secondary, border: Border::None },
        (true, Visual::Disabled) => Style { background: t.accent_disabled, foreground: t.text_on_accent_disabled, border: Border::None },
    }
}

/// `SubtleButton*` (Default and Light): the border repeats the fill, so the
/// `InnerBorderEdge` fill plus its 1-DIP ring read as one surface.
fn subtle_style(visual: Visual, t: &Theme) -> Style {
    let solid = |fill| Style { background: fill, foreground: t.text_primary, border: Border::Solid(fill) };
    match visual {
        Visual::Rest => Style { background: theme::SUBTLE_TRANSPARENT, foreground: t.text_primary, border: Border::None },
        Visual::PointerOver => solid(t.subtle_hover),
        Visual::Pressed => Style { foreground: t.text_secondary, ..solid(t.subtle_pressed) },
        Visual::Disabled => Style { background: theme::SUBTLE_TRANSPARENT, foreground: t.text_disabled, border: Border::None },
    }
}

/// `HighContrast` dictionaries of `Button_themeresources.xaml` and
/// `ToggleButton_themeresources.xaml` (`Checked*`; unchecked states equal the
/// default button), with each `SystemControl*Brush` resolved through the
/// framework's `generic.xaml`.
fn high_contrast_style(visual: Visual, look: Look, c: &SystemColors) -> Style {
    let style = |background, foreground, border| Style { background, foreground, border: Border::Solid(border) };
    match (look, visual) {
        (_, Visual::Disabled) => style(c.button_face, c.gray_text, c.gray_text),
        // SubtleButton's HighContrast entries equal the default button's.
        (Look::Default | Look::Subtle, Visual::Rest) => style(c.button_face, c.button_text, c.button_text),
        (Look::Default | Look::Subtle, Visual::PointerOver) => style(c.highlight_text, c.highlight, c.highlight),
        (Look::Default | Look::Subtle | Look::Checked, Visual::Pressed) => style(c.highlight_text, c.highlight, c.highlight_text),
        (Look::Checked, Visual::Rest) => style(c.highlight, c.highlight_text, c.highlight_text),
        (Look::Checked, Visual::PointerOver) => style(c.button_text, c.button_face, c.button_text),
        (Look::Accent, Visual::Rest) => style(c.highlight, c.highlight_text, c.highlight),
        (Look::Accent, Visual::PointerOver) => style(c.button_text, c.button_face, c.highlight),
        (Look::Accent, Visual::Pressed) => style(c.button_face, c.highlight, c.highlight),
    }
}

fn lerp(a: D2D1_COLOR_F, b: D2D1_COLOR_F, k: f32) -> D2D1_COLOR_F {
    let m = |x: f32, y: f32| x + (y - x) * k;
    D2D1_COLOR_F { r: m(a.r, b.r), g: m(a.g, b.g), b: m(a.b, b.b), a: m(a.a, b.a) }
}

fn inset(r: D2D_RECT_F, by: f32) -> D2D_RECT_F {
    rect(r.left + by, r.top + by, r.right - by, r.bottom - by)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::rect;
    use crate::theme::tests::HC;

    const INSIDE: (f32, f32) = (20.0, 16.0);
    const OUTSIDE: (f32, f32) = (200.0, 200.0);

    fn button() -> Button {
        Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), false)
    }

    fn focused(kind: FocusKind) -> Button {
        let mut b = button();
        b.handle(Input::Focus(kind));
        b
    }

    fn click_with(b: &mut Button, key: Key) -> (Visual, Response) {
        b.handle(Input::KeyDown(key));
        (b.visual(), b.handle(Input::KeyUp(key)))
    }

    #[test]
    fn hovering_should_show_pointer_over_and_redraw() {
        let mut b = button();
        let r = b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        assert_eq!((b.visual(), r.redraw), (Visual::PointerOver, true));
    }

    #[test]
    fn leaving_should_return_to_rest() {
        let mut b = button();
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        b.handle(Input::PointerLeave);
        assert_eq!(b.visual(), Visual::Rest);
    }

    #[test]
    fn press_should_capture_take_focus_and_show_pressed() {
        let mut b = button();
        let r = b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        assert_eq!((b.visual(), r.capture, r.take_focus), (Visual::Pressed, Some(true), true));
    }

    #[test]
    fn release_inside_after_press_should_click_and_release_capture() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let r = b.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        assert_eq!((r.clicked, r.capture, b.visual()), (true, Some(false), Visual::PointerOver));
    }

    #[test]
    fn press_outside_should_do_nothing() {
        let mut b = button();
        let r = b.handle(Input::PointerDown(OUTSIDE.0, OUTSIDE.1));
        assert_eq!((r, b.visual()), (Response::default(), Visual::Rest));
    }

    #[test]
    fn dragging_out_while_pressed_should_drop_pressed_and_cancel_click() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        b.handle(Input::PointerMove(OUTSIDE.0, OUTSIDE.1));
        let pressed_after_drag = b.visual() == Visual::Pressed;
        let r = b.handle(Input::PointerUp(OUTSIDE.0, OUTSIDE.1));
        assert_eq!((pressed_after_drag, r.clicked, r.capture), (false, false, Some(false)));
    }

    #[test]
    fn dragging_back_in_should_restore_pressed_and_click() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        b.handle(Input::PointerMove(OUTSIDE.0, OUTSIDE.1));
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        let visual = b.visual();
        assert_eq!((visual, b.handle(Input::PointerUp(INSIDE.0, INSIDE.1)).clicked), (Visual::Pressed, true));
    }

    #[test]
    fn edge_within_tolerance_should_count_as_inside() {
        // ButtonBase::IsValidPointerPosition allows 0.05 DIP outside the bounds.
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        b.handle(Input::PointerMove(80.04, 16.0));
        assert_eq!(b.visual(), Visual::Pressed);
    }

    #[test]
    fn space_should_press_on_key_down_and_click_on_key_up() {
        let mut b = focused(FocusKind::Keyboard);
        let (held, r) = click_with(&mut b, Key::Space);
        assert_eq!((held, r.clicked, b.visual()), (Visual::Pressed, true, Visual::Rest));
    }

    #[test]
    fn enter_should_behave_like_space() {
        let mut b = focused(FocusKind::Keyboard);
        let (held, r) = click_with(&mut b, Key::Enter);
        assert_eq!((held, r.clicked), (Visual::Pressed, true));
    }

    #[test]
    fn other_key_while_space_held_should_cancel_the_click() {
        let mut b = focused(FocusKind::Keyboard);
        b.handle(Input::KeyDown(Key::Space));
        b.handle(Input::KeyDown(Key::Other));
        let r = b.handle(Input::KeyUp(Key::Space));
        assert_eq!((r.clicked, b.visual()), (false, Visual::Rest));
    }

    #[test]
    fn keys_should_be_ignored_without_focus() {
        let mut b = button();
        let (held, r) = click_with(&mut b, Key::Space);
        assert_eq!((held, r.clicked), (Visual::Rest, false));
    }

    #[test]
    fn keys_should_be_ignored_while_pointer_is_captured() {
        let mut b = focused(FocusKind::Keyboard);
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        b.handle(Input::KeyDown(Key::Space));
        let r = b.handle(Input::KeyUp(Key::Space));
        // Key up while the left button is down never clicks (KeyUp checks m_bIsPointerLeftButtonDown).
        assert!(!r.clicked);
    }

    #[test]
    fn disabled_should_show_disabled_and_ignore_input() {
        let mut b = focused(FocusKind::Keyboard);
        b.set_enabled(false);
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let r = b.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        assert_eq!((b.visual(), r.clicked), (Visual::Disabled, false));
    }

    #[test]
    fn disabling_should_clear_pressed_and_pointer_over() {
        let mut b = button();
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        b.set_enabled(false);
        b.set_enabled(true);
        assert_eq!(b.visual(), Visual::Rest);
    }

    #[test]
    fn disabling_a_focused_button_should_drop_focus() {
        let mut b = focused(FocusKind::Keyboard);
        let r = b.set_enabled(false);
        b.set_enabled(true);
        // Re-enabling must not bring the keyboard ring back unasked.
        assert_eq!((r.lost_focus, b.focus_visible()), (true, false));
    }

    #[test]
    fn disabling_an_unfocused_button_should_not_report_lost_focus() {
        let mut b = button();
        assert!(!b.set_enabled(false).lost_focus);
    }

    #[test]
    fn blur_should_cancel_press_and_release_capture() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let r = b.handle(Input::Blur);
        let up = b.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        assert_eq!((r.capture, up.clicked, b.focus_visible()), (Some(false), false, false));
    }

    #[test]
    fn press_should_hold_the_capture() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        assert!(b.is_captured());
    }

    #[test]
    fn capture_lost_should_abandon_the_press_without_click_or_release() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let lost = b.handle(Input::CaptureLost);
        let up = b.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        // Capture already belongs to someone else: nothing to release, no click.
        assert_eq!((lost.capture, b.is_captured(), b.visual() == Visual::Pressed, up), (None, false, false, Response::default()));
    }

    #[test]
    fn focus_ring_should_show_for_keyboard_focus_only() {
        let keyboard = focused(FocusKind::Keyboard).focus_visible();
        let pointer = focused(FocusKind::Pointer).focus_visible();
        assert_eq!((keyboard, pointer), (true, false));
    }

    fn pressed(accent: bool) -> Button {
        let mut b = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), accent);
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        b
    }

    #[test]
    fn default_rest_should_use_control_fill_and_top_elevation_in_dark() {
        let s = button().style(&Theme::DARK);
        let t = Theme::DARK;
        let border = Border::Elevation { edge: Edge::Top, edge_color: t.stroke_secondary, rest: t.stroke };
        assert_eq!((s.background, s.foreground, s.border), (t.control_fill, t.text_primary, border));
    }

    #[test]
    fn default_elevation_should_sit_on_the_bottom_in_light() {
        let s = button().style(&Theme::LIGHT);
        assert!(matches!(s.border, Border::Elevation { edge: Edge::Bottom, .. }));
    }

    #[test]
    fn default_pressed_should_use_tertiary_fill_secondary_text_and_solid_stroke() {
        let (s, t) = (pressed(false).style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.background, s.foreground, s.border), (t.control_fill_pressed, t.text_secondary, Border::Solid(t.stroke)));
    }

    #[test]
    fn accent_pressed_should_use_accent_at_80_percent_and_no_border() {
        let (s, t) = (pressed(true).style(&Theme::DARK), Theme::DARK);
        let expected = crate::theme::with_opacity(t.accent, 0.8);
        assert_eq!((s.background, s.foreground, s.border), (expected, t.text_on_accent_secondary, Border::None));
    }

    #[test]
    fn accent_rest_elevation_should_be_flipped_in_both_themes() {
        let dark = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), true).style(&Theme::DARK);
        assert!(matches!(dark.border, Border::Elevation { edge: Edge::Bottom, .. }));
    }

    #[test]
    fn disabled_should_use_disabled_tokens() {
        let mut b = button();
        b.set_enabled(false);
        let (s, t) = (b.style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.background, s.foreground), (t.control_fill_disabled, t.text_disabled));
    }

    #[test]
    fn visual_change_should_start_the_background_transition() {
        let mut b = button();
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        assert!(b.animating());
    }

    /// (background, foreground, border) of `b` in the high contrast test theme.
    fn hc(b: &Button) -> (D2D1_COLOR_F, D2D1_COLOR_F, Border) {
        let s = b.style(&Theme::high_contrast(HC));
        (s.background, s.foreground, s.border)
    }

    fn hovered(accent: bool) -> Button {
        let mut b = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), accent);
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        b
    }

    #[test]
    fn high_contrast_default_rest_should_use_button_face_and_button_text() {
        assert_eq!(hc(&button()), (HC.button_face, HC.button_text, Border::Solid(HC.button_text)));
    }

    #[test]
    fn high_contrast_default_pointer_over_should_invert_to_highlight() {
        assert_eq!(hc(&hovered(false)), (HC.highlight_text, HC.highlight, Border::Solid(HC.highlight)));
    }

    #[test]
    fn high_contrast_default_pressed_should_border_with_highlight_text() {
        assert_eq!(hc(&pressed(false)), (HC.highlight_text, HC.highlight, Border::Solid(HC.highlight_text)));
    }

    #[test]
    fn high_contrast_accent_rest_should_fill_with_highlight() {
        let b = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), true);
        assert_eq!(hc(&b), (HC.highlight, HC.highlight_text, Border::Solid(HC.highlight)));
    }

    #[test]
    fn high_contrast_accent_pointer_over_should_fill_with_button_text() {
        assert_eq!(hc(&hovered(true)), (HC.button_text, HC.button_face, Border::Solid(HC.highlight)));
    }

    #[test]
    fn high_contrast_accent_pressed_should_fill_with_button_face() {
        assert_eq!(hc(&pressed(true)), (HC.button_face, HC.highlight, Border::Solid(HC.highlight)));
    }

    #[test]
    fn high_contrast_disabled_should_use_gray_text_for_text_and_border() {
        let mut b = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), true);
        b.set_enabled(false);
        assert_eq!(hc(&b), (HC.button_face, HC.gray_text, Border::Solid(HC.gray_text)));
    }

    fn toggle(checked: bool) -> Button {
        Button::toggle("Wi-Fi", rect(0.0, 0.0, 80.0, 32.0), checked)
    }

    fn click(b: &mut Button) -> Response {
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        b.handle(Input::PointerUp(INSIDE.0, INSIDE.1))
    }

    #[test]
    fn enter_should_do_nothing_when_return_is_not_accepted() {
        let mut b = toggle(false);
        b.set_accepts_return(false);
        b.handle(Input::Focus(FocusKind::Keyboard));
        b.handle(Input::KeyDown(Key::Enter));
        let up = b.handle(Input::KeyUp(Key::Enter));
        assert_eq!((b.is_checked(), up.clicked), (Some(false), false));
    }

    #[test]
    fn three_state_click_should_cycle_unchecked_checked_indeterminate_unchecked() {
        let mut b = toggle(false);
        b.set_three_state(true);
        let mut seen = Vec::new();
        for _ in 0..3 {
            click(&mut b);
            seen.push((b.is_checked(), b.is_indeterminate()));
        }
        assert_eq!(seen, [(Some(true), false), (Some(false), true), (Some(false), false)]);
    }

    #[test]
    fn two_state_click_from_checked_should_uncheck() {
        let mut b = toggle(true);
        click(&mut b);
        assert_eq!((b.is_checked(), b.is_indeterminate()), (Some(false), false));
    }

    #[test]
    fn set_checked_should_leave_indeterminate() {
        let mut b = toggle(false);
        b.set_indeterminate();
        b.set_checked(true);
        assert_eq!((b.is_checked(), b.is_indeterminate()), (Some(true), false));
    }

    #[test]
    fn set_indeterminate_should_report_a_redraw() {
        let mut b = toggle(true);
        assert_eq!((b.set_indeterminate().redraw, b.is_indeterminate()), (true, true));
    }

    #[test]
    fn set_checked_should_change_the_state_without_a_click() {
        let mut b = toggle(false);
        let r = b.set_checked(true);
        assert_eq!((b.is_checked(), r.clicked, r.redraw), (Some(true), false, true));
    }

    #[test]
    fn plain_button_should_have_no_checked_state() {
        assert_eq!(button().is_checked(), None);
    }

    #[test]
    fn toggle_click_should_check_and_report_the_click() {
        let mut b = toggle(false);
        let r = click(&mut b);
        assert_eq!((b.is_checked(), r.clicked), (Some(true), true));
    }

    #[test]
    fn toggle_second_click_should_uncheck() {
        let mut b = toggle(false);
        click(&mut b);
        click(&mut b);
        assert_eq!(b.is_checked(), Some(false));
    }

    #[test]
    fn toggle_keyboard_click_should_check() {
        let mut b = toggle(false);
        b.handle(Input::Focus(FocusKind::Keyboard));
        b.handle(Input::KeyDown(Key::Space));
        b.handle(Input::KeyUp(Key::Space));
        assert_eq!(b.is_checked(), Some(true));
    }

    #[test]
    fn invoke_should_toggle_and_click() {
        let mut b = toggle(false);
        let r = b.invoke();
        assert_eq!((b.is_checked(), r.clicked, r.redraw), (Some(true), true, true));
    }

    #[test]
    fn invoke_should_click_a_plain_button_without_checking_it() {
        let mut b = button();
        let r = b.invoke();
        assert_eq!((b.is_checked(), r.clicked), (None, true));
    }

    #[test]
    fn invoke_should_do_nothing_while_disabled() {
        let mut b = toggle(false);
        b.set_enabled(false);
        assert_eq!((b.invoke(), b.is_checked()), (Response::default(), Some(false)));
    }

    #[test]
    fn checked_should_use_the_accent_brushes_outside_high_contrast() {
        let accent = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), true).style(&Theme::DARK);
        assert_eq!(toggle(true).style(&Theme::DARK), accent);
    }

    fn subtle() -> Button {
        Button::subtle("Iniciar", rect(0.0, 0.0, 80.0, 32.0))
    }

    #[test]
    fn subtle_rest_should_have_a_transparent_fill_and_no_border() {
        let (s, t) = (subtle().style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.background, s.foreground, s.border), (theme::SUBTLE_TRANSPARENT, t.text_primary, Border::None));
    }

    #[test]
    fn subtle_pointer_over_should_fill_and_border_with_subtle_secondary() {
        let mut b = subtle();
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        let (s, t) = (b.style(&Theme::LIGHT), Theme::LIGHT);
        assert_eq!((s.background, s.foreground, s.border), (t.subtle_hover, t.text_primary, Border::Solid(t.subtle_hover)));
    }

    #[test]
    fn subtle_pressed_should_use_subtle_tertiary_and_secondary_text() {
        let mut b = subtle();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let (s, t) = (b.style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.background, s.foreground, s.border), (t.subtle_pressed, t.text_secondary, Border::Solid(t.subtle_pressed)));
    }

    #[test]
    fn subtle_disabled_should_be_transparent_with_disabled_text() {
        let mut b = subtle();
        b.set_enabled(false);
        let (s, t) = (b.style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.background, s.foreground, s.border), (theme::SUBTLE_TRANSPARENT, t.text_disabled, Border::None));
    }

    #[test]
    fn high_contrast_subtle_should_match_the_default_button() {
        let mut b = subtle();
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        assert_eq!(hc(&b), hc(&hovered(false)));
    }

    #[test]
    fn checked_background_should_reach_the_outer_border_edge() {
        assert!(toggle(true).outer_background());
    }

    #[test]
    fn checked_disabled_background_should_stay_inside_the_border() {
        let mut b = toggle(true);
        b.set_enabled(false);
        assert!(!b.outer_background());
    }

    #[test]
    fn accent_disabled_background_should_still_reach_the_outer_edge() {
        let mut b = Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), true);
        b.set_enabled(false);
        assert!(b.outer_background());
    }

    #[test]
    fn checking_should_start_the_background_transition() {
        let mut b = toggle(false);
        b.invoke();
        assert!(b.animating());
    }

    #[test]
    fn high_contrast_checked_rest_should_fill_with_highlight_and_border_with_highlight_text() {
        assert_eq!(hc(&toggle(true)), (HC.highlight, HC.highlight_text, Border::Solid(HC.highlight_text)));
    }

    #[test]
    fn high_contrast_checked_pointer_over_should_fill_with_button_text() {
        let mut b = toggle(true);
        b.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        assert_eq!(hc(&b), (HC.button_text, HC.button_face, Border::Solid(HC.button_text)));
    }

    #[test]
    fn high_contrast_checked_pressed_should_fill_with_highlight_text() {
        let mut b = toggle(true);
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        assert_eq!(hc(&b), (HC.highlight_text, HC.highlight, Border::Solid(HC.highlight_text)));
    }

    #[test]
    fn pointer_press_should_hide_a_keyboard_focus_ring() {
        let mut b = focused(FocusKind::Keyboard);
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        assert!(!b.focus_visible());
    }
}
