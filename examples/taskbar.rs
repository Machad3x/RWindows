//! Fase 2: a taskbar-like appbar that reserves the bottom of the work area
//! and shows the system clock/date in the user's locale. Explorer stays alive.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::time::Duration;

use fluent_win::app_bar::{self, AppBar};
use fluent_win::backdrop;
use fluent_win::flyout;
use fluent_win::gfx::{Surface, rect};
use fluent_win::panel::Formats;
use fluent_win::theme::Theme;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint, ValidateRect,
};
use windows::Win32::Globalization::{
    DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::ABN_POSCHANGED;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowRect, IDC_ARROW,
    KillTimer, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SetTimer, SetWindowPos,
    SWP_NOACTIVATE, SWP_NOZORDER, TranslateMessage, WM_APP, WM_DESTROY, WM_PAINT,
    WM_TIMER, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const APPBAR_CALLBACK: u32 = WM_APP + 0x60;
const CLOCK_TIMER: usize = 1;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    formats: Formats,
    bar: AppBar,
    backdrop: bool,
}

fn with_app(f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win taskbar: {e}");
        }
    });
    LRESULT(0)
}

fn locale_text() -> (String, String) {
    let now = unsafe { GetSystemTime() };
    let mut time = [0u16; 64];
    let mut date = [0u16; 128];
    unsafe {
        _ = GetTimeFormatEx(None, TIME_NOSECONDS, Some(&now), None, Some(&mut time));
        _ = GetDateFormatEx(None, DATE_SHORTDATE, Some(&now), None, Some(&mut date), None);
    }
    let read = |buf: &[u16]| -> String {
        let len = buf.iter().position(|&u| u == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..len])
    };
    (read(&time), read(&date))
}

impl App {
    /// Re-query the work area and place the bar; also called on ABN_POSCHANGED.
    fn reposition(&mut self) -> Result<()> {
        let k = self.surface.scale();
        let height = (app_bar::HEIGHT * k).round() as i32;
        let work = unsafe {
            let monitor = MonitorFromPoint(windows::Win32::Foundation::POINT { x: 0, y: 0 },
                MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            _ = GetMonitorInfoW(monitor, &mut info);
            info.rcWork
        };
        // SHAppBarMessage(ABM_SETPOS) shrinks the work area, which sends us
        // ABN_POSCHANGED. Once our top edge sits at the new work bottom we are
        // already placed: committing again would walk the bar up the screen.
        let current = unsafe {
            let mut r = RECT::default();
            _ = GetWindowRect(self.hwnd, &mut r);
            r
        };
        if current.top == work.bottom && current.right == work.right {
            return self.render();
        }
        let desired = app_bar::bottom_rect(work, height);
        let final_rect = self.bar.set_bottom(desired);
        unsafe {
            SetWindowPos(self.hwnd, None, final_rect.left, final_rect.top,
                final_rect.right - final_rect.left, final_rect.bottom - final_rect.top,
                SWP_NOZORDER | SWP_NOACTIVATE)?;
        }
        self.render()
    }

    fn render(&mut self) -> Result<()> {
        let (w, h) = self.surface.size();
        let style = flyout::style(&self.theme, self.backdrop);
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        s.begin();
        s.with_opacity(1.0, |s| {
            flyout::draw_chrome(s, &style, (w, h));
            Ok(())
        })?;
        if let Some(c) = t.high_contrast {
            s.fill_round(rect(0.0, 0.0, w, h), 0.0, c.window);
        }
        let (time, date) = locale_text();
        let text = format!("{date}  {time}");
        let label_w = 220.0;
        s.text_no_wrap(&text, &f.switch,
            rect(w - label_w - 16.0, 0.0, w - 16.0, h), t.text_primary);
        s.end()
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::render)
        }
        APPBAR_CALLBACK if wp.0 as u32 == ABN_POSCHANGED => with_app(App::reposition),
        WM_TIMER if wp.0 == CLOCK_TIMER => with_app(App::render),
        WM_DESTROY => unsafe {
            _ = KillTimer(Some(hwnd), CLOCK_TIMER);
            // Dropping `App` unregisters the appbar through `AppBar::drop`.
            APP.with(|cell| drop(cell.borrow_mut().take()));
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
        let class = w!("fluent-win-taskbar");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Taskbar"),
            WS_POPUP | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            (800.0 * k) as i32, (app_bar::HEIGHT * k).round() as i32,
            None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        let backdrop = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark).is_ok();
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let bar = AppBar::new(hwnd, APPBAR_CALLBACK)?;
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { hwnd, surface, theme, formats, bar, backdrop })
        });
        with_app(App::reposition);
        _ = SetTimer(Some(hwnd), CLOCK_TIMER, Duration::from_secs(1).as_millis() as u32, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
