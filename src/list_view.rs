//! Virtualization core for a WinUI `ListView`: only the items crossing the
//! viewport (plus overscan) are laid out. Drawing and scrolling input belong to
//! the app, as with [`crate::flyout`].

use std::ops::Range;

use windows::Win32::Graphics::Direct2D::Common::{D2D_RECT_F, D2D1_COLOR_F};
use windows::core::Result;

use crate::button::{Input, Key};
use crate::gfx::{Surface, rect};
use crate::panel::Formats;
use crate::theme::{self, Theme};

/// `ListViewItemMinHeight`.
pub const ITEM_HEIGHT: f32 = 40.0;
/// `ListViewItemMinWidth`.
pub const MIN_WIDTH: f32 = 88.0;
/// `Padding` of `DefaultListViewItemStyle`: 16,0,12,0 (left/top/right/bottom).
pub const PADDING: (f32, f32, f32, f32) = (16.0, 0.0, 12.0, 0.0);
/// `ListViewItemCornerRadius`.
pub const RADIUS: f32 = 4.0;

/// A flat, fixed-height, single-selection list. Coordinates are DIPs relative
/// to the list's viewport; `offset` is how far the content is scrolled.
#[derive(Clone, Debug, PartialEq)]
pub struct ListView {
    items: Vec<String>,
    item_height: f32,
    offset: f32,
    viewport: f32,
    overscan: usize,
    /// Pointer-over item.
    pub hot: Option<usize>,
    /// Selected item (single selection).
    pub selected: Option<usize>,
    pressed: Option<usize>,
    captured: bool,
    focused: bool,
    key_pressed: bool,
}

