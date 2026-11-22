//! WinUI `MenuFlyout`: the `MenuFlyoutPresenter` list of `MenuFlyoutItem`s and
//! `MenuFlyoutSeparator`s, over [`crate::flyout`]'s popup chrome. The window and
//! the light dismiss belong to the app, as with the content flyout.

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::core::Result;

use crate::button::{Input, Key};
use crate::gfx::{Surface, rect};
use crate::panel::Formats;
use crate::theme::{self, Theme};

/// `MenuFlyoutPresenterThemePadding` = 0,2,0,2 (left, top, right, bottom).
pub const PRESENTER_PADDING: (f32, f32, f32, f32) = (0.0, 2.0, 0.0, 2.0);
/// `MenuFlyoutItemMargin` = 4,2,4,2.
pub const ITEM_MARGIN: (f32, f32, f32, f32) = (4.0, 2.0, 4.0, 2.0);
/// `MenuFlyoutItemThemePaddingNarrow` = 11,4,11,5: mouse, pen and keyboard.
pub const ITEM_PADDING: (f32, f32, f32, f32) = (11.0, 4.0, 11.0, 5.0);
/// `MenuFlyoutSeparatorThemePadding` = -4,1,-4,1.
pub const SEPARATOR_PADDING: (f32, f32, f32, f32) = (-4.0, 1.0, -4.0, 1.0);
/// `MenuFlyoutSeparatorHeight`.
pub const SEPARATOR_HEIGHT: f32 = 1.0;
/// `FlyoutThemeMinWidth` and `MenuFlyoutThemeMinHeight`.
pub const MIN_SIZE: (f32, f32) = (96.0, 32.0);
/// `KeyboardAcceleratorTextBlock` left margin: 24,4,0,0.
pub const ACCELERATOR_GAP: f32 = 24.0;

/// One `MenuFlyoutItem`.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// `MenuFlyoutItem.Text`.
    pub text: String,
    /// `KeyboardAcceleratorTextOverride`, right-aligned in Caption.
    pub accelerator: Option<String>,
    /// `IsEnabled`: disabled items are skipped and never highlight.
    pub enabled: bool,
}

impl Item {
    /// An enabled item with no accelerator.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), accelerator: None, enabled: true }
    }
}

/// An entry of the menu.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// A `MenuFlyoutItem`.
    Item(Item),
    /// A `MenuFlyoutSeparator`.
    Separator,
}

/// Sizes measured by the caller, as [`crate::layout::Stack`] takes them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Measure {
    /// Item text (width, line height).
    pub text: (f32, f32),
    /// Accelerator text (width, line height).
    pub accelerator: Option<(f32, f32)>,
}

impl Entry {
    /// Measures the text and accelerator of this entry; a separator measures zero.
    pub fn measure(&self, s: &Surface, f: &Formats) -> Result<Measure> {
        let Entry::Item(item) = self else { return Ok(Measure::default()) };
        Ok(Measure {
            text: s.measure(&item.text, &f.switch)?,
            accelerator: item.accelerator.as_ref().map(|a| s.measure(a, &f.caption)).transpose()?,
        })
    }
}

/// What the app must do after [`MenuFlyout::handle`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Outcome {
    /// Repaint.
    pub redraw: bool,
    /// `Some(true)`: `SetCapture`; `Some(false)`: `ReleaseCapture`.
    pub capture: Option<bool>,
    /// Entry invoked by a click, Space or Enter: the menu should close.
    pub invoked: Option<usize>,
}

/// The menu, laid out in client DIPs.
pub struct MenuFlyout {
    /// The entries, in menu order.
    pub entries: Vec<Entry>,
    /// Highlight box of each entry (separators included), in client DIPs.
    pub rects: Vec<D2D_RECT_F>,
    sizes: Vec<Measure>,
    size: (f32, f32),
    /// Entry under the pointer or with keyboard focus.
    hot: Option<usize>,
    /// Entry held down (pointer, Space or Enter).
    pressed: Option<usize>,
    /// The pointer press belongs to this menu (capture taken).
    captured: bool,
}

