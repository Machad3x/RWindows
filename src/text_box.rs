//! WinUI `TextBox`: editing model, selection, clipboard and caret. The window
//! feeds Unicode characters from `WM_CHAR` with [`TextBox::insert`]; IME
//! composition preview is left to the app.

use std::ptr;

use windows::Win32::Foundation::{E_POINTER, HANDLE, HGLOBAL, GlobalFree};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::core::Result;

use crate::button::{FocusKind, Input, Key};
use crate::gfx::{Surface, rect};
use crate::panel::Formats;
use crate::theme::{self, Theme};

/// `TextBox` corner radius and content padding (`ControlCornerRadius`, 8,6).
pub const RADIUS: f32 = theme::CONTROL_RADIUS;
/// Content padding, left and top.
pub const PADDING: (f32, f32) = (8.0, 6.0);
/// Height of a single-line TextBox (`ContentThemeFontSize` 14 + padding).
pub const HEIGHT: f32 = 32.0;

/// A single-line editable text box. Cursor and selection are character indices.
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    chars: Vec<char>,
    /// Caret position, in `0..=chars.len()`.
    pub cursor: usize,
    /// Selected range `[start, end)` in character indices.
    pub selection: Option<(usize, usize)>,
    /// `IsEnabled`.
    pub enabled: bool,
    /// Shown when the text is empty.
    pub placeholder: String,
    focused: bool,
    hot: bool,
    /// Last measured prefix width: (caret index, text length, width in DIPs).
    prefix: Option<(usize, usize, f32)>,
}

impl Default for TextBox {
    fn default() -> Self {
        Self::new("")
    }
}

impl TextBox {
    /// Creates a text box with initial content.
    pub fn new(text: impl Into<String>) -> Self {
        let chars: Vec<char> = text.into().chars().collect();
        let cursor = chars.len();
        Self { chars, cursor, selection: None, enabled: true, placeholder: String::new(), focused: false, hot: false, prefix: None }
    }

    /// Current content.
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Number of characters.
    pub fn len(&self) -> usize {
        self.chars.len()
    }

    /// True when there is no content.
    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    /// Replaces the whole content and places the caret at the end.
    pub fn set_text(&mut self, text: &str) {
        self.prefix = None;
        self.chars = text.chars().collect();
        self.cursor = self.chars.len();
        self.selection = None;
    }

    /// Inserts `text` at the caret, replacing any selection.
    pub fn insert(&mut self, text: &str) {
        if !self.enabled {
            return;
        }
        self.prefix = None;
        let (start, end) = self.selection.take().unwrap_or((self.cursor, self.cursor));
        let insert: Vec<char> = text.chars().filter(|&c| c != '\r' && c != '\n').collect();
        self.chars.splice(start..end, insert.iter().copied());
        self.cursor = start + insert.len();
    }

    /// Deletes the character before the caret (Backspace).
    pub fn backspace(&mut self) {
        if !self.enabled {
            return;
        }
        self.prefix = None;
        if let Some((start, end)) = self.selection.take() {
            self.chars.drain(start..end);
            self.cursor = start;
        } else if self.cursor > 0 {
            self.chars.remove(self.cursor - 1);
            self.cursor -= 1;
        }
    }

    /// Deletes the character after the caret (Delete).
    pub fn delete(&mut self) {
        if !self.enabled {
            return;
        }
        self.prefix = None;
        if let Some((start, end)) = self.selection.take() {
            self.chars.drain(start..end);
            self.cursor = start;
        } else if self.cursor < self.chars.len() {
            self.chars.remove(self.cursor);
        }
    }

    /// Moves the caret by `delta` characters, clearing the selection.
    pub fn move_cursor(&mut self, delta: isize) {
        if !self.enabled {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, self.chars.len() as isize) as usize;
        self.cursor = next;
        self.selection = None;
    }

    /// `Ctrl+A`.
    pub fn select_all(&mut self) {
        if !self.enabled {
            return;
        }
        self.selection = Some((0, self.chars.len()));
        self.cursor = self.chars.len();
    }

