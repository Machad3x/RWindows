//! Start-menu-style "Pinned" row on Mica: hover a tile to see the WinUI 83 ms brush transition.
//! Idle = blocked in GetMessageW (0% CPU); frames render only while a tween runs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::easing::{self, Tween};
use fluent_win::gfx::{Surface, rect};
use fluent_win::theme::{self, Theme};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::DirectWrite::{DWRITE_TEXT_ALIGNMENT_CENTER, IDWriteTextFormat};
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW, SWP_NOACTIVATE,
    SIZE_MINIMIZED, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_DESTROY, WM_DPICHANGED,
    WM_DWMCOLORIZATIONCOLORCHANGED, WM_MOUSEMOVE, WM_PAINT, WM_QUIT,
    WM_SETTINGCHANGE, WM_SIZE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};
use windows::core::{PCWSTR, Result, w};

const TILES: [&str; 4] = ["Edge", "Terminal", "Configurações", "Explorador"];
const PAD: f32 = 32.0;
const TILE_W: f32 = 96.0;
const TILE_H: f32 = 84.0;
const GAP: f32 = 4.0;

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    header: IDWriteTextFormat,
    label: IDWriteTextFormat,
    hover: [Tween; TILES.len()],
    hot: Option<usize>,
    tracking: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn tile_rect(i: usize) -> D2D_RECT_F {
    let x = PAD + i as f32 * (TILE_W + GAP);
    rect(x, 64.0, x + TILE_W, 64.0 + TILE_H)
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (s, t) = (&mut self.surface, &self.theme);
        s.begin();
        s.text("Fixado", &self.header, rect(PAD, 24.0, PAD + 300.0, 48.0), t.text_primary);
        for (i, name) in TILES.iter().enumerate() {
            let r = tile_rect(i);
            let v = self.hover[i].value();
            if v > 0.0 {
                let mut c = t.subtle_hover;
                c.a *= v;
                s.fill_round(r, theme::CONTROL_RADIUS, c);
            }
            // Placeholder for the app icon (32x32, like Start).
            let x = (r.left + r.right) / 2.0 - 16.0;
            s.fill_round(rect(x, r.top + 12.0, x + 32.0, r.top + 44.0), theme::CONTROL_RADIUS, t.accent);
            s.text(name, &self.label, rect(r.left + 4.0, r.top + 52.0, r.right - 4.0, r.bottom), t.text_primary);
        }
        s.end()
    }

    fn animating(&self) -> bool {
        self.hover.iter().any(Tween::running)
    }

    fn set_hot(&mut self, hot: Option<usize>) {
        if hot == self.hot {
            return;
        }
        self.hot = hot;
        for (i, tween) in self.hover.iter_mut().enumerate() {
            let to = if hot == Some(i) { 1.0 } else { 0.0 };
            tween.retarget(to, easing::FASTER, easing::LINEAR);
        }
    }

    fn reload_theme(&mut self) -> Result<()> {
        self.theme = Theme::system();
        backdrop::set_dark(self.hwnd, self.theme.dark)?;
        self.render()
    }
}

/// Runs `f` on the app unless it is missing or already borrowed (re-entrant message).
fn with_app(f: impl FnOnce(&mut App) -> Result<()>) {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win demo: {e}");
        }
    });
}

fn loword(v: isize) -> i16 {
    (v & 0xffff) as i16
}

fn hiword(v: isize) -> i16 {
    ((v >> 16) & 0xffff) as i16
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(|a| a.render());
        }
        // Minimized reports 0x0: keep the swap chain as is until restore.
        WM_SIZE if wp.0 as u32 == SIZE_MINIMIZED => {}
        WM_SIZE => with_app(|a| {
            a.surface.resize(loword(lp.0) as u16 as u32, hiword(lp.0) as u16 as u32)?;
            a.render()
        }),
        WM_MOUSEMOVE => with_app(|a| {
            if !a.tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe { TrackMouseEvent(&mut tme)? };
                a.tracking = true;
            }
            let k = a.surface.scale();
            let (x, y) = (f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k);
            let hot = (0..TILES.len()).find(|&i| {
                let r = tile_rect(i);
                x >= r.left && x < r.right && y >= r.top && y < r.bottom
            });
            a.set_hot(hot);
            Ok(())
        }),
        WM_MOUSELEAVE => with_app(|a| {
            a.tracking = false;
            a.set_hot(None);
            Ok(())
        }),
        WM_DPICHANGED => {
            with_app(|a| a.surface.set_dpi(f32::from(loword(wp.0 as isize) as u16)));
            // Outside with_app: SetWindowPos sends WM_SIZE synchronously.
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        WM_SETTINGCHANGE
            if lp.0 != 0
                && unsafe { PCWSTR(lp.0 as *const u16).to_string() }
                    .is_ok_and(|s| s == "ImmersiveColorSet") =>
        {
            with_app(App::reload_theme);
        }
        WM_DWMCOLORIZATIONCOLORCHANGED => with_app(App::reload_theme),
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-demo");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });

        let k = GetDpiForSystem() as f32 / 96.0;
        let width = (2.0 * PAD + 4.0 * TILE_W + 3.0 * GAP + 16.0) * k;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            width as i32, (260.0 * k) as i32, None, None, Some(instance.into()), None)?;

        let theme = Theme::system();
        // Non-fatal: before Win11 22H2 there is no system backdrop; content still draws.
        if let Err(e) = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark) {
            eprintln!("fluent-win demo: backdrop unavailable: {e}");
        }
        let surface = Surface::new(hwnd)?;
        let header = surface.format(&theme::BODY_STRONG)?;
        let label = surface.format(&theme::CAPTION)?;
        label.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        let mut app = App {
            hwnd, surface, theme, header, label,
            hover: [Tween::idle(0.0); TILES.len()],
            hot: None,
            tracking: false,
        };
        app.render()?;
        APP.with(|cell| *cell.borrow_mut() = Some(app));

        let result = message_loop();
        // Release GPU objects while the process is still running: dropped later, in
        // thread-local teardown, they turned the exit code into a DXGI HRESULT (0x87A).
        APP.with(|cell| drop(cell.borrow_mut().take()));
        result
    }
}

/// Pumps messages until WM_QUIT; renders frames only while a transition runs.
fn message_loop() -> Result<()> {
    unsafe {
        let mut msg = MSG::default();
        let mut was_animating = false;
        loop {
            let mut animating = false;
            with_app(|a| {
                animating = a.animating();
                Ok(())
            });
            if animating || was_animating {
                // Last frame after a tween ends lands exactly on its target value.
                with_app(|a| a.render());
            }
            was_animating = animating;
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return Ok(());
                    }
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    return Ok(());
                }
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