impl MenuFlyout {
    /// Lays out `entries`, one [`Measure`] each, from the top-left of the popup.
    pub fn new(entries: Vec<Entry>, sizes: &[Measure]) -> Self {
        assert_eq!(entries.len(), sizes.len(), "one measure per entry");
        let mut width: f32 = 0.0;
        for (entry, m) in entries.iter().zip(sizes) {
            if matches!(entry, Entry::Item(_)) {
                let accelerator = m.accelerator.map_or(0.0, |a| ACCELERATOR_GAP + a.0);
                let content = m.text.0 + accelerator;
                width = width.max(ITEM_MARGIN.0 + ITEM_PADDING.0 + content + ITEM_PADDING.2 + ITEM_MARGIN.2);
            }
        }
        let width = width.max(MIN_SIZE.0);
        let mut y = PRESENTER_PADDING.1;
        let rects = entries.iter().zip(sizes).map(|(entry, m)| match entry {
            Entry::Item(_) => {
                let top = y + ITEM_MARGIN.1;
                let bottom = top + ITEM_PADDING.1 + m.text.1 + ITEM_PADDING.3;
                y = bottom + ITEM_MARGIN.3;
                rect(ITEM_MARGIN.0, top, width - ITEM_MARGIN.2, bottom)
            }
            Entry::Separator => {
                let top = y + SEPARATOR_PADDING.1;
                y += SEPARATOR_HEIGHT + SEPARATOR_PADDING.1 + SEPARATOR_PADDING.3;
                rect(0.0, top, width, top + SEPARATOR_HEIGHT)
            }
        }).collect();
        let height = (y + PRESENTER_PADDING.3).max(MIN_SIZE.1);
        Self { entries, rects, sizes: sizes.to_vec(), size: (width, height), hot: None, pressed: None, captured: false }
    }

    /// Menu size in DIPs, for [`crate::flyout::place`].
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// Text of entry `index`, if it is an item.
    pub fn text(&self, index: usize) -> Option<&str> {
        match self.entries.get(index) {
            Some(Entry::Item(item)) => Some(&item.text),
            _ => None,
        }
    }

    /// Routes pointer, Space/Enter and arrow keys. Escape and light dismiss are the app's.
    pub fn handle(&mut self, input: Input) -> Outcome {
        let mut out = Outcome::default();
        match input {
            Input::PointerDown(x, y) => {
                let was = (self.hot, self.pressed);
                self.hot = self.hit(x, y);
                self.pressed = self.hot;
                // A press on a margin or a disabled item can also clear a keyboard highlight.
                out.redraw = (self.hot, self.pressed) != was;
                if self.hot.is_some() {
                    self.captured = true;
                    out.capture = Some(true);
                }
            }
            Input::PointerMove(x, y) => {
                let hit = self.hit(x, y);
                // Captured moves go only to the press target, as XAML routes them.
                let hot = if self.captured { if hit == self.pressed { self.pressed } else { None } } else { hit };
                out.redraw = hot != self.hot;
                self.hot = hot;
            }
            Input::PointerUp(x, y) => {
                if self.captured {
                    let hit = self.hit(x, y);
                    out.capture = Some(false);
                    if self.pressed.is_some() && self.pressed == hit {
                        out.invoked = hit;
                    }
                    self.captured = false;
                    self.pressed = None;
                    out.redraw = true;
                }
            }
            Input::PointerLeave => {
                out.redraw = self.hot.is_some() || self.pressed.is_some();
                self.hot = None;
                self.pressed = None;
                if self.captured {
                    self.captured = false;
                    out.capture = Some(false);
                }
            }
            Input::CaptureLost => {
                self.captured = false;
                self.pressed = None;
                out.redraw = self.hot.take().is_some();
            }
            Input::KeyDown(key) => {
                match key {
                    Key::Down => { self.pressed = None; self.hot = self.step(self.hot, true); }
                    Key::Up => { self.pressed = None; self.hot = self.step(self.hot, false); }
                    Key::Home => { self.pressed = None; self.hot = self.step(None, true); }
                    Key::End => { self.pressed = None; self.hot = self.step(None, false); }
                    Key::Space | Key::Enter => self.pressed = self.hot,
                    _ => self.pressed = None,
                }
                out.redraw = self.pressed.is_some() || self.hot.is_some();
            }
            Input::KeyUp(Key::Space | Key::Enter) => {
                if self.pressed.is_some() && self.pressed == self.hot {
                    out.invoked = self.pressed;
                }
                out.redraw = self.pressed.take().is_some();
            }
            Input::KeyUp(_) | Input::Focus(_) | Input::Blur => {}
        }
        out
    }

    /// Next enabled entry from `from` in `down` direction, wrapping around.
    fn step(&self, from: Option<usize>, down: bool) -> Option<usize> {
        let n = self.entries.len();
        if n == 0 {
            return None;
        }
        let start = match (from, down) {
            (Some(i), true) => (i + 1) % n,
            (Some(i), false) => (i + n - 1) % n,
            (None, true) => 0,
            (None, false) => n - 1,
        };
        (0..n)
            .map(|k| if down { (start + k) % n } else { (start + n - k) % n })
            .find(|&i| self.enabled(i))
    }

