//! WinUI `CheckBox` (`DefaultCheckBoxStyle`), two-state, for raw Win32.
//!
//! Input is the `ToggleButton` machine of [`Button`] with Enter ignored
//! (`CheckBox::Initialize` calls `SetAcceptsReturn(false)`), plus numpad `+`/`-`
//! (`CheckBox::OnKeyDownInternal`). Brushes: `CheckBox_themeresources.xaml`.

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::core::Result;

use crate::button::{Button, Input, Key, Response, Visual};
use crate::gfx::{Surface, rect};
use crate::theme::{self, Theme};

/// `CheckBoxSize`.
const BOX: f32 = 20.0;
/// `CheckBoxGlyphSize`.
pub const GLYPH_SIZE: f32 = 12.0;
/// `CheckBoxCheckedGlyph` (Segoe Fluent Icons `CheckMark`).
const CHECK_GLYPH: &str = "\u{E73E}";
/// `CheckBoxPadding` = 8,5,0,0 (content after the box).
const PADDING: (f32, f32) = (8.0, 5.0);
/// `CheckBoxMinWidth` / `CheckBoxHeight` (MinHeight).
const MIN_SIZE: (f32, f32) = (120.0, 32.0);
/// `CheckBoxFocusVisualMargin` = -7,-3,-7,-3.
const FOCUS_MARGIN: (f32, f32) = (7.0, 3.0);

/// Brushes of one combined state (`CheckBox*` resources).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CheckStyle {
    /// `CheckBoxForeground*` (content text).
    pub content: D2D1_COLOR_F,
    /// `CheckBoxCheckBackgroundFill*` (`NormalRectangle` fill).
    pub box_fill: D2D1_COLOR_F,
    /// `CheckBoxCheckBackgroundStroke*` (1-DIP `NormalRectangle` stroke).
    pub box_stroke: D2D1_COLOR_F,
    /// `CheckBoxCheckGlyphForeground*`.
    pub glyph: D2D1_COLOR_F,
    /// `CheckBoxBackground*` (`RootGrid`): `SystemColorWindowColor` in high
    /// contrast, transparent (`None`) otherwise.
    pub background: Option<D2D1_COLOR_F>,
}

/// A check box with text content.
#[derive(Clone, Debug)]
pub struct CheckBox {
    inner: Button,
}

impl CheckBox {
    /// Enabled check box at `rect` (size from [`CheckBox::desired_size`]).
    pub fn new(label: &str, rect: D2D_RECT_F, checked: bool) -> Self {
        let mut inner = Button::toggle(label, rect, checked);
        inner.set_accepts_return(false);
        Self { inner }
    }

    /// Auto size: box + 8 + text, text 5 DIPs from the top; at least 120 x 32.
    pub fn desired_size(surface: &Surface, format: &IDWriteTextFormat, label: &str) -> Result<(f32, f32)> {
        let (w, h) = surface.measure(label, format)?;
        let width = (BOX + PADDING.0 + w).max(MIN_SIZE.0);
        let height = (PADDING.1 + h).max(BOX).max(MIN_SIZE.1);
        Ok((width.ceil(), height.ceil()))
    }

    /// Bounds in DIPs.
    pub fn rect(&self) -> D2D_RECT_F {
        self.inner.rect
    }

    /// Content text (also the UI Automation name).
    pub fn label(&self) -> &str {
        &self.inner.label
    }

    /// `IsChecked`.
    pub fn is_checked(&self) -> bool {
        self.inner.is_checked() == Some(true)
    }

    /// Current visual state.
    pub fn visual(&self) -> Visual {
        self.inner.visual()
    }

    /// True while this check box holds the mouse capture.
    pub fn is_captured(&self) -> bool {
        self.inner.is_captured()
    }

    /// True when the focus rectangle must be drawn (keyboard focus only).
    pub fn focus_visible(&self) -> bool {
        self.inner.focus_visible()
    }

