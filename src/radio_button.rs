//! WinUI `RadioButton`, following `RadioButton_themeresources.xaml`.

use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D_RECT_F};

use crate::button::{FocusKind, Input, Key};
use crate::gfx::{Surface, rect};
use crate::panel::Formats;
use crate::theme::Theme;

/// `RadioButtonOuterEllipse*` size and the 32 DIP row of the template grid.
pub const ELLIPSE: f32 = 20.0;
/// Height of the template's outer grid row.
pub const ROW_HEIGHT: f32 = 32.0;
/// `DefaultRadioButtonStyle.MinWidth`.
pub const MIN_WIDTH: f32 = 120.0;
/// `DefaultRadioButtonStyle.Padding` = 8,6,0,0.
pub const PADDING: (f32, f32) = (8.0, 6.0);
/// `RadioButtonCheckGlyphSize`, PointerOver and Pressed sizes.
pub const GLYPH: [f32; 3] = [12.0, 14.0, 10.0];

/// One WinUI radio button.
#[derive(Clone, Debug, PartialEq)]
pub struct RadioButton {
    /// Content text.
    pub label: String,
    /// `IsChecked`.
    pub checked: bool,
    /// `IsEnabled`.
    pub enabled: bool,
    hot: bool,
    pressed: bool,
    focus: Option<FocusKind>,
    /// Size of the hit area, set by the caller to match the drawn rect.
    hit_size: (f32, f32),
}

impl RadioButton {
    /// Creates a radio button, checked or not.
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self {
            label: label.into(), checked, enabled: true, hot: false, pressed: false, focus: None,
            hit_size: (MIN_WIDTH, ROW_HEIGHT),
        }
    }

    /// True when the focus rectangle must be drawn (keyboard focus only).
    pub fn focus_visible(&self) -> bool {
        self.focus == Some(FocusKind::Keyboard)
    }

    /// Sets the hit area (width, height) to the size this radio is drawn at.
    pub fn set_hit_size(&mut self, width: f32, height: f32) {
        self.hit_size = (width.max(0.0), height.max(0.0));
    }

    /// Sets `IsChecked` directly (used by [`check_one`]).
    pub fn set_checked(&mut self, checked: bool) {
        self.checked = checked;
    }

    /// Handles one input event; returns true when it was clicked.
    pub fn handle(&mut self, input: Input) -> bool {
        if !self.enabled {
            return false;
        }
        match input {
            Input::PointerMove(x, y) => self.hot = self.hit(x, y),
            Input::PointerDown(x, y) => {
                self.hot = self.hit(x, y);
                self.focus = self.hot.then_some(FocusKind::Pointer);
                self.pressed = self.hot;
            }
            Input::PointerUp(x, y) => {
                let clicked = self.pressed && self.hit(x, y);
                self.pressed = false;
                return clicked;
            }
            Input::KeyDown(Key::Space) if self.focus.is_some() => self.pressed = true,
            Input::KeyUp(Key::Space) if self.pressed => {
                self.pressed = false;
                return true;
            }
            Input::KeyDown(_) => self.pressed = false,
            Input::Focus(kind) => self.focus = Some(kind),
            // Only a real focus loss clears the focus ring; leaving the pointer
            // or losing capture must not, like ButtonBase.
            Input::Blur => {
                self.focus = None;
                self.hot = false;
                self.pressed = false;
            }
            Input::PointerLeave | Input::CaptureLost => {
                self.hot = false;
                self.pressed = false;
            }
            _ => {}
        }
        false
    }

    fn hit(&self, x: f32, y: f32) -> bool {
        // The template's root grid is the hit target, not just the ellipse.
        x >= 0.0 && y >= 0.0 && x <= self.hit_size.0 && y <= self.hit_size.1
    }

    /// Target brushes for the current state. High contrast follows the
    /// CheckBox: Highlight for the selected ellipse, ButtonFace when not.
    pub fn style(&self, t: &Theme) -> (D2D1_COLOR_F, D2D1_COLOR_F, D2D1_COLOR_F) {
        if let Some(c) = t.high_contrast {
            return if !self.enabled {
                (c.button_face, c.gray_text, c.gray_text)
            } else if self.checked {
                if self.hot {
                    (c.highlight_text, c.highlight, c.highlight)
                } else {
                    (c.highlight, c.highlight, c.highlight_text)
                }
            } else if self.hot {
                (c.highlight, c.highlight, c.window_text)
            } else {
                (c.button_face, c.button_text, c.window_text)
            };
        }
        if !self.enabled {
            (t.control_alt_fill_disabled, t.control_strong_stroke_disabled, t.text_disabled)
        } else if self.checked {
            (t.accent, t.accent, t.text_on_accent)
        } else {
            (t.control_alt_fill_secondary, t.control_strong_stroke, t.text_primary)
        }
    }

    /// Draws the ellipse and the left-aligned content.
    pub fn draw(&self, s: &Surface, t: &Theme, f: &Formats, r: D2D_RECT_F) {
        let cy = r.top + ELLIPSE / 2.0;
        let cx = r.left + ELLIPSE / 2.0;
        let ellipse = rect(cx - ELLIPSE / 2.0, cy - ELLIPSE / 2.0, cx + ELLIPSE / 2.0, cy + ELLIPSE / 2.0);
        let (fill, stroke, glyph_color) = self.style(t);
        s.fill_round(ellipse, ELLIPSE / 2.0, fill);
        s.stroke_round(ellipse, ELLIPSE / 2.0, 1.0, stroke);
        if self.checked {
            let g = if self.pressed { GLYPH[2] } else if self.hot { GLYPH[1] } else { GLYPH[0] };
            let glyph = rect(cx - g / 2.0, cy - g / 2.0, cx + g / 2.0, cy + g / 2.0);
            s.fill_round(glyph, g / 2.0, glyph_color);
        }
        let text = rect(r.left + ELLIPSE + PADDING.0, r.top + PADDING.1, r.right, r.bottom);
        s.text(&self.label, &f.body, text,
            if self.enabled { t.text_primary } else { t.text_disabled });
        if self.focus_visible() {
            let ring = rect(r.left - 7.0, r.top - 3.0, r.right + 7.0, r.bottom + 3.0);
            s.stroke_round(ring, 4.0, 2.0, t.focus_outer);
        }
    }
}