    fn enabled(&self, i: usize) -> bool {
        matches!(self.entries.get(i), Some(Entry::Item(item)) if item.enabled)
    }

    /// Enabled item under the pointer.
    fn hit(&self, x: f32, y: f32) -> Option<usize> {
        self.rects.iter().enumerate()
            .find(|&(i, r)| self.enabled(i) && x >= r.left && x < r.right && y >= r.top && y < r.bottom)
            .map(|(i, _)| i)
    }

    /// Draws the items and separators; the presenter chrome is [`crate::flyout::draw_chrome`].
    pub fn draw(&self, s: &Surface, t: &Theme, f: &Formats) {
        for (i, entry) in self.entries.iter().enumerate() {
            let r = self.rects[i];
            let Entry::Item(item) = entry else {
                s.fill_round(r, 0.0, t.divider);
                continue;
            };
            let state = if !item.enabled {
                State::Disabled
            } else if self.pressed == Some(i) {
                State::Pressed
            } else if self.hot == Some(i) {
                State::Hot
            } else {
                State::Rest
            };
            let c = colors(t, state);
            if let Some(background) = c.background {
                s.fill_round(r, theme::CONTROL_RADIUS, background);
            }
            let m = self.sizes[i];
            let top = r.top + ITEM_PADDING.1;
            let mut right = r.right - ITEM_PADDING.2;
            if let Some(accelerator) = &item.accelerator {
                let (aw, ah) = m.accelerator.unwrap_or_default();
                let x = right - aw;
                let y = top + (m.text.1 - ah) / 2.0;
                s.text(accelerator, &f.caption, rect(x, y, right, y + ah), c.accelerator);
                right = x - ACCELERATOR_GAP;
            }
            s.text(&item.text, &f.switch, rect(r.left + ITEM_PADDING.0, top, right, r.bottom - ITEM_PADDING.3), c.text);
        }
    }
}

/// Visual state, in WinUI priority order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Rest,
    Hot,
    Pressed,
    Disabled,
}

struct Colors {
    background: Option<D2D1_COLOR_F>,
    text: D2D1_COLOR_F,
    accelerator: D2D1_COLOR_F,
}