    /// Enables or disables the check box.
    pub fn set_enabled(&mut self, enabled: bool) -> Response {
        self.inner.set_enabled(enabled)
    }

    /// `Toggle()` (UI Automation Toggle pattern); does nothing while disabled.
    pub fn invoke(&mut self) -> Response {
        self.inner.invoke()
    }

    /// Feeds one input event; `Response::clicked` means `IsChecked` changed.
    pub fn handle(&mut self, input: Input) -> Response {
        let target = match input {
            Input::KeyDown(Key::Add) => Some(true),
            Input::KeyDown(Key::Subtract) => Some(false),
            _ => None,
        };
        let enabled = self.visual() != Visual::Disabled;
        if let Some(checked) = target.filter(|_| enabled && self.inner.has_focus()) {
            // OnKeyDownInternal: put_IsPressed(FALSE) (also during a mouse press: the
            // release then skips OnClick), then put_IsChecked.
            let was = self.is_checked();
            let base = self.inner.handle(input);
            let cancel = self.inner.cancel_press();
            let set = self.inner.set_checked(checked);
            let redraw = base.redraw || cancel.redraw || set.redraw;
            return Response { redraw, clicked: was != checked, ..base };
        }
        self.inner.handle(input)
    }

    /// Always false: every CheckBox state change is a discrete setter (no transitions).
    pub fn animating(&self) -> bool {
        false
    }

    /// `NormalRectangle`: 20 x 20, vertically centered in the control.
    pub fn check_box(&self) -> D2D_RECT_F {
        let r = self.rect();
        let top = ((r.top + r.bottom) - BOX) / 2.0;
        rect(r.left, top, r.left + BOX, top + BOX)
    }

    /// Target brushes for the current state.
    pub fn style(&self, t: &Theme) -> CheckStyle {
        style_of(self.visual(), self.is_checked(), t)
    }

    /// Draws box, check glyph (when checked), content and the focus rectangle.
    /// `text` is a Body format (left, top); `icon` is [`Surface::icon_format`] at [`GLYPH_SIZE`].
    pub fn draw(&self, s: &Surface, t: &Theme, text: &IDWriteTextFormat, icon: &IDWriteTextFormat) -> Result<()> {
        let style = self.style(t);
        let r = self.rect();
        let radius = theme::CONTROL_RADIUS;
        if let Some(bg) = style.background {
            s.fill_round(r, radius, bg);
        }
        let b = self.check_box();
        s.fill_round(b, radius, style.box_fill);
        s.stroke_round(b, radius, 1.0, style.box_stroke);
        // The AnimatedIcon shows nothing in its Off/Indeterminate-less states; its
        // FontIcon fallback has no "off" frame, so the glyph is drawn only when checked.
        if self.is_checked() {
            // CheckGlyph Margin 0,1,0,-1: one DIP lower than the box.
            s.text(CHECK_GLYPH, icon, rect(b.left, b.top + 1.0, b.right, b.bottom + 1.0), style.glyph);
        }
        let content = rect(b.right + PADDING.0, r.top + PADDING.1, r.right, r.bottom);
        s.text(self.label(), text, content, style.content);
        if self.focus_visible() {
            let outer = rect(r.left - FOCUS_MARGIN.0, r.top - FOCUS_MARGIN.1, r.right + FOCUS_MARGIN.0, r.bottom + FOCUS_MARGIN.1);
            let rr = radius + FOCUS_MARGIN.1;
            s.stroke_round(outer, rr, 2.0, t.focus_outer);
            let ring = s.snap_len(2.0);
            s.stroke_round(rect(outer.left + ring, outer.top + ring, outer.right - ring, outer.bottom - ring), rr - 2.0, 1.0, t.focus_inner);
        }
        Ok(())
    }
}

