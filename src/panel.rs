//! A set of controls with the routing every window needs: pointer capture,
//! keyboard focus and Tab order, key and UI Automation dispatch.
//!
//! The window feeds Win32 input to [`Panel`] and applies the returned
//! [`Outcome`]: call `SetCapture`/`ReleaseCapture`, raise the UIA events, repaint.

use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::UI::Accessibility::{
    ToggleState, ToggleState_Indeterminate, ToggleState_Off, ToggleState_On, UIA_ButtonControlTypeId,
    UIA_CONTROLTYPE_ID, UIA_CheckBoxControlTypeId,
};
use windows::core::Result;

use crate::button::{Button, FocusKind, Input, Key, Response, Visual};
use crate::check_box::{self, CheckBox, CheckState};
use crate::gfx::Surface;
use crate::theme::{self, Theme};
use crate::toggle_switch::ToggleSwitch;
use crate::uia;

/// Text formats the built-in controls draw with.
pub struct Formats {
    /// Centered Body ([`Button`]).
    pub label: IDWriteTextFormat,
    /// Left, vertically centered Body ([`ToggleSwitch`]).
    pub switch: IDWriteTextFormat,
    /// Left, top Body ([`CheckBox`] content; free text).
    pub body: IDWriteTextFormat,
    /// Segoe Fluent Icons at the CheckBox glyph size.
    pub icon: IDWriteTextFormat,
}

impl Formats {
    /// Creates every format on `surface`'s DirectWrite factory.
    pub fn new(surface: &Surface) -> Result<Self> {
        Ok(Self {
            label: Button::text_format(surface)?,
            switch: ToggleSwitch::text_format(surface)?,
            body: surface.format(&theme::BODY)?,
            icon: surface.icon_format(check_box::GLYPH_SIZE)?,
        })
    }
}

/// What a [`Panel`] needs from a control.
pub trait Control {
    /// Feeds one input event.
    fn handle(&mut self, input: Input) -> Response;
    /// Programmatic click (UI Automation Invoke/Toggle).
    fn invoke(&mut self) -> Response;
    /// Draws the control.
    fn draw(&self, s: &Surface, t: &Theme, f: &Formats) -> Result<()>;
    /// True while a transition needs frames.
    fn animating(&self) -> bool;
    /// True while the control holds the mouse capture.
    fn is_captured(&self) -> bool;
    /// `IsEnabled` (disabled controls are skipped by Tab).
    fn enabled(&self) -> bool;
    /// UI Automation `Name`.
    fn name(&self) -> &str;
    /// Bounds in client DIPs.
    fn rect(&self) -> D2D_RECT_F;
    /// `ToggleState` for controls with the Toggle pattern; `None` exposes Invoke.
    fn toggle_state(&self) -> Option<ToggleState> {
        None
    }
    /// UI Automation `ControlType`.
    fn control_type(&self) -> UIA_CONTROLTYPE_ID {
        UIA_ButtonControlTypeId
    }
}

fn on_off(on: bool) -> ToggleState {
    if on { ToggleState_On } else { ToggleState_Off }
}

impl Control for Button {
    fn handle(&mut self, input: Input) -> Response {
        Button::handle(self, input)
    }
    fn invoke(&mut self) -> Response {
        Button::invoke(self)
    }
    fn draw(&self, s: &Surface, t: &Theme, f: &Formats) -> Result<()> {
        Button::draw(self, s, t, &f.label)
    }
    fn animating(&self) -> bool {
        Button::animating(self)
    }
    fn is_captured(&self) -> bool {
        Button::is_captured(self)
    }
    fn enabled(&self) -> bool {
        self.visual() != Visual::Disabled
    }
    fn name(&self) -> &str {
        &self.label
    }
    fn rect(&self) -> D2D_RECT_F {
        self.rect
    }
    fn toggle_state(&self) -> Option<ToggleState> {
        self.is_checked().map(on_off)
    }
}