impl ListView {
    /// A list with [`ITEM_HEIGHT`] rows, no scroll and one row of overscan.
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            item_height: ITEM_HEIGHT,
            offset: 0.0,
            viewport: 0.0,
            overscan: 1,
            hot: None,
            selected: None,
            pressed: None,
            captured: false,
            focused: false,
            key_pressed: false,
        }
    }

    /// Number of items.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// True when the list has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Text of item `index`.
    pub fn item(&self, index: usize) -> Option<&str> {
        self.items.get(index).map(String::as_str)
    }

    /// Height of the whole content (not clipped to the viewport).
    pub fn total_height(&self) -> f32 {
        self.items.len() as f32 * self.item_height
    }

    /// Visible height of the viewport.
    pub fn viewport(&self) -> f32 {
        self.viewport
    }

    /// Current scroll offset.
    pub fn offset(&self) -> f32 {
        self.offset
    }

    /// Sets the viewport height and keeps the scroll offset valid.
    pub fn set_viewport(&mut self, height: f32) {
        self.viewport = height.max(0.0);
        self.scroll_to(self.offset);
    }

    /// Scrolls to `offset`, clamped to the content.
    pub fn scroll_to(&mut self, offset: f32) {
        let max = (self.total_height() - self.viewport).max(0.0);
        let next = offset.clamp(0.0, max);
        if next != self.offset {
            // The row under the pointer changed; the app repaints the whole list.
            self.hot = None;
        }
        self.offset = next;
    }

    /// Scrolls by `delta` DIPs.
    pub fn scroll_by(&mut self, delta: f32) {
        self.scroll_to(self.offset + delta);
    }

    /// Indices whose rows can intersect the viewport, with overscan.
    pub fn visible(&self) -> Range<usize> {
        let n = self.items.len();
        if n == 0 || self.viewport <= 0.0 {
            return 0..0;
        }
        let first = (self.offset / self.item_height) as usize;
        let last = ((self.offset + self.viewport) / self.item_height).ceil() as usize;
        let first = first.saturating_sub(self.overscan).min(n);
        let last = (last + self.overscan).min(n);
        first..last
    }

    /// Rects of the virtualized rows, relative to the viewport's top-left.
    pub fn arrange(&self, left: f32, top: f32, width: f32) -> Vec<(usize, D2D_RECT_F)> {
        self.visible()
            .map(|i| {
                let y = top + i as f32 * self.item_height - self.offset;
                (i, rect(left, y, left + width, y + self.item_height))
            })
            .collect()
    }

    /// Row under a point in viewport coordinates.
    pub fn hit(&self, x: f32, y: f32, left: f32, top: f32, width: f32) -> Option<usize> {
        if x < left || x >= left + width || y < top || y >= top + self.viewport {
            return None;
        }
        let i = ((y + self.offset - top) / self.item_height) as usize;
        (i < self.items.len()).then_some(i)
    }

    /// Selects an item and scrolls it into view if needed.
    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = Some(index);
            self.ensure_visible(index);
        }
    }

    /// Moves the selection by `delta`, keeping it inside the list.
    pub fn move_selection(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let next = match self.selected {
            Some(i) => (i as isize + delta).clamp(0, self.items.len() as isize - 1) as usize,
            None if delta >= 0 => 0,
            None => self.items.len() - 1,
        };
        self.select(next)
    }

    /// Scrolls just enough to bring `index` inside the viewport. Assumes a
    /// viewport at least one row tall; shorter ones align the row's top.
    pub fn ensure_visible(&mut self, index: usize) {
        if index >= self.items.len() || self.viewport == 0.0 {
            return;
        }
        let top = index as f32 * self.item_height;
        let bottom = top + self.item_height;
        if top < self.offset {
            self.scroll_to(top);
        } else if bottom > self.offset + self.viewport {
            self.scroll_to(bottom - self.viewport);
        }
    }

    /// Routes pointer, keyboard and focus input. `left/top/width` are the
    /// viewport origin and width in client DIPs.
    pub fn handle(&mut self, input: Input, left: f32, top: f32, width: f32) -> Outcome {
        let mut out = Outcome::default();
        match input {
            Input::PointerMove(x, y) => {
                let hit = self.hit(x, y, left, top, width);
                let hot = if self.captured { if hit == self.pressed { self.pressed } else { None } } else { hit };
                out.redraw = hot != self.hot;
                self.hot = hot;
            }
            Input::PointerDown(x, y) => {
                let was = (self.hot, self.pressed);
                self.hot = self.hit(x, y, left, top, width);
                self.pressed = self.hot;
                out.redraw = (self.hot, self.pressed) != was;
                if let Some(i) = self.hot {
                    self.select(i);
                    self.captured = true;
                    out.capture = Some(true);
                    out.redraw = true;
                }
            }
            Input::PointerUp(x, y) => {
                if self.captured {
                    let hit = self.hit(x, y, left, top, width);
                    out.capture = Some(false);
                    out.clicked = self.pressed.is_some() && self.pressed == hit;
                    self.captured = false;
                    self.pressed = None;
                    out.redraw = true;
                }
            }
            Input::PointerLeave | Input::CaptureLost | Input::Blur => {
                out.redraw = self.hot.is_some() || self.pressed.is_some();
                self.hot = None;
                self.pressed = None;
                self.key_pressed = false;
                if self.captured {
                    self.captured = false;
                    out.capture = Some(false);
                }
                if matches!(input, Input::Blur) {
                    self.focused = false;
                }
            }
            Input::Focus(_) => {
                out.redraw = !self.focused;
                self.focused = true;
            }
            Input::KeyDown(key) if self.focused => {
                let before = (self.offset, self.selected);
                match key {
                    Key::Down => self.move_selection(1),
                    Key::Up => self.move_selection(-1),
                    Key::Home => self.select(0),
                    Key::End => self.select(self.items.len().saturating_sub(1)),
                    Key::Space | Key::Enter => self.key_pressed = true,
                    _ => self.key_pressed = false,
                }
                out.redraw = (self.offset, self.selected) != before;
            }
            Input::KeyUp(Key::Space | Key::Enter) if self.key_pressed => {
                out.clicked = self.selected.is_some();
                self.key_pressed = false;
            }
            Input::KeyDown(_) | Input::KeyUp(_) => {}
        }
        out
    }

    /// Draws the virtualized rows with the `DefaultListViewItemStyle` brushes.
    pub fn draw(&self, s: &Surface, t: &Theme, f: &Formats, left: f32, top: f32, width: f32) -> Result<()> {
        let bottom = top + self.viewport;
        s.with_clip(rect(left, top, left + width, bottom), |s| {
        for (i, r) in self.arrange(left, top, width) {
            // Overscan rows exist for hit testing and keyboard handoff, but the
            // ScrollViewer clips them: never paint outside the viewport.
            if r.bottom <= top || r.top >= bottom {
                continue;
            }
            let r = rect(r.left, r.top.max(top), r.right, r.bottom.min(bottom));
            let selected = self.selected == Some(i);
            let color = style(t, selected, self.hot == Some(i), self.pressed == Some(i));
            if let Some(bg) = color.background {
                s.fill_round(r, RADIUS, bg);
            }
            if selected {
                // `SelectionIndicatorBrush` (AccentFillColorDefault), 4 DIPs wide.
                let bar = rect(r.left, r.top, r.left + 4.0, r.bottom);
                s.fill_round(bar, theme::CONTROL_RADIUS / 2.0, t.accent);
            }
            if self.focused && selected {
                s.stroke_round(r, RADIUS, 2.0, t.focus_outer);
            }
            s.text(&self.items[i], &f.switch,
                rect(r.left + PADDING.0, r.top, r.right - PADDING.2, r.bottom), color.text);
        }
        });
        Ok(())
    }
}