/// `MenuFlyoutItem*` brushes for `state`.
fn colors(t: &Theme, state: State) -> Colors {
    if let Some(c) = t.high_contrast {
        let (background, text) = match state {
            State::Rest => (None, c.button_text),
            State::Hot | State::Pressed => (Some(c.highlight), c.highlight_text),
            State::Disabled => (None, c.gray_text),
        };
        let accelerator = match state {
            State::Rest => c.window_text,
            State::Hot | State::Pressed => c.highlight_text,
            State::Disabled => c.gray_text,
        };
        return Colors { background, text, accelerator };
    }
    Colors {
        background: match state {
            State::Hot => Some(t.subtle_hover),
            State::Pressed => Some(t.subtle_pressed),
            State::Rest | State::Disabled => None,
        },
        text: if state == State::Disabled { t.text_disabled } else { t.text_primary },
        accelerator: if state == State::Disabled { t.text_disabled } else { t.text_secondary },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tests::HC;

    /// An item with a 100 DIP text, 20 DIP line.
    const WIDE: Measure = Measure { text: (100.0, 20.0), accelerator: None };
    /// An item with a 10 DIP text, 20 DIP line.
    const NARROW: Measure = Measure { text: (10.0, 20.0), accelerator: None };

    fn item(text: &str) -> Entry {
        Entry::Item(Item::new(text))
    }

    fn two_items() -> MenuFlyout {
        MenuFlyout::new(vec![item("Abrir"), item("Fechar")], &[WIDE, WIDE])
    }

    #[test]
    fn first_item_should_sit_after_the_presenter_padding_and_its_margin() {
        let m = two_items();
        let r = m.rects[0];
        assert_eq!((r.left, r.top, r.right, r.bottom), (4.0, 4.0, 126.0, 33.0));
    }

    #[test]
    fn items_should_be_separated_by_the_two_margins() {
        assert_eq!(two_items().rects[1].top, 33.0 + 4.0);
    }

    #[test]
    fn width_should_fit_the_widest_item_plus_padding_and_margins() {
        assert_eq!(two_items().size(), (130.0, 70.0));
    }

    #[test]
    fn width_should_never_be_below_the_flyout_minimum() {
        let m = MenuFlyout::new(vec![item("A")], &[NARROW]);
        assert_eq!((m.size().0, m.rects[0].right), (96.0, 92.0));
    }

    #[test]
    fn accelerator_should_reserve_24_dips_before_it() {
        let m = MenuFlyout::new(vec![item("Abrir")], &[Measure { text: (100.0, 20.0), accelerator: Some((30.0, 16.0)) }]);
        assert_eq!(m.size().0, 184.0);
    }

    #[test]
    fn separator_should_take_three_dips_and_span_the_full_width() {
        let m = MenuFlyout::new(vec![item("Abrir"), Entry::Separator, item("Fechar")], &[WIDE, Measure::default(), WIDE]);
        let r = m.rects[1];
        assert_eq!((r.left, r.top, r.right, r.bottom), (0.0, 36.0, 130.0, 37.0));
        assert_eq!(m.rects[2].top, 40.0);
    }

    #[test]
    fn menu_should_never_be_shorter_than_32() {
        let tiny = Measure { text: (1.0, 1.0), accelerator: None };
        assert_eq!(MenuFlyout::new(vec![item("A")], &[tiny]).size().1, 32.0);
    }

    /// Center of entry `i`.
    fn center(m: &MenuFlyout, i: usize) -> (f32, f32) {
        let r = m.rects[i];
        ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0)
    }

    #[test]
    fn pressing_and_releasing_an_item_should_invoke_it_and_release_the_capture() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        assert_eq!(m.handle(Input::PointerDown(x, y)).capture, Some(true));
        let out = m.handle(Input::PointerUp(x, y));
        assert_eq!((out.capture, out.invoked), (Some(false), Some(0)));
    }

    #[test]
    fn releasing_outside_the_pressed_item_should_not_invoke() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        m.handle(Input::PointerDown(x, y));
        let (x2, y2) = center(&m, 1);
        let out = m.handle(Input::PointerUp(x2, y2));
        assert_eq!((out.capture, out.invoked), (Some(false), None));
    }

    #[test]
    fn disabled_item_should_not_invoke_by_pointer_or_keyboard() {
        let mut disabled = Item::new("Fechar");
        disabled.enabled = false;
        let mut m = MenuFlyout::new(vec![Entry::Item(disabled)], &[WIDE]);
        let (x, y) = center(&m, 0);
        assert_eq!(m.handle(Input::PointerDown(x, y)), Outcome::default());
        assert_eq!(m.handle(Input::PointerUp(x, y)), Outcome::default());
        assert!(!m.handle(Input::KeyDown(Key::Down)).redraw);
        assert_eq!(m.handle(Input::KeyUp(Key::Enter)).invoked, None);
    }

    #[test]
    fn pressing_a_disabled_item_should_clear_a_keyboard_highlight() {
        let mut disabled = Item::new("Fechar");
        disabled.enabled = false;
        let mut m = MenuFlyout::new(vec![item("Abrir"), Entry::Item(disabled)], &[WIDE, WIDE]);
        m.handle(Input::KeyDown(Key::Down));
        assert_eq!(m.hot, Some(0));
        let (x, y) = center(&m, 1);
        assert!(m.handle(Input::PointerDown(x, y)).redraw);
        assert_eq!(m.hot, None);
    }

    #[test]
    fn hover_should_highlight_only_the_item_under_the_pointer() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        assert!(m.handle(Input::PointerMove(x, y)).redraw);
        assert_eq!(m.hot, Some(0));
        // Same item: nothing to repaint.
        assert!(!m.handle(Input::PointerMove(x, y)).redraw);
        let (x2, y2) = center(&m, 1);
        assert!(m.handle(Input::PointerMove(x2, y2)).redraw);
        assert_eq!(m.hot, Some(1));
    }

    #[test]
    fn pointer_leave_should_clear_the_highlight() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        m.handle(Input::PointerMove(x, y));
        let out = m.handle(Input::PointerLeave);
        assert_eq!((out.redraw, m.hot), (true, None));
    }

    #[test]
    fn leaving_the_popup_should_release_the_capture_and_cancel_the_press() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        m.handle(Input::PointerDown(x, y));
        let out = m.handle(Input::PointerLeave);
        assert_eq!((out.capture, out.redraw), (Some(false), true));
        assert_eq!(m.handle(Input::PointerUp(x, y)).invoked, None);
    }

    #[test]
    fn the_right_and_bottom_edges_of_an_item_are_exclusive() {
        let mut m = two_items();
        let r = m.rects[0];
        assert!(!m.handle(Input::PointerMove(r.right, r.top + 1.0)).redraw);
        assert!(!m.handle(Input::PointerMove(r.left + 1.0, r.bottom)).redraw);
        assert_eq!(m.hot, None);
        assert!(m.handle(Input::PointerMove(r.right - 0.01, r.bottom - 0.01)).redraw);
    }

    #[test]
    fn separator_is_not_hittable() {
        let mut m = MenuFlyout::new(vec![item("Abrir"), Entry::Separator], &[WIDE, Measure::default()]);
        let r = m.rects[1];
        let out = m.handle(Input::PointerMove((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0));
        assert_eq!((out.redraw, m.hot), (false, None));
    }

    #[test]
    fn arrows_should_skip_disabled_entries_and_wrap() {
        let mut disabled = Item::new("Off");
        disabled.enabled = false;
        let mut m = MenuFlyout::new(
            vec![item("A"), Entry::Item(disabled), Entry::Separator, item("D")],
            &[WIDE, WIDE, Measure::default(), WIDE],
        );
        let down = |m: &mut MenuFlyout| m.handle(Input::KeyDown(Key::Down)).redraw;
        down(&mut m);
        assert_eq!(m.hot, Some(0));
        down(&mut m);
        assert_eq!(m.hot, Some(3));
        down(&mut m);
        assert_eq!(m.hot, Some(0));
        m.handle(Input::KeyDown(Key::Up));
        assert_eq!(m.hot, Some(3));
        m.handle(Input::KeyDown(Key::Home));
        assert_eq!(m.hot, Some(0));
        m.handle(Input::KeyDown(Key::End));
        assert_eq!(m.hot, Some(3));
    }

    #[test]
    fn enter_should_invoke_the_keyboard_item_on_key_up() {
        let mut m = two_items();
        m.handle(Input::KeyDown(Key::Down));
        let down = m.handle(Input::KeyDown(Key::Enter));
        assert_eq!((down.invoked, down.redraw), (None, true));
        assert_eq!(m.handle(Input::KeyUp(Key::Enter)).invoked, Some(0));
    }

    #[test]
    fn other_keys_should_cancel_a_held_enter() {
        let mut m = two_items();
        m.handle(Input::KeyDown(Key::Down));
        m.handle(Input::KeyDown(Key::Enter));
        m.handle(Input::KeyDown(Key::Other));
        assert_eq!(m.handle(Input::KeyUp(Key::Enter)).invoked, None);
    }

    #[test]
    fn losing_the_capture_should_abandon_the_press() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        m.handle(Input::PointerDown(x, y));
        m.handle(Input::CaptureLost);
        assert_eq!(m.handle(Input::PointerUp(x, y)).invoked, None);
    }

    #[test]
    fn an_empty_menu_should_ignore_the_keyboard() {
        let mut m = MenuFlyout::new(Vec::new(), &[]);
        assert_eq!(m.handle(Input::KeyDown(Key::Up)), Outcome::default());
        assert_eq!(m.handle(Input::KeyDown(Key::End)), Outcome::default());
    }

    #[test]
    fn a_captured_drag_should_highlight_only_the_press_target() {
        let mut m = two_items();
        let (x, y) = center(&m, 0);
        m.handle(Input::PointerDown(x, y));
        let (x2, y2) = center(&m, 1);
        m.handle(Input::PointerMove(x2, y2));
        assert_eq!((m.hot, m.pressed), (None, Some(0)));
        m.handle(Input::PointerMove(x, y));
        assert_eq!(m.hot, Some(0));
    }

    #[test]
    fn rest_and_hover_should_use_the_menu_item_brushes() {
        let t = Theme::DARK;
        let rest = colors(&t, State::Rest);
        let hot = colors(&t, State::Hot);
        let pressed = colors(&t, State::Pressed);
        assert_eq!((rest.background, rest.text, rest.accelerator), (None, t.text_primary, t.text_secondary));
        assert_eq!((hot.background, pressed.background), (Some(t.subtle_hover), Some(t.subtle_pressed)));
    }

    #[test]
    fn disabled_should_use_the_disabled_text_without_a_background() {
        let t = Theme::LIGHT;
        let c = colors(&t, State::Disabled);
        assert_eq!((c.background, c.text, c.accelerator), (None, t.text_disabled, t.text_disabled));
    }

    #[test]
    fn high_contrast_should_use_highlight_and_window_text() {
        let t = Theme::high_contrast(HC);
        let (rest, hot) = (colors(&t, State::Rest), colors(&t, State::Hot));
        assert_eq!((rest.background, rest.text, rest.accelerator), (None, HC.button_text, HC.window_text));
        assert_eq!((hot.background, hot.text, hot.accelerator), (Some(HC.highlight), HC.highlight_text, HC.highlight_text));
    }
}