impl Control for ToggleSwitch {
    fn handle(&mut self, input: Input) -> Response {
        ToggleSwitch::handle(self, input)
    }
    fn invoke(&mut self) -> Response {
        ToggleSwitch::invoke(self)
    }
    fn draw(&self, s: &Surface, t: &Theme, f: &Formats) -> Result<()> {
        ToggleSwitch::draw(self, s, t, &f.switch)
    }
    fn animating(&self) -> bool {
        ToggleSwitch::animating(self)
    }
    fn is_captured(&self) -> bool {
        ToggleSwitch::is_captured(self)
    }
    fn enabled(&self) -> bool {
        self.visual() != Visual::Disabled
    }
    /// `ToggleSwitchAutomationPeer::GetNameCore` builds the name from the header.
    fn name(&self) -> &str {
        &self.header
    }
    fn rect(&self) -> D2D_RECT_F {
        self.rect
    }
    fn toggle_state(&self) -> Option<ToggleState> {
        Some(on_off(self.is_on()))
    }
}

impl Control for CheckBox {
    fn handle(&mut self, input: Input) -> Response {
        CheckBox::handle(self, input)
    }
    fn invoke(&mut self) -> Response {
        CheckBox::invoke(self)
    }
    fn draw(&self, s: &Surface, t: &Theme, f: &Formats) -> Result<()> {
        CheckBox::draw(self, s, t, &f.body, &f.icon)
    }
    fn animating(&self) -> bool {
        CheckBox::animating(self)
    }
    fn is_captured(&self) -> bool {
        CheckBox::is_captured(self)
    }
    fn enabled(&self) -> bool {
        self.visual() != Visual::Disabled
    }
    fn name(&self) -> &str {
        self.label()
    }
    fn rect(&self) -> D2D_RECT_F {
        CheckBox::rect(self)
    }
    fn toggle_state(&self) -> Option<ToggleState> {
        Some(match self.state() {
            CheckState::Checked => ToggleState_On,
            CheckState::Unchecked => ToggleState_Off,
            CheckState::Indeterminate => ToggleState_Indeterminate,
        })
    }
    /// `CheckBoxAutomationPeer` reports CheckBox.
    fn control_type(&self) -> UIA_CONTROLTYPE_ID {
        UIA_CheckBoxControlTypeId
    }
}

/// Something the window should tell UI Automation (and may show).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    /// A push button was invoked: raise `UIA_Invoke_InvokedEventId`.
    Invoked(usize),
    /// A toggle control changed state: raise the `ToggleState` property change.
    Toggled {
        /// Control index.
        index: usize,
        /// State before.
        old: ToggleState,
        /// State after.
        new: ToggleState,
    },
    /// Keyboard focus moved to a control: raise `UIA_AutomationFocusChangedEventId`.
    Focused(usize),
}

/// What the window must do after a [`Panel`] call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Repaint.
    pub redraw: bool,
    /// `Some(true)`: `SetCapture`; `Some(false)`: `ReleaseCapture` (the last request wins).
    pub capture: Option<bool>,
    /// UIA notifications, in order.
    pub events: Vec<Event>,
}

/// Controls in Tab order, with focus and capture routing.
pub struct Panel {
    /// The controls, in Tab (and UIA navigation) order.
    pub controls: Vec<Box<dyn Control>>,
    focused: Option<usize>,
}

impl Panel {
    /// A panel over `controls`, nothing focused.
    pub fn new(controls: Vec<Box<dyn Control>>) -> Self {
        Self { controls, focused: None }
    }

    /// Index of the control with keyboard focus.
    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// Routes one input. Pointer moves/ups/leaves go only to the control holding
    /// the capture, as XAML routes captured pointer events; key events go to the
    /// focused control.
    pub fn handle(&mut self, input: Input) -> Outcome {
        let mut out = Outcome::default();
        let targets: Vec<usize> = match input {
            Input::KeyDown(_) | Input::KeyUp(_) => self.focused.into_iter().collect(),
            Input::PointerMove(..) | Input::PointerUp(..) | Input::PointerLeave => {
                match self.controls.iter().position(|c| c.is_captured()) {
                    Some(captor) => vec![captor],
                    None => (0..self.controls.len()).collect(),
                }
            }
            _ => (0..self.controls.len()).collect(),
        };
        for i in targets {
            let before = self.controls[i].toggle_state();
            let r = self.controls[i].handle(input);
            self.apply(i, before, r, &mut out);
        }
        out
    }