/// What the app must do after [`ListView::handle`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Repaint.
    pub redraw: bool,
    /// `Some(true)`: `SetCapture`; `Some(false)`: `ReleaseCapture`.
    pub capture: Option<bool>,
    /// Item activated by a click or Space/Enter.
    pub clicked: bool,
}

/// `ListViewItem*` brushes for one state (`ListViewItem_themeresources.xaml`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Style {
    background: Option<D2D1_COLOR_F>,
    text: D2D1_COLOR_F,
}

fn style(t: &Theme, selected: bool, hot: bool, pressed: bool) -> Style {
    if let Some(c) = t.high_contrast {
        let active = selected || hot || pressed;
        return Style {
            background: active.then_some(c.highlight),
            text: if active { c.highlight_text } else { c.button_text },
        };
    }
    let background = if pressed && (selected || hot) {
        Some(t.subtle_pressed) // SelectedPressed / SelectedPointerOver when pressed
    } else if selected || hot || pressed {
        Some(t.subtle_hover) // Selected / PointerOver / Pressed
    } else {
        None
    };
    Style { background, text: t.text_primary }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(n: usize) -> ListView {
        let mut l = ListView::new((0..n).map(|i| format!("Item {i}")).collect());
        l.set_viewport(100.0);
        l
    }

    #[test]
    fn pointer_down_should_select_capture_and_click_on_release() {
        let mut l = list(10);
        let out = l.handle(Input::PointerDown(50.0, 10.0), 0.0, 0.0, 200.0);
        assert_eq!((out.capture, out.redraw, l.selected), (Some(true), true, Some(0)));
        let out = l.handle(Input::PointerUp(50.0, 10.0), 0.0, 0.0, 200.0);
        assert_eq!((out.capture, out.clicked), (Some(false), true));
    }

    #[test]
    fn dragging_out_should_cancel_the_click() {
        let mut l = list(10);
        l.handle(Input::PointerDown(50.0, 10.0), 0.0, 0.0, 200.0);
        let out = l.handle(Input::PointerUp(50.0, 90.0), 0.0, 0.0, 200.0);
        assert!(!out.clicked);
    }

    #[test]
    fn arrows_should_move_the_selection_and_scroll_it_into_view() {
        let mut l = list(100);
        l.handle(Input::Focus(crate::button::FocusKind::Keyboard), 0.0, 0.0, 200.0);
        for _ in 0..10 {
            l.handle(Input::KeyDown(Key::Down), 0.0, 0.0, 200.0);
        }
        assert_eq!(l.selected, Some(9));
        assert_eq!(l.offset(), 300.0);
        l.handle(Input::KeyDown(Key::End), 0.0, 0.0, 200.0);
        assert_eq!(l.selected, Some(99));
        assert_eq!(l.offset(), 3900.0);
        l.handle(Input::KeyDown(Key::Home), 0.0, 0.0, 200.0);
        assert_eq!(l.selected, Some(0));
        assert_eq!(l.offset(), 0.0);
    }

    #[test]
    fn scrolling_should_invalidate_the_hover() {
        let mut l = list(10);
        l.hot = Some(3);
        l.scroll_by(40.0);
        assert_eq!(l.hot, None);
    }

    #[test]
    fn keyboard_click_needs_focus_and_a_matching_key_down() {
        let mut l = list(10);
        l.select(2);
        assert!(!l.handle(Input::KeyUp(Key::Space), 0.0, 0.0, 200.0).clicked);
        l.handle(Input::Focus(crate::button::FocusKind::Keyboard), 0.0, 0.0, 200.0);
        assert!(!l.handle(Input::KeyUp(Key::Space), 0.0, 0.0, 200.0).clicked);
        l.handle(Input::KeyDown(Key::Space), 0.0, 0.0, 200.0);
        assert!(l.handle(Input::KeyUp(Key::Space), 0.0, 0.0, 200.0).clicked);
    }

    #[test]
    fn item_brushes_should_follow_the_winui_states() {
        let t = Theme::DARK;
        assert_eq!(style(&t, false, false, false).background, None);
        assert_eq!(style(&t, true, false, false).background, Some(t.subtle_hover));
        assert_eq!(style(&t, false, true, false).background, Some(t.subtle_hover));
        assert_eq!(style(&t, true, true, true).background, Some(t.subtle_pressed));
        assert!(style(&t, false, false, false).text.a > 0.0);
    }

    #[test]
    fn total_height_should_be_the_number_of_items_times_40() {
        assert_eq!(list(100).total_height(), 4000.0);
        assert!(ListView::new(Vec::new()).is_empty());
    }

    #[test]
    fn visible_rows_should_cover_the_viewport_plus_one_row_of_overscan() {
        assert_eq!(list(10).visible(), 0..4);
    }

    #[test]
    fn scrolling_should_move_the_virtualized_window() {
        let mut l = list(10);
        l.scroll_to(60.0);
        assert_eq!(l.visible(), 0..5);
        l.scroll_to(120.0);
        assert_eq!(l.visible(), 2..7);
    }

    #[test]
    fn scroll_should_clamp_to_the_content_end() {
        let mut l = list(10);
        l.scroll_to(10_000.0);
        assert_eq!(l.offset(), 300.0);
        l.scroll_by(-10_000.0);
        assert_eq!(l.offset(), 0.0);
    }

    #[test]
    fn a_resized_viewport_should_keep_the_offset_valid() {
        let mut l = list(10);
        l.scroll_to(500.0);
        l.set_viewport(1000.0);
        assert_eq!(l.offset(), 0.0);
    }

    #[test]
    fn arrange_should_return_only_visible_rows_at_their_scrolled_y() {
        let mut l = list(10);
        l.scroll_to(50.0);
        let rows = l.arrange(10.0, 20.0, 200.0);
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].0, 0);
        assert_eq!(rows[0].1.top, -30.0);
        assert_eq!(rows[1].1.top, 10.0);
        assert_eq!(rows[0].1.right, 210.0);
    }

    #[test]
    fn hit_should_use_the_scroll_offset() {
        let mut l = list(10);
        l.scroll_to(100.0);
        assert_eq!(l.hit(50.0, 10.0, 0.0, 0.0, 200.0), Some(2));
        assert_eq!(l.hit(50.0, 0.0, 0.0, 0.0, 200.0), Some(2));
        assert_eq!(l.hit(201.0, 10.0, 0.0, 0.0, 200.0), None);
    }

    #[test]
    fn selection_should_scroll_an_offscreen_item_into_view() {
        let mut l = list(100);
        l.select(50);
        assert_eq!(l.selected, Some(50));
        assert_eq!(l.offset(), 1940.0);
        l.select(48);
        assert_eq!(l.offset(), 1920.0);
        l.move_selection(1);
        assert_eq!(l.selected, Some(49));
    }

    #[test]
    fn hit_should_reject_points_outside_the_viewport() {
        let l = list(10);
        assert_eq!(l.hit(-1.0, 10.0, 0.0, 0.0, 200.0), None);
        assert_eq!(l.hit(10.0, -1.0, 0.0, 0.0, 200.0), None);
        assert_eq!(l.hit(10.0, 100.0, 0.0, 0.0, 200.0), None);
        assert_eq!(l.hit(10.0, 99.9, 0.0, 0.0, 200.0), Some(2));
    }

    #[test]
    fn a_zero_viewport_should_virtualize_nothing() {
        let mut l = ListView::new(vec!["a".to_owned()]);
        assert_eq!(l.visible(), 0..0);
        assert!(l.arrange(0.0, 0.0, 200.0).is_empty());
        l.set_viewport(40.0);
        assert_eq!(l.visible(), 0..1);
    }

    #[test]
    fn an_exact_multiple_boundary_should_not_include_the_next_row() {
        let mut l = ListView::new((0..10).map(|i| i.to_string()).collect());
        l.set_viewport(80.0);
        l.scroll_to(40.0);
        assert_eq!(l.visible(), 0..4);
    }

    #[test]
    fn a_viewport_shorter_than_a_row_should_align_the_top() {
        let mut l = list(10);
        l.set_viewport(10.0);
        l.select(5);
        assert_eq!(l.offset(), 230.0);
    }

    #[test]
    fn an_empty_list_should_ignore_hit_and_navigation() {
        let mut l = ListView::new(Vec::new());
        l.set_viewport(100.0);
        assert_eq!(l.hit(10.0, 10.0, 0.0, 0.0, 200.0), None);
        l.move_selection(-1);
        l.move_selection(1);
        assert_eq!(l.selected, None);
        assert!(l.arrange(0.0, 0.0, 200.0).is_empty());
    }

    #[test]
    fn a_tiny_list_should_not_need_scrolling() {
        let mut l = list(2);
        l.scroll_to(1000.0);
        assert_eq!(l.offset(), 0.0);
        l.move_selection(5);
        assert_eq!(l.selected, Some(0));
        l.move_selection(1);
        assert_eq!(l.selected, Some(1));
    }
}