/// Enforces single selection, like a `RadioButtons` group.
pub fn check_one(group: &mut [RadioButton], index: usize) {
    for (i, item) in group.iter_mut().enumerate() {
        item.set_checked(i == index);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_click_should_toggle() {
        let mut r = RadioButton::new("Claro", false);
        assert!(!r.handle(Input::PointerDown(10.0, 10.0)));
        assert!(r.handle(Input::PointerUp(10.0, 10.0)));
        assert!(!r.handle(Input::PointerUp(200.0, 10.0)));
    }

    #[test]
    fn space_needs_focus_and_a_matching_key_up() {
        let mut r = RadioButton::new("Escuro", false);
        r.handle(Input::KeyDown(Key::Space));
        assert!(!r.handle(Input::KeyUp(Key::Space)));
        r.handle(Input::Focus(FocusKind::Keyboard));
        r.handle(Input::KeyDown(Key::Space));
        assert!(r.handle(Input::KeyUp(Key::Space)));
    }

    #[test]
    fn hit_area_should_follow_the_size_set_by_the_caller() {
        let mut r = RadioButton::new("A", false);
        r.set_hit_size(60.0, 24.0);
        r.handle(Input::PointerDown(100.0, 10.0));
        assert!(!r.focus_visible());
        r.handle(Input::PointerDown(10.0, 10.0));
        assert!(r.focus.is_some());
    }

    #[test]
    fn pointer_press_should_take_focus_and_mouse_leave_should_not_drop_it() {
        let mut r = RadioButton::new("A", false);
        r.handle(Input::PointerDown(4.0, 4.0));
        assert!(r.focus.is_some());
        r.handle(Input::PointerLeave);
        assert!(r.focus.is_some());
        r.handle(Input::Blur);
        assert!(r.focus.is_none());
    }

    #[test]
    fn only_keyboard_focus_should_draw_the_ring() {
        let mut r = RadioButton::new("A", false);
        r.handle(Input::PointerDown(4.0, 4.0));
        assert!(!r.focus_visible());
        r.handle(Input::Focus(FocusKind::Keyboard));
        assert!(r.focus_visible());
        r.handle(Input::Blur);
        r.handle(Input::Focus(FocusKind::Pointer));
        assert!(!r.focus_visible());
    }

    #[test]
    fn high_contrast_should_use_highlight_for_the_checked_ellipse() {
        use crate::theme::tests::HC;
        let mut r = RadioButton::new("A", true);
        assert_eq!(r.style(&Theme::high_contrast(HC)), (HC.highlight, HC.highlight, HC.highlight_text));
        r.handle(Input::PointerMove(4.0, 4.0));
        assert_eq!(r.style(&Theme::high_contrast(HC)), (HC.highlight_text, HC.highlight, HC.highlight));
    }

    #[test]
    fn disabled_radio_should_ignore_input() {
        let mut r = RadioButton::new("Off", false);
        r.enabled = false;
        assert!(!r.handle(Input::PointerDown(4.0, 4.0)));
        assert!(!r.handle(Input::PointerUp(4.0, 4.0)));
    }

    #[test]
    fn check_one_should_leave_exactly_one_checked() {
        let mut group = vec![RadioButton::new("A", true), RadioButton::new("B", false)];
        check_one(&mut group, 1);
        assert_eq!((group[0].checked, group[1].checked), (false, true));
    }
}
