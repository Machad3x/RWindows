//! WinUI `Button` (`DefaultButtonStyle` / `AccentButtonStyle`) for raw Win32.
//!
//! Input rules follow WinUI's `ButtonBase` with the default `ClickMode.Release`
//! (`ButtonBase_Partial.cpp`, `ButtonBaseKeyProcess.h`).

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;

/// Keys a button reacts to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Space bar.
    Space,
    /// Enter.
    Enter,
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
        }
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
        if !enabled {
            self.left_down = false;
            self.key_down = false;
        }
        Response { capture, ..self.changed(before) }
    }

    /// Feeds one input event.
    pub fn handle(&mut self, input: Input) -> Response {
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
                Key::Other if self.key_down => {
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
        Response { clicked: r.clicked, capture: r.capture, take_focus: r.take_focus, ..self.changed(before) }
    }

    fn contains(&self, x: f32, y: f32, tolerance: f32) -> bool {
        let r = self.rect;
        x >= r.left - tolerance && x <= r.right + tolerance && y >= r.top - tolerance && y <= r.bottom + tolerance
    }

    fn snapshot(&self) -> (Visual, bool) {
        (self.visual(), self.focus_visible())
    }

    fn changed(&self, before: (Visual, bool)) -> Response {
        Response { redraw: self.snapshot() != before, ..Response::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::rect;

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
    fn blur_should_cancel_press_and_release_capture() {
        let mut b = button();
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        let r = b.handle(Input::Blur);
        let up = b.handle(Input::PointerUp(INSIDE.0, INSIDE.1));
        assert_eq!((r.capture, up.clicked, b.focus_visible()), (Some(false), false, false));
    }

    #[test]
    fn focus_ring_should_show_for_keyboard_focus_only() {
        let keyboard = focused(FocusKind::Keyboard).focus_visible();
        let pointer = focused(FocusKind::Pointer).focus_visible();
        assert_eq!((keyboard, pointer), (true, false));
    }

    #[test]
    fn pointer_press_should_hide_a_keyboard_focus_ring() {
        let mut b = focused(FocusKind::Keyboard);
        b.handle(Input::PointerDown(INSIDE.0, INSIDE.1));
        assert!(!b.focus_visible());
    }
}
