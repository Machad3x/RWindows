//! Fase 2: a taskbar-like appbar that reserves the bottom of the work area
//! and shows the system clock/date in the user's locale. Explorer stays alive.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::time::Duration;

use fluent_win::app_bar::{self, AppBar};
use fluent_win::backdrop;
use fluent_win::flyout;
use fluent_win::gfx::{Surface, rect};
use fluent_win::icons;
use fluent_win::panel::Formats;
use fluent_win::theme::Theme;
use fluent_win::window_list::{self, ShellHook, WindowInfo};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint, ValidateRect,
};
use windows::Win32::Globalization::{
    DATE_SHORTDATE, GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_NAME_FORMAT, QueryFullProcessImageNameW, OpenProcess,
};
use windows::Win32::System::SystemInformation::GetSystemTime;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::ABN_POSCHANGED;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
    GetWindowRect, GetWindowThreadProcessId, HSHELL_HIGHBIT, HSHELL_MONITORCHANGED, HSHELL_REDRAW,
    HSHELL_WINDOWACTIVATED, HSHELL_WINDOWCREATED, HSHELL_WINDOWDESTROYED, IDC_ARROW, IsIconic,
    KillTimer, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer,
    SetWindowPos, ShowWindow, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE, SWP_NOZORDER,
    TranslateMessage, WM_APP, WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT, WM_TIMER,
    WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const APPBAR_CALLBACK: u32 = WM_APP + 0x60;
const CLOCK_TIMER: usize = 1;
/// `HSHELL_FLASH` is `HSHELL_REDRAW` with the high bit set (winuser.h).
const HSHELL_FLASH: u32 = HSHELL_REDRAW | HSHELL_HIGHBIT;

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
    /// Kept alive so `Drop` deregisters the shell hook.
    _hook: ShellHook,
    windows: Vec<WindowInfo>,
    icons: Vec<Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>>,
    active: Option<HWND>,
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

/// Icon of a top-level window: the shell icon of its executable. `WM_GETICON`
/// is unreliable for modern apps and its HICON mask needs extra compositing.
fn window_icon(surface: &Surface, hwnd: HWND) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return None;
        };
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_FORMAT(0),
            windows::core::PWSTR(buffer.as_mut_ptr()), &mut len).is_ok();
        _ = CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        let mut cache = icons::IconCache::new();
        cache.get(surface, &path, 32).ok().cloned()
    }
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

    /// Rebuilds the task-button list from all top-level windows.
    fn refresh_windows(&mut self) {
        let surface = &self.surface;
        let list: Vec<WindowInfo> = window_list::enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter(|info| info.hwnd != self.hwnd && !info.title.is_empty())
            .collect();
        self.icons = list.iter().map(|info| window_icon(surface, info.hwnd)).collect();
        self.windows = list;
    }

    /// Handles the dynamic SHELLHOOK notification.
    fn shell_event(&mut self, code: usize, hwnd: HWND) -> Result<()> {
        let refresh = matches!(code as u32,
            HSHELL_WINDOWCREATED | HSHELL_WINDOWDESTROYED | HSHELL_REDRAW | HSHELL_FLASH
            | HSHELL_MONITORCHANGED);
        if code as u32 == HSHELL_WINDOWACTIVATED {
            self.active = Some(hwnd);
        } else if !refresh {
            return Ok(());
        }
        self.refresh_windows();
        self.render()
    }

    /// Activates a task button, or minimizes the window when it is already active.
    fn activate_button(&mut self, x: f32) -> Result<()> {
        let width = 160.0;
        let gap = 8.0;
        let index = ((x - 16.0) / (width + gap)) as isize;
        let Some(info) = self.windows.get(index.max(0) as usize) else { return Ok(()) };
        if index < 0 || x - 16.0 - index as f32 * (width + gap) > width {
            return Ok(());
        }
        let hwnd = info.hwnd;
        unsafe {
            if self.active == Some(hwnd) && !IsIconic(hwnd).as_bool() {
                _ = ShowWindow(hwnd, SW_MINIMIZE);
            } else {
                _ = ShowWindow(hwnd, SW_RESTORE);
                _ = SetForegroundWindow(hwnd);
            }
        }
        Ok(())
    }

    fn render(&mut self) -> Result<()> {
        let (w, h) = self.surface.size();
        let style = flyout::style(&self.theme, self.backdrop);
        let buttons: Vec<(String, bool, Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>)> =
            self.windows.iter().enumerate()
                .map(|(i, info)| (info.title.clone(), self.active == Some(info.hwnd), self.icons.get(i).cloned().flatten()))
                .collect();
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        s.begin();
        s.with_opacity(1.0, |s| {
            flyout::draw_chrome(s, &style, (w, h));
            Ok(())
        })?;
        if let Some(c) = t.high_contrast {
            s.fill_round(rect(0.0, 0.0, w, h), 0.0, c.window);
        }
        let button_width = 160.0;
        let button_gap = 8.0;
        let task_area = rect(16.0, 4.0, w - 260.0, h - 4.0);
        s.with_clip(task_area, |s| {
            for (i, (title, active, icon)) in buttons.iter().enumerate() {
                let x = 16.0 + i as f32 * (button_width + button_gap);
                let r = rect(x, 4.0, x + button_width, h - 4.0);
                if *active {
                    s.fill_round(r, 4.0, t.subtle_hover);
                    s.fill_round(rect(x, r.bottom - 3.0, x + button_width, r.bottom), 1.0, t.accent);
                }
                let text_x = if let Some(bitmap) = icon {
                    icons::draw(s, bitmap, x + 6.0, (h - 16.0) / 2.0, 16.0);
                    x + 28.0
                } else {
                    x + 10.0
                };
                let label: String = title.chars().take(20).collect();
                s.with_clip(r, |s| {
                    s.text_no_wrap(&label, &f.switch, rect(text_x, r.top, r.right - 10.0, r.bottom), t.text_primary);
                });
            }
        });
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
        WM_LBUTTONDOWN => with_app(|a| {
            let k = a.surface.scale();
            a.activate_button((lp.0 & 0xffff) as f32 / k)
        }),
        WM_TIMER if wp.0 == CLOCK_TIMER => with_app(App::render),
        WM_DESTROY => unsafe {
            _ = KillTimer(Some(hwnd), CLOCK_TIMER);
            // Dropping `App` unregisters the appbar through `AppBar::drop`.
            APP.with(|cell| drop(cell.borrow_mut().take()));
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ if msg == window_list::shellhook_message() => with_app(|a| {
            a.shell_event(wp.0, HWND(lp.0 as *mut core::ffi::c_void))
        }),
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
        let hook = ShellHook::new(hwnd)?;
        let windows = window_list::enumerate()?;
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App {
                hwnd, surface, theme, formats, bar, backdrop, _hook: hook, windows, icons: Vec::new(), active: None,
            })
        });
        with_app(|a| { a.refresh_windows(); Ok(()) });
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