    /// Copies the selection to the clipboard and deletes it; does nothing when
    /// there is no selection.
    pub fn cut(&mut self) -> Result<()> {
        let selected = self.copy()?;
        if selected {
            self.delete();
        }
        Ok(())
    }

    /// Copies the selection to the clipboard. Returns `false` when empty.
    pub fn copy(&self) -> Result<bool> {
        let Some((start, end)) = self.selection else { return Ok(false) };
        let len = self.chars.len();
        let start = start.min(len);
        let end = end.max(start).min(len);
        if start == end {
            return Ok(false);
        }
        let text: String = self.chars[start..end].iter().collect();
        set_clipboard(&text)?;
        Ok(true)
    }

    /// Pastes clipboard text at the caret.
    pub fn paste(&mut self) -> Result<()> {
        if let Some(text) = clipboard_text()? {
            self.insert(&text);
        }
        Ok(())
    }

    /// Routes navigation/focus keys; Unicode characters come through `insert`.
    pub fn handle(&mut self, input: Input) -> bool {
        if !self.enabled {
            return false;
        }
        match input {
            Input::Focus(FocusKind::Keyboard) => self.focused = true,
            Input::Blur => {
                self.focused = false;
                self.hot = false;
            }
            Input::PointerMove(x, y) => self.hot = x >= 0.0 && y >= 0.0,
            Input::KeyDown(Key::Left) => self.move_cursor(-1),
            Input::KeyDown(Key::Right) => self.move_cursor(1),
            Input::KeyDown(Key::Home) => self.move_cursor(-(self.cursor as isize)),
            Input::KeyDown(Key::End) => self.move_cursor((self.chars.len() - self.cursor) as isize),
            _ => {}
        }
        true
    }

    /// Draws the border, text, selection and blinking caret.
    /// Width of `chars[..index]`, cached between frames of the same text.
    fn prefix_width(&mut self, s: &Surface, format: &IDWriteTextFormat, index: usize) -> Result<f32> {
        let len = self.chars.len();
        if let Some((cached_index, cached_len, width)) = self.prefix
            && cached_index == index && cached_len == len
        {
            return Ok(width);
        }
        let prefix: String = self.chars[..index].iter().collect();
        let (width, _) = s.measure(&prefix, format)?;
        self.prefix = Some((index, len, width));
        Ok(width)
    }

    /// Draws the border, text, selection and blinking caret.
    pub fn draw(&mut self, s: &Surface, t: &Theme, f: &Formats, r: D2D_RECT_F, blink: bool) -> Result<()> {
        let bg = if self.enabled { t.control_fill } else { t.control_fill_disabled };
        s.fill_round(r, RADIUS, bg);
        s.stroke_round(r, RADIUS, 1.0, t.stroke);
        let text_rect = rect(r.left + PADDING.0, r.top + PADDING.1, r.right - PADDING.0, r.bottom - PADDING.1);
        if self.chars.is_empty() {
            s.text_no_wrap(&self.placeholder, &f.body, text_rect, t.text_secondary);
            return Ok(());
        }
        let len = self.chars.len();
        let cursor = self.cursor.min(len);
        let (start, end) = self.selection
            .map(|(a, b)| (a.min(len), b.max(a).min(len)))
            .unwrap_or((cursor, cursor));
        if end > start {
            let w = self.prefix_width(s, &f.body, start)?;
            let (sw, _) = s.measure(&self.chars[start..end].iter().collect::<String>(), &f.body)?;
            let x = text_rect.left + w;
            s.fill_round(rect(x, text_rect.top, x + sw, text_rect.bottom), 0.0, t.accent);
        }
        s.text_no_wrap(&self.text(), &f.body, text_rect, if self.enabled { t.text_primary } else { t.text_disabled });
        if self.focused && blink && end == start {
            let w = self.prefix_width(s, &f.body, cursor)?;
            let x = text_rect.left + w;
            s.fill_round(rect(x, text_rect.top, x + 1.0, text_rect.bottom), 0.0, t.text_primary);
        }
        Ok(())
    }
}