    /// Applies one control's [`Response`]: capture, focus moves and click events.
    fn apply(&mut self, i: usize, before: Option<ToggleState>, r: Response, out: &mut Outcome) {
        if r.capture.is_some() {
            out.capture = r.capture;
        }
        out.redraw |= r.redraw;
        if r.take_focus {
            self.move_focus(Some(i), FocusKind::Pointer, out);
        }
        if r.lost_focus && self.focused == Some(i) {
            self.focused = None;
        }
        if r.clicked {
            out.redraw = true;
            out.events.push(match (before, self.controls[i].toggle_state()) {
                (Some(old), Some(new)) => Event::Toggled { index: i, old, new },
                _ => Event::Invoked(i),
            });
        }
    }

    /// Blurs the previous control (which releases a capture it held: Tab while the
    /// mouse is down) and focuses `to`.
    fn move_focus(&mut self, to: Option<usize>, kind: FocusKind, out: &mut Outcome) {
        if let Some(old) = self.focused.filter(|&old| Some(old) != to) {
            let blur = self.controls[old].handle(Input::Blur);
            if blur.capture.is_some() {
                out.capture = blur.capture;
            }
            out.redraw |= blur.redraw;
        }
        let changed = self.focused != to;
        if let Some(new) = to {
            out.redraw |= self.controls[new].handle(Input::Focus(kind)).redraw;
        }
        self.focused = to;
        if let Some(new) = to.filter(|_| changed) {
            out.events.push(Event::Focused(new));
        }
    }

    /// Key press or release for the focused control.
    pub fn key(&mut self, key: Key, down: bool) -> Outcome {
        self.handle(if down { Input::KeyDown(key) } else { Input::KeyUp(key) })
    }

    /// Tab / Shift+Tab over enabled controls.
    pub fn tab(&mut self, back: bool) -> Outcome {
        let n = self.controls.len();
        let mut out = Outcome::default();
        if n == 0 {
            return out;
        }
        let start = self.focused.unwrap_or(if back { 0 } else { n - 1 });
        // Disabled controls are skipped, like IsTabStop on a disabled control.
        let next = (1..=n)
            .map(|k| if back { (start + n - k) % n } else { (start + k) % n })
            .find(|&i| self.controls[i].enabled());
        self.move_focus(next, FocusKind::Keyboard, &mut out);
        out
    }

    /// Moves keyboard focus (UIA `SetFocus`, or `None` when the window loses focus).
    pub fn focus(&mut self, to: Option<usize>, kind: FocusKind) -> Outcome {
        let mut out = Outcome::default();
        if to.is_none_or(|i| i < self.controls.len()) {
            self.move_focus(to, kind, &mut out);
        }
        out
    }

    /// Programmatic click of control `index` (UIA Invoke/Toggle); controls ignore
    /// it while disabled.
    pub fn invoke(&mut self, index: usize) -> Outcome {
        let mut out = Outcome::default();
        if let Some(c) = self.controls.get_mut(index) {
            let before = c.toggle_state();
            let r = c.invoke();
            self.apply(index, before, r, &mut out);
        }
        out
    }

    /// True while any control animates.
    pub fn animating(&self) -> bool {
        self.controls.iter().any(|c| c.animating())
    }

    /// Draws every control.
    pub fn draw(&self, s: &Surface, t: &Theme, f: &Formats) -> Result<()> {
        self.controls.iter().try_for_each(|c| c.draw(s, t, f))
    }