fn style_of(visual: Visual, checked: bool, t: &Theme) -> CheckStyle {
    use Visual::{Disabled, PointerOver, Pressed, Rest};
    if let Some(c) = t.high_contrast {
        let (content, box_fill, box_stroke, glyph) = match (checked, visual) {
            (false, Rest) => (c.button_text, c.button_face, c.button_text, c.button_face),
            (false, PointerOver) => (c.button_text, c.highlight_text, c.highlight, c.highlight),
            (false, Pressed) => (c.button_text, c.highlight, c.highlight, c.highlight_text),
            (false, Disabled) => (c.gray_text, c.window, c.gray_text, c.gray_text),
            (true, Rest) => (c.button_text, c.highlight, c.highlight, c.highlight_text),
            (true, PointerOver) => (c.button_text, c.button_text, c.button_text, c.button_face),
            (true, Pressed) => (c.button_text, c.button_face, c.button_face, c.button_text),
            (true, Disabled) => (c.gray_text, c.gray_text, c.gray_text, c.window),
        };
        return CheckStyle { content, box_fill, box_stroke, glyph, background: Some(c.window) };
    }
    let accent = |opacity| theme::with_opacity(t.accent, opacity);
    let (content, box_fill, box_stroke, glyph) = match (checked, visual) {
        (false, Rest) => (t.text_primary, t.control_alt_fill_secondary, t.control_strong_stroke, t.text_on_accent),
        (false, PointerOver) => (t.text_primary, t.control_alt_fill_tertiary, t.control_strong_stroke, t.text_on_accent),
        (false, Pressed) => (t.text_primary, t.control_alt_fill_quarternary, t.control_strong_stroke_disabled, t.text_on_accent),
        (false, Disabled) => (t.text_disabled, t.control_alt_fill_disabled, t.control_strong_stroke_disabled, t.text_on_accent_disabled),
        (true, Rest) => (t.text_primary, accent(1.0), accent(1.0), t.text_on_accent),
        (true, PointerOver) => (t.text_primary, accent(0.9), accent(0.9), t.text_on_accent),
        (true, Pressed) => (t.text_primary, accent(0.8), accent(0.8), t.text_on_accent_secondary),
        (true, Disabled) => (t.text_disabled, t.accent_disabled, t.control_strong_stroke_disabled, t.text_on_accent_disabled),
    };
    CheckStyle { content, box_fill, box_stroke, glyph, background: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::button::FocusKind;
    use crate::theme::tests::HC;

    fn check_box(checked: bool) -> CheckBox {
        CheckBox::new("Lembrar", rect(0.0, 0.0, 120.0, 32.0), checked)
    }

    const INSIDE: (f32, f32) = (60.0, 16.0);

    fn focused(checked: bool) -> CheckBox {
        let mut c = check_box(checked);
        c.handle(Input::Focus(FocusKind::Keyboard));
        c
    }

    #[test]
    fn click_should_check() {
        let mut c = check_box(false);
        c.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let r = c.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        assert_eq!((c.is_checked(), r.clicked), (true, true));
    }

    #[test]
    fn space_should_toggle_but_enter_should_not() {
        let mut c = focused(false);
        c.handle(Input::KeyDown(Key::Enter));
        c.handle(Input::KeyUp(Key::Enter));
        let enter = c.is_checked();
        c.handle(Input::KeyDown(Key::Space));
        c.handle(Input::KeyUp(Key::Space));
        assert_eq!((enter, c.is_checked()), (false, true));
    }

    #[test]
    fn numpad_plus_should_check_and_report_the_change() {
        let mut c = focused(false);
        let r = c.handle(Input::KeyDown(Key::Add));
        assert_eq!((c.is_checked(), r.clicked), (true, true));
    }

    #[test]
    fn numpad_plus_on_a_checked_box_should_change_nothing() {
        let mut c = focused(true);
        assert!(!c.handle(Input::KeyDown(Key::Add)).clicked);
    }

    #[test]
    fn numpad_minus_should_uncheck() {
        let mut c = focused(true);
        c.handle(Input::KeyDown(Key::Subtract));
        assert!(!c.is_checked());
    }

    #[test]
    fn numpad_plus_should_cancel_a_held_space() {
        let mut c = focused(true);
        c.handle(Input::KeyDown(Key::Space));
        c.handle(Input::KeyDown(Key::Subtract));
        // put_IsPressed(FALSE): the Space release no longer clicks.
        c.handle(Input::KeyUp(Key::Space));
        assert_eq!((c.is_checked(), c.visual()), (false, Visual::Rest));
    }

    #[test]
    fn numpad_plus_during_a_mouse_press_should_cancel_the_click() {
        let mut c = check_box(false);
        c.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        c.handle(Input::KeyDown(Key::Add));
        // put_IsPressed(FALSE): the release no longer runs OnClick.
        let up = c.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        assert_eq!((c.is_checked(), up.clicked), (true, false));
    }

    #[test]
    fn numpad_keys_should_be_ignored_while_disabled() {
        let mut c = focused(false);
        c.set_enabled(false);
        c.handle(Input::KeyDown(Key::Add));
        assert!(!c.is_checked());
    }

    #[test]
    fn box_should_be_20_square_vertically_centered() {
        let b = check_box(false).check_box();
        assert_eq!((b.left, b.top, b.right, b.bottom), (0.0, 6.0, 20.0, 26.0));
    }

    #[test]
    fn unchecked_rest_should_use_control_alt_fill_and_strong_stroke() {
        let (s, t) = (check_box(false).style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.content, s.box_fill, s.box_stroke, s.background),
            (t.text_primary, t.control_alt_fill_secondary, t.control_strong_stroke, None));
    }

    #[test]
    fn unchecked_pressed_should_use_quarternary_fill_and_disabled_stroke() {
        let mut c = check_box(false);
        c.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let (s, t) = (c.style(&Theme::LIGHT), Theme::LIGHT);
        assert_eq!((s.box_fill, s.box_stroke), (t.control_alt_fill_quarternary, t.control_strong_stroke_disabled));
    }

    #[test]
    fn checked_pointer_over_should_use_accent_secondary_for_fill_and_stroke() {
        let mut c = check_box(true);
        c.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        let (s, t) = (c.style(&Theme::DARK), Theme::DARK);
        let accent = theme::with_opacity(t.accent, 0.9);
        assert_eq!((s.box_fill, s.box_stroke, s.glyph), (accent, accent, t.text_on_accent));
    }

    #[test]
    fn checked_pressed_glyph_should_use_text_on_accent_secondary() {
        let mut c = check_box(true);
        c.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let (s, t) = (c.style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.box_fill, s.glyph), (theme::with_opacity(t.accent, 0.8), t.text_on_accent_secondary));
    }

    #[test]
    fn checked_disabled_should_use_disabled_tokens() {
        let mut c = check_box(true);
        c.set_enabled(false);
        let (s, t) = (c.style(&Theme::DARK), Theme::DARK);
        assert_eq!((s.content, s.box_fill, s.box_stroke, s.glyph),
            (t.text_disabled, t.accent_disabled, t.control_strong_stroke_disabled, t.text_on_accent_disabled));
    }

    #[test]
    fn high_contrast_checked_pointer_over_should_invert_the_box() {
        let mut c = check_box(true);
        c.handle(Input::PointerMove(INSIDE.0, INSIDE.1));
        let s = c.style(&Theme::high_contrast(HC));
        assert_eq!((s.content, s.box_fill, s.box_stroke, s.glyph, s.background),
            (HC.button_text, HC.button_text, HC.button_text, HC.button_face, Some(HC.window)));
    }

    #[test]
    fn high_contrast_unchecked_pressed_should_fill_with_highlight() {
        let mut c = check_box(false);
        c.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let s = c.style(&Theme::high_contrast(HC));
        assert_eq!((s.box_fill, s.box_stroke), (HC.highlight, HC.highlight));
    }
}