/// Always releases the clipboard, even on an error path.
struct Clipboard;

impl Clipboard {
    fn open() -> Result<Self> {
        unsafe { OpenClipboard(None)? };
        Ok(Self)
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe { _ = CloseClipboard() };
    }
}

fn set_clipboard(text: &str) -> Result<()> {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // Prepare the replacement before touching the clipboard: a failed allocation
    // must not destroy what the user already had copied.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2)?;
        let dst = GlobalLock(mem).cast::<u16>();
        if dst.is_null() {
            _ = GlobalFree(Some(mem));
            return Err(E_POINTER.into());
        }
        ptr::copy_nonoverlapping(units.as_ptr(), dst, units.len());
        // GlobalUnlock returns FALSE with NO_ERROR when the lock count reaches
        // zero; the windows crate surfaces that as Err(S_OK), so ignore it.
        _ = GlobalUnlock(mem);
        let _clipboard = match Clipboard::open() {
            Ok(guard) => guard,
            Err(e) => {
                _ = GlobalFree(Some(mem));
                return Err(e);
            }
        };
        if let Err(e) = EmptyClipboard() {
            _ = GlobalFree(Some(mem));
            return Err(e);
        }
        // On success the clipboard owns `mem`; on failure we still own it.
        if let Err(e) = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0))) {
            _ = GlobalFree(Some(mem));
            return Err(e);
        }
    }
    Ok(())
}

fn clipboard_text() -> Result<Option<String>> {
    let _clipboard = Clipboard::open()?;
    unsafe {
        let data = GetClipboardData(CF_UNICODETEXT.0 as u32)?;
        if data.is_invalid() {
            return Ok(None);
        }
        let global = HGLOBAL(data.0);
        let src = GlobalLock(global).cast::<u16>();
        if src.is_null() {
            return Ok(None);
        }
        let capacity = GlobalSize(global) / 2;
        let mut len = 0;
        while len < capacity && *src.add(len) != 0 {
            len += 1;
        }
        let result = String::from_utf16_lossy(std::slice::from_raw_parts(src, len));
        _ = GlobalUnlock(global);
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    static CLIPBOARD_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn clipboard_should_round_trip_unicode() {
        let _guard = CLIPBOARD_LOCK.lock().unwrap();
        let text = "olá 😀";
        set_clipboard(text).unwrap();
        assert_eq!(clipboard_text().unwrap().as_deref(), Some(text));
    }

    #[test]
    fn insert_and_backspace_should_work_at_the_caret() {
        let mut box_ = TextBox::new("abc");
        box_.insert("X");
        assert_eq!(box_.text(), "abcX");
        box_.backspace();
        assert_eq!(box_.text(), "abc");
        box_.move_cursor(-2);
        box_.delete();
        assert_eq!(box_.text(), "ac");
    }

    #[test]
    fn selection_should_be_replaced_by_an_insert() {
        let mut box_ = TextBox::new("hello world");
        box_.selection = Some((0, 5));
        box_.insert("bye");
        assert_eq!(box_.text(), "bye world");
        assert_eq!(box_.cursor, 3);
    }

    #[test]
    fn home_end_and_clamping_should_not_panic_on_an_empty_box() {
        let mut box_ = TextBox::new("");
        box_.move_cursor(-5);
        box_.move_cursor(5);
        box_.select_all();
        assert_eq!(box_.selection, Some((0, 0)));
        box_.backspace();
        assert!(box_.is_empty());
    }

    #[test]
    fn multibyte_characters_should_be_handled_by_character_index() {
        let mut box_ = TextBox::new("olá");
        box_.move_cursor(-1);
        box_.insert("!");
        assert_eq!(box_.text(), "ol!á");
    }

    #[test]
    fn disabled_box_should_ignore_text_and_navigation() {
        let mut box_ = TextBox::new("x");
        box_.enabled = false;
        box_.insert("y");
        assert_eq!(box_.text(), "x");
    }
}