    /// The UI Automation view of the controls.
    pub fn uia_items(&self) -> Vec<uia::Item> {
        self.controls.iter().map(|c| uia::Item {
            name: c.name().to_owned(),
            rect: c.rect(),
            enabled: c.enabled(),
            toggle: c.toggle_state(),
            control_type: c.control_type(),
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gfx::rect;

    /// Plain, toggle, disabled, check box: in a row, 100 DIPs apart.
    fn panel() -> Panel {
        let mut disabled = Button::new("Off", rect(200.0, 0.0, 280.0, 32.0), false);
        disabled.set_enabled(false);
        Panel::new(vec![
            Box::new(Button::new("Ok", rect(0.0, 0.0, 80.0, 32.0), false)),
            Box::new(Button::toggle("Wi-Fi", rect(100.0, 0.0, 180.0, 32.0), false)),
            Box::new(disabled),
            Box::new(CheckBox::new("Lembrar", rect(300.0, 0.0, 420.0, 32.0), false)),
        ])
    }

    fn at(i: usize) -> (f32, f32) {
        (i as f32 * 100.0 + 40.0, 16.0)
    }

    fn click(p: &mut Panel, i: usize) -> Outcome {
        let (x, y) = at(i);
        p.handle(Input::PointerDown(x, y));
        p.handle(Input::PointerUp(x, y))
    }

    #[test]
    fn press_should_capture_focus_the_control_and_raise_focus() {
        let mut p = panel();
        let (x, y) = at(0);
        let o = p.handle(Input::PointerDown(x, y));
        assert_eq!((o.capture, p.focused(), o.events), (Some(true), Some(0), vec![Event::Focused(0)]));
    }

    #[test]
    fn click_on_a_push_button_should_report_invoked_and_release() {
        let mut p = panel();
        let o = click(&mut p, 0);
        assert_eq!((o.capture, o.events, o.redraw), (Some(false), vec![Event::Invoked(0)], true));
    }

    #[test]
    fn click_on_a_toggle_should_report_old_and_new_state() {
        let mut p = panel();
        let o = click(&mut p, 1);
        assert_eq!(o.events, vec![Event::Toggled { index: 1, old: ToggleState_Off, new: ToggleState_On }]);
    }

    #[test]
    fn captured_pointer_moves_should_reach_only_the_captor() {
        let mut p = panel();
        let (x, y) = at(0);
        p.handle(Input::PointerDown(x, y));
        // Dragging over the toggle must not light it (its PointerOver would redraw it).
        let (tx, ty) = at(1);
        p.handle(Input::PointerMove(tx, ty));
        let up = p.handle(Input::PointerUp(tx, ty));
        // Released outside the captor: no click anywhere.
        assert!(up.events.is_empty());
    }

    #[test]
    fn tab_should_skip_disabled_controls() {
        let mut p = panel();
        p.tab(false);
        p.tab(false);
        let o = p.tab(false);
        assert_eq!((p.focused(), o.events), (Some(3), vec![Event::Focused(3)]));
    }

    #[test]
    fn shift_tab_from_nothing_should_focus_the_last_enabled() {
        let mut p = panel();
        p.tab(true);
        assert_eq!(p.focused(), Some(3));
    }

    #[test]
    fn tab_while_pressing_should_release_the_capture() {
        let mut p = panel();
        let (x, y) = at(0);
        p.handle(Input::PointerDown(x, y));
        let o = p.tab(false);
        assert_eq!((o.capture, p.focused()), (Some(false), Some(1)));
    }

    #[test]
    fn space_should_reach_the_focused_control() {
        let mut p = panel();
        p.focus(Some(1), FocusKind::Keyboard);
        p.key(Key::Space, true);
        let o = p.key(Key::Space, false);
        assert_eq!(o.events, vec![Event::Toggled { index: 1, old: ToggleState_Off, new: ToggleState_On }]);
    }

    #[test]
    fn losing_window_focus_should_blur_the_control() {
        let mut p = panel();
        p.focus(Some(0), FocusKind::Keyboard);
        let o = p.focus(None, FocusKind::Keyboard);
        assert_eq!((p.focused(), o.redraw, o.events), (None, true, vec![]));
    }

    #[test]
    fn invoke_should_toggle_a_check_box_and_report_it() {
        let mut p = panel();
        let o = p.invoke(3);
        assert_eq!(o.events, vec![Event::Toggled { index: 3, old: ToggleState_Off, new: ToggleState_On }]);
    }

    #[test]
    fn invoke_on_a_disabled_control_should_do_nothing() {
        let mut p = panel();
        assert_eq!(p.invoke(2), Outcome::default());
    }

    #[test]
    fn uia_items_should_reflect_the_controls() {
        let items = panel().uia_items();
        let summary: Vec<_> = items.iter().map(|i| (i.name.as_str(), i.enabled, i.toggle, i.control_type)).collect();
        assert_eq!(summary, [
            ("Ok", true, None, UIA_ButtonControlTypeId),
            ("Wi-Fi", true, Some(ToggleState_Off), UIA_ButtonControlTypeId),
            ("Off", false, None, UIA_ButtonControlTypeId),
            ("Lembrar", true, Some(ToggleState_Off), UIA_CheckBoxControlTypeId),
        ]);
    }
}
