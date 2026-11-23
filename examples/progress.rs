//! WinUI RadioButton, ProgressBar and ProgressRing on Mica.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::button::{FocusKind, Input, Key};
use fluent_win::gfx::{Surface, rect};
use fluent_win::icons::{IconCache, draw as draw_icon};
use fluent_win::panel::Formats;
use fluent_win::progress::{ProgressBar, ProgressRing};
use fluent_win::radio_button::{MIN_WIDTH, ROW_HEIGHT, RadioButton, check_one};
use fluent_win::text_box::TextBox;
use fluent_win::theme::Theme;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_A, VK_BACK, VK_C, VK_CONTROL, VK_DELETE, VK_END, VK_HOME, VK_LEFT, VK_RIGHT,
    VK_V, VK_X, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    KillTimer, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SIZE_MAXIMIZED, SIZE_MINIMIZED,
    SIZE_RESTORED, SWP_NOACTIVATE, SWP_NOZORDER, SetTimer, SetWindowPos, TranslateMessage,
    WM_CHAR, WM_DESTROY, WM_DPICHANGED, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_PAINT, WM_SETFOCUS, WM_SIZE, WM_TIMER,
    WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{Result, w};

/// Icon source from the actual system directory, not a hardcoded C: path.
fn notepad_path() -> String {
    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    let directory = String::from_utf16_lossy(&buffer[..length]);
    format!("{directory}\\notepad.exe")
}

const PAD: f32 = 32.0;
const GAP: f32 = 12.0;
const TIMER: usize = 1;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    surface: Surface,
    theme: Theme,
    formats: Formats,
    radios: Vec<RadioButton>,
    bar: ProgressBar,
    ring: ProgressRing,
    text: TextBox,
    icon: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>,
    icon_gen: u64,
    /// First half of a UTF-16 surrogate pair from WM_CHAR.
    pending_surrogate: Option<u16>,
    time: f32,
    blink: bool,
}

fn loword(v: isize) -> i16 {
    (v & 0xffff) as i16
}

fn hiword(v: isize) -> i16 {
    ((v >> 16) & 0xffff) as i16
}

fn with_app(f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win progress: {e}");
        }
    });
    LRESULT(0)
}

impl App {
    fn render(&mut self) -> Result<()> {
        // Device loss replaces the D2D context: reload the device-bound bitmap.
        let generation = self.surface.generation();
        if generation != self.icon_gen {
            let mut cache = IconCache::new();
            if let Ok(icon) = cache.get(&self.surface, &notepad_path(), 32) {
                self.icon = Some(icon.clone());
                self.icon_gen = generation;
            }
        }
        let (s, t) = (&mut self.surface, &self.theme);
        s.begin();
        if let Some(c) = t.high_contrast {
            let (w, h) = s.size();
            s.fill_round(rect(0.0, 0.0, w, h), 0.0, c.window);
        }
        for (i, radio) in self.radios.iter().enumerate() {
            let y = PAD + i as f32 * (ROW_HEIGHT + GAP);
            radio.draw(s, t, &self.formats, rect(PAD, y, PAD + MIN_WIDTH, y + ROW_HEIGHT));
        }
        let y = PAD + 3.0 * (ROW_HEIGHT + GAP) + GAP;
        self.bar.draw(s, t, rect(PAD, y, PAD + 240.0, y + 3.0));
        self.ring.draw(s, t, self.time);
        self.text.draw(s, t, &self.formats,
            rect(PAD, 250.0, PAD + 240.0, 282.0), self.blink)?;
        if let Some(icon) = &self.icon {
            draw_icon(s, icon, 300.0, 24.0, 32.0);
        }
        s.end()
    }

    fn input(&mut self, x: f32, y: f32, down: bool) -> Result<()> {
        if !down {
            let mut clicked = None;
            for (i, radio) in self.radios.iter_mut().enumerate() {
                let top = PAD + i as f32 * (ROW_HEIGHT + GAP);
                let local = (x - PAD, y - top);
                if radio.handle(Input::PointerUp(local.0, local.1)) {
                    clicked = Some(i);
                }
            }
            if let Some(i) = clicked {
                check_one(&mut self.radios, i);
            }
            return self.render();
        }
        let mut target = None;
        for (i, radio) in self.radios.iter_mut().enumerate() {
            let top = PAD + i as f32 * (ROW_HEIGHT + GAP);
            let local_x = x - PAD;
            let local_y = y - top;
            radio.handle(Input::PointerDown(local_x, local_y));
            if (0.0..MIN_WIDTH).contains(&local_x) && (0.0..ROW_HEIGHT).contains(&local_y) {
                target = Some(i);
            }
        }
        if let Some(i) = target {
            check_one(&mut self.radios, i);
        }
        self.render()
    }

