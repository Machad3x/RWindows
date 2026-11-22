//! WinUI `RadioButton`, following `RadioButton_themeresources.xaml`.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

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
    focused: bool,
}

impl RadioButton {
    /// Creates a radio button, checked or not.
    pub fn new(label: impl Into<String>, checked: bool) -> Self {
        Self { label: label.into(), checked, enabled: true, hot: false, pressed: false, focused: false }
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
                self.pressed = self.hot;
            }
            Input::PointerUp(x, y) => {
                let clicked = self.pressed && self.hit(x, y);
                self.pressed = false;
                return clicked;
            }
            Input::KeyDown(Key::Space) if self.focused => self.pressed = true,
            Input::KeyUp(Key::Space) if self.pressed => {
                self.pressed = false;
                return true;
            }
            Input::KeyDown(_) => self.pressed = false,
            Input::Focus(FocusKind::Keyboard) => self.focused = true,
            Input::Blur | Input::PointerLeave | Input::CaptureLost => {
                self.focused = false;
                self.hot = false;
                self.pressed = false;
            }
            _ => {}
        }
        false
    }

    fn hit(&self, x: f32, y: f32) -> bool {
        // The template's root grid is the hit target, not just the ellipse.
        x >= 0.0 && y >= 0.0 && x <= MIN_WIDTH && y <= ROW_HEIGHT
    }

    /// Draws the ellipse and the left-aligned content.
    pub fn draw(&self, s: &Surface, t: &Theme, f: &Formats, r: D2D_RECT_F) {
        let cy = r.top + ELLIPSE / 2.0;
        let cx = r.left + ELLIPSE / 2.0;
        let ellipse = rect(cx - ELLIPSE / 2.0, cy - ELLIPSE / 2.0, cx + ELLIPSE / 2.0, cy + ELLIPSE / 2.0);
        let (fill, stroke) = if !self.enabled {
            (t.control_alt_fill_disabled, t.control_strong_stroke_disabled)
        } else if self.checked {
            (t.accent, t.accent)
        } else {
            (t.control_alt_fill_secondary, t.control_strong_stroke)
        };
        s.fill_round(ellipse, ELLIPSE / 2.0, fill);
        s.stroke_round(ellipse, ELLIPSE / 2.0, 1.0, stroke);
        if self.checked {
            let g = if self.pressed { GLYPH[2] } else if self.hot { GLYPH[1] } else { GLYPH[0] };
            let glyph = rect(cx - g / 2.0, cy - g / 2.0, cx + g / 2.0, cy + g / 2.0);
            s.fill_round(glyph, g / 2.0, t.text_on_accent);
        }
        let text = rect(r.left + ELLIPSE + PADDING.0, r.top + PADDING.1, r.right, r.bottom);
        s.text(&self.label, &f.body, text,
            if self.enabled { t.text_primary } else { t.text_disabled });
        if self.focused {
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