    /// Accepts one UTF-16 unit from WM_CHAR and joins surrogate pairs.
    fn unit(&mut self, unit: u16) -> Result<()> {
        let decoded = if let Some(high) = self.pending_surrogate.take() {
            if (0xDC00..0xE000).contains(&unit) {
                let scalar = 0x10000
                    + (((u32::from(high) - 0xD800) << 10) | (u32::from(unit) - 0xDC00));
                char::from_u32(scalar)
            } else {
                None
            }
        } else if (0xD800..0xDC00).contains(&unit) {
            self.pending_surrogate = Some(unit);
            None
        } else {
            char::from_u32(u32::from(unit))
        };
        if let Some(c) = decoded
            && !c.is_control()
        {
            self.text.insert(&c.to_string());
        }
        self.render()
    }

    fn key(&mut self, vk: VIRTUAL_KEY) -> Result<()> {
        let ctrl = unsafe { GetKeyState(i32::from(VK_CONTROL.0)) } < 0;
        if ctrl {
            match vk {
                VK_A => self.text.select_all(),
                VK_C => { _ = self.text.copy(); }
                VK_X => { _ = self.text.cut(); }
                VK_V => { _ = self.text.paste(); }
                _ => {}
            }
        } else {
            match vk {
                VK_BACK => self.text.backspace(),
                VK_DELETE => self.text.delete(),
                VK_LEFT => { self.text.handle(Input::KeyDown(Key::Left)); }
                VK_RIGHT => { self.text.handle(Input::KeyDown(Key::Right)); }
                VK_HOME => { self.text.handle(Input::KeyDown(Key::Home)); }
                VK_END => { self.text.handle(Input::KeyDown(Key::End)); }
                _ => {}
            }
        }
        self.render()
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::render)
        }
        WM_LBUTTONDOWN => with_app(|a| {
            let k = a.surface.scale();
            a.input(f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k, true)
        }),
        WM_LBUTTONUP => with_app(|a| {
            let k = a.surface.scale();
            a.input(f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k, false)
        }),
        WM_CHAR => with_app(|a| a.unit(wp.0 as u16)),
        WM_SETFOCUS => with_app(|a| {
            a.text.handle(Input::Focus(FocusKind::Keyboard));
            a.render()
        }),
        WM_KILLFOCUS => with_app(|a| {
            a.text.handle(Input::Blur);
            a.render()
        }),
        WM_KEYDOWN => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16))),
        WM_SIZE if wp.0 as u32 == SIZE_MINIMIZED => unsafe {
            // No frames while minimized: the ring animation can wait.
            _ = KillTimer(Some(hwnd), TIMER);
            LRESULT(0)
        },
        WM_SIZE if wp.0 as u32 == SIZE_RESTORED || wp.0 as u32 == SIZE_MAXIMIZED => {
            // The swap chain follows the window, so maximising is not a blank frame.
            with_app(|a| {
                a.surface.resize(loword(lp.0) as u16 as u32, hiword(lp.0) as u16 as u32)?;
                a.render()
            });
            unsafe { _ = SetTimer(Some(hwnd), TIMER, 33, None) };
            LRESULT(0)
        }
        WM_DPICHANGED => {
            with_app(|a| a.surface.set_dpi(f32::from(loword(wp.0 as isize) as u16)));
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE);
            }
            with_app(App::render)
        }
        WM_TIMER if wp.0 == TIMER => with_app(|a| {
            a.time += 1.0 / 30.0;
            a.blink = ((a.time * 1.6) as u32).is_multiple_of(2);
            a.render()
        }),
        WM_DESTROY => unsafe {
            _ = KillTimer(Some(hwnd), TIMER);
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-progress");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Progress"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            (400.0 * k) as i32, (320.0 * k) as i32, None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let radios = vec![
            RadioButton::new("Claro", true),
            RadioButton::new("Escuro", false),
            RadioButton::new("Sistema", false),
        ];
        let bar = ProgressBar::new(60.0, 100.0);
        let ring = ProgressRing { center: (300.0, 200.0), radius: 24.0, value: None };
        let mut text = TextBox::new("");
        text.placeholder = "Digite algo...".to_owned();
        let mut icons = IconCache::new();
        let icon = icons.get(&surface, &notepad_path(), 32).ok().cloned();
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { surface, theme, formats, radios, bar, ring, text, icon, icon_gen: 0, pending_surrogate: None, time: 0.0, blink: true })
        });
        with_app(App::render);
        _ = SetTimer(Some(hwnd), TIMER, 33, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|cell| drop(cell.borrow_mut().take()));
        Ok(())
    }
}
