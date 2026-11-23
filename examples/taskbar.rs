//! Fase 2: a taskbar-like appbar that reserves the bottom of the work area
//! and shows the system clock/date in the user's locale. Explorer stays alive.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use fluent_win::app_bar::{self, AppBar};
use fluent_win::button::{Input, Key};
use fluent_win::backdrop;
use fluent_win::flyout;
use fluent_win::gfx::{Surface, rect};
use fluent_win::icons;
use fluent_win::menu_flyout::{Entry, Item, MenuFlyout};
use fluent_win::metrics;
use fluent_win::panel::Formats;
use fluent_win::theme::Theme;
use fluent_win::uia::{self, WM_UIA_INVOKE};
use fluent_win::window_list::{self, ShellHook, WindowInfo};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, MonitorFromRect, MonitorFromWindow, ValidateRect,
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
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Accessibility::{
    UIA_MenuItemControlTypeId, UiaReturnRawElementProvider, UiaRootObjectId,
};
use windows::Win32::UI::Shell::ABN_POSCHANGED;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, VK_DOWN, VK_END, VK_ESCAPE, VK_HOME, VK_RETURN, VK_SPACE, VK_UP,
    VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowRect, GetWindowThreadProcessId, HSHELL_HIGHBIT, HSHELL_MONITORCHANGED, HSHELL_REDRAW,
    HSHELL_WINDOWACTIVATED, HSHELL_WINDOWCREATED, HSHELL_WINDOWDESTROYED, IDC_ARROW, IsIconic,
    IsWindow, KillTimer, LoadCursorW, MSG, PostMessageW, PostQuitMessage, RegisterClassW,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, SW_MINIMIZE, SW_RESTORE, SWP_NOACTIVATE,
    SW_SHOW, SWP_NOZORDER, TranslateMessage, WA_INACTIVE, WM_ACTIVATE, WM_APP, WM_CLOSE, WM_DESTROY,
    WM_GETOBJECT, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_RBUTTONDOWN, WM_TIMER,
    WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, Result, w};

const APPBAR_CALLBACK: u32 = WM_APP + 0x60;
/// Width of one task button in DIPs, measured from the real taskbar.
const BUTTON_WIDTH: f32 = 48.0;
const CLOCK_TIMER: usize = 1;
/// `HSHELL_FLASH` is `HSHELL_REDRAW` with the high bit set (winuser.h).
const HSHELL_FLASH: u32 = HSHELL_REDRAW | HSHELL_HIGHBIT;

thread_local! {
    /// One App per monitor; each owns its appbar window and surface.
    static APP: RefCell<Vec<App>> = const { RefCell::new(Vec::new()) };
    static MENU: RefCell<Option<MenuPopup>> = const { RefCell::new(None) };
    /// Kept outside MENU: WM_GETOBJECT can arrive while MENU is borrowed.
    static MENU_UIA: RefCell<Option<uia::Shared>> = const { RefCell::new(None) };
}

/// Context menu as its own popup window, drawn with our `MenuFlyout`.
struct MenuPopup {
    hwnd: HWND,
    surface: Surface,
    menu: MenuFlyout,
    theme: Theme,
    formats: Formats,
    backdrop: bool,
    target: HWND,
}

fn close_menu() {
    MENU_UIA.with(|cell| drop(cell.borrow_mut().take()));
    let hwnd = MENU.with(|cell| cell.borrow_mut().take().map(|popup| popup.hwnd));
    if let Some(hwnd) = hwnd {
        unsafe { _ = DestroyWindow(hwnd) };
    }
}

fn menu_action(index: usize, target: HWND) {
    if !unsafe { IsWindow(Some(target)) }.as_bool() {
        return;
    }
    unsafe {
        match index {
            0 => { _ = ShowWindow(target, SW_RESTORE); _ = SetForegroundWindow(target); }
            1 => { _ = ShowWindow(target, SW_MINIMIZE); }
            2 => { _ = PostMessageW(Some(target), WM_CLOSE, WPARAM(0), LPARAM(0)); }
            _ => {}
        }
    }
}

/// Copies the highlighted entry into the UIA model.
fn sync_menu_uia() {
    let focused = MENU.with(|cell| cell.borrow().as_ref().and_then(|popup| popup.menu.focused()));
    MENU_UIA.with(|cell| {
        if let Some(model) = cell.borrow().as_ref() {
            model.borrow_mut().focused = focused;
        }
    });
}

/// Applies capture and activation from a `MenuFlyout` outcome.
fn finish_menu(out: fluent_win::menu_flyout::Outcome, hwnd: HWND) {
    sync_menu_uia();
    match out.capture {
        Some(true) => unsafe { _ = SetCapture(hwnd) },
        Some(false) => unsafe { _ = ReleaseCapture() },
        None => {}
    }
    if let Some(index) = out.invoked {
        let target = MENU.with(|cell| cell.borrow().as_ref().map(|popup| popup.target));
        close_menu();
        if let Some(target) = target {
            menu_action(index, target);
        }
    }
}

extern "system" fn menu_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            MENU.with(|cell| {
                if let Some(popup) = cell.borrow_mut().as_mut() {
                    let style = flyout::style(&popup.theme, popup.backdrop);
                    let (w, h) = popup.surface.size();
                    popup.surface.begin();
                    let _ = popup.surface.with_opacity(1.0, |s| {
                        flyout::draw_chrome(s, &style, (w, h));
                        popup.menu.draw(s, &popup.theme, &popup.formats);
                        Ok(())
                    });
                    let _ = popup.surface.end();
                }
            });
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            let input = MENU.with(|cell| {
                cell.borrow_mut().as_mut().map(|popup| {
                    let k = popup.surface.scale();
                    let x = (lp.0 & 0xffff) as f32 / k;
                    let y = ((lp.0 >> 16) & 0xffff) as f32 / k;
                    match msg {
                        WM_MOUSEMOVE => popup.menu.handle(Input::PointerMove(x, y)),
                        WM_LBUTTONDOWN => popup.menu.handle(Input::PointerDown(x, y)),
                        _ => popup.menu.handle(Input::PointerUp(x, y)),
                    }
                })
            });
            if let Some(out) = input {
                finish_menu(out, hwnd);
            }
        }
        WM_GETOBJECT if lp.0 as i32 == UiaRootObjectId => {
            let root = MENU_UIA.with(|cell| cell.borrow().as_ref().map(uia::root));
            if let Some(root) = root {
                return unsafe { UiaReturnRawElementProvider(hwnd, wp, lp, &root) };
            }
        }
        WM_UIA_INVOKE => {
            let index = wp.0;
            let target = MENU.with(|cell| cell.borrow().as_ref().map(|popup| popup.target));
            close_menu();
            if let Some(target) = target {
                menu_action(index, target);
            }
        }
        WM_KEYDOWN | WM_KEYUP => {
            let vk = VIRTUAL_KEY(wp.0 as u16);
            if vk == VK_ESCAPE && msg == WM_KEYDOWN {
                close_menu();
            } else {
                let out = MENU.with(|cell| {
                    cell.borrow_mut().as_mut().map(|popup| {
                        let input = if msg == WM_KEYDOWN {
                            Input::KeyDown(key_of(vk))
                        } else {
                            Input::KeyUp(key_of(vk))
                        };
                        popup.menu.handle(input)
                    })
                });
                if let Some(out) = out {
                    finish_menu(out, hwnd);
                }
            }
        }
        WM_ACTIVATE if u32::from(loword(wp.0 as isize) as u16) == WA_INACTIVE => close_menu(),
        WM_DESTROY => {}
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    formats: Formats,
    bar: AppBar,
    backdrop: bool,
    /// Only the first bar owns the global shell hook; Drop deregisters it.
    _hook: Option<ShellHook>,
    windows: Vec<WindowInfo>,
    icons: Vec<Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>>,
    /// Executable icons, kept across shell events so a redraw does not re-extract.
    icon_cache: icons::IconCache,
    active: Option<HWND>,
}

extern "system" fn each_monitor(monitor: HMONITOR, _hdc: HDC, _rect: *mut RECT, data: LPARAM) -> BOOL {
    unsafe {
        let monitors = &mut *(data.0 as *mut Vec<HMONITOR>);
        monitors.push(monitor);
    }
    BOOL(1)
}

fn loword(v: isize) -> i16 {
    (v & 0xffff) as i16
}

fn key_of(vk: VIRTUAL_KEY) -> Key {
    match vk {
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        VK_SPACE => Key::Space,
        VK_RETURN => Key::Enter,
        _ => Key::Other,
    }
}

fn with_app_at(hwnd: HWND, f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut apps) = cell.try_borrow_mut()
            && let Some(app) = apps.iter_mut().find(|app| app.hwnd == hwnd)
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win taskbar: {e}");
        }
    });
    LRESULT(0)
}

fn with_apps(f: impl FnOnce(&mut [App]) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut apps) = cell.try_borrow_mut()
            && let Err(e) = f(&mut apps)
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
fn window_icon(surface: &Surface, cache: &mut icons::IconCache, hwnd: HWND) -> Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1> {
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
        cache.get(surface, &path, 32).ok().cloned()
    }
}

/// Window under a task-button x coordinate (DIPs from the bar's left edge).
fn button_at(app: &App, x: f32) -> Option<HWND> {
    let width = BUTTON_WIDTH;
    // Same right limit as the drawing clip, so the clock area is not clickable.
    let (w, _) = app.surface.size();
    if x < 8.0 || x > w - 260.0 {
        return None;
    }
    let index = ((x - 8.0) / width) as isize;
    if index < 0 || x - 8.0 - index as f32 * width > width {
        return None;
    }
    app.windows.get(index as usize).map(|info| info.hwnd)
}

impl App {
    /// Opens a WinUI `MenuFlyout` next to the pointer, aimed at `target`.
    fn open_menu(&mut self, px: i32, py: i32, target: HWND) -> Result<()> {
        close_menu();
        let entries = vec![
            Entry::Item(Item::new("Restaurar")),
            Entry::Item(Item::new("Minimizar")),
            Entry::Item(Item::new("Fechar")),
        ];
        let sizes = entries.iter()
            .map(|entry| entry.measure(&self.surface, &self.formats))
            .collect::<Result<Vec<_>>>()?;
        let menu = MenuFlyout::new(entries, &sizes);
        let uia_items = ["Restaurar", "Minimizar", "Fechar"].iter().enumerate()
            .map(|(i, name)| uia::Item {
                id: i as u64,
                name: (*name).to_owned(),
                rect: menu.rects[i],
                enabled: true,
                toggle: None,
                control_type: UIA_MenuItemControlTypeId,
                range: None,
            })
            .collect();
        let (w, h) = menu.size();
        let mut screen = POINT { x: px, y: py };
        unsafe { _ = ClientToScreen(self.hwnd, &mut screen) };
        let anchor = RECT {
            left: screen.x - 8, top: screen.y - 8, right: screen.x + 8, bottom: screen.y + 8,
        };
        let (work, k) = unsafe {
            let monitor = MonitorFromRect(&anchor, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            _ = GetMonitorInfoW(monitor, &mut info);
            let (mut dx, mut dy) = (96, 96);
            _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            (info.rcWork, dx as f32 / 96.0)
        };
        let dips = |r: RECT| rect(r.left as f32 / k, r.top as f32 / k, r.right as f32 / k, r.bottom as f32 / k);
        let (placed, _) = flyout::place(dips(anchor), w, h, dips(work), flyout::Placement::Bottom);
        let origin = ((placed.left * k).round() as i32, (placed.top * k).round() as i32);
        let hwnd = unsafe {
            CreateWindowExW(
                // Activatable so it receives the keyboard (Esc/arrows) and
                // closes itself when focus moves away (light dismiss).
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
                w!("fluent-win-taskbar-menu"), w!(""), WS_POPUP, origin.0, origin.1,
                (w * k).round() as i32, (h * k).round() as i32, Some(self.hwnd), None, None, None)?
        };
        let backdrop = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, self.theme.dark).is_ok();
        let surface = Surface::new(hwnd)?;
        let model = Rc::new(RefCell::new(uia::Model {
            hwnd, scale: k, items: uia_items, focused: None,
        }));
        MENU_UIA.with(|cell| *cell.borrow_mut() = Some(model));
        MENU.with(|cell| {
            *cell.borrow_mut() = Some(MenuPopup {
                hwnd, surface, menu, theme: self.theme, formats: self.formats.clone(), backdrop, target,
            });
        });
        unsafe { _ = ShowWindow(hwnd, SW_SHOW) };
        // A menu takes the keyboard while it is open; losing it closes the menu.
        unsafe { _ = SetForegroundWindow(hwnd) };
        Ok(())
    }

    /// Re-query the work area and place the bar; also called on ABN_POSCHANGED.
    fn reposition(&mut self) -> Result<()> {
        let k = self.surface.scale();
        let (work, strip) = unsafe {
            let monitor = MonitorFromWindow(self.hwnd, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            _ = GetMonitorInfoW(monitor, &mut info);
            (info.rcWork, info.rcMonitor.bottom - info.rcWork.bottom)
        };
        let height = if strip > 0 {
            (strip as f32 / k).round() as i32
        } else {
            (app_bar::HEIGHT * k).round() as i32
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
        let cache = &mut self.icon_cache;
        let icons: Vec<_> = list.iter().map(|info| window_icon(surface, cache, info.hwnd)).collect();
        self.icons = icons;
        self.windows = list;
    }

    /// Handles the dynamic SHELLHOOK notification.
    fn apply_shell_event(&mut self, code: usize, hwnd: HWND, snapshot: Vec<WindowInfo>) -> Result<()> {
        let refresh = matches!(code as u32,
            HSHELL_WINDOWCREATED | HSHELL_WINDOWDESTROYED | HSHELL_REDRAW | HSHELL_FLASH
            | HSHELL_MONITORCHANGED);
        if code as u32 == HSHELL_WINDOWACTIVATED {
            self.active = Some(hwnd);
        } else if !refresh {
            return Ok(());
        }
        let surface = &self.surface;
        let list: Vec<WindowInfo> = snapshot.into_iter()
            .filter(|info| info.hwnd != self.hwnd && !info.title.is_empty())
            .collect();
        let cache = &mut self.icon_cache;
        let icons: Vec<_> = list.iter().map(|info| window_icon(surface, cache, info.hwnd)).collect();
        self.icons = icons;
        self.windows = list;
        self.render()
    }

    /// Activates a task button, or minimizes the window when it is already active.
    fn activate_button(&mut self, x: f32) -> Result<()> {
        let Some(hwnd) = button_at(self, x) else { return Ok(()) };
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
        // Measured on the real Windows 11 bar: 48-DIP slots with a 24-DIP
        // centred icon, no title, and a short indicator under the active one.
        let button_width = BUTTON_WIDTH;
        let task_area = rect(8.0, 0.0, w - 260.0, h);
        s.with_clip(task_area, |s| {
            for (i, (_title, active, icon)) in buttons.iter().enumerate() {
                let x = 8.0 + i as f32 * button_width;
                if *active {
                    s.fill_round(rect(x + 4.0, 0.0, x + button_width - 4.0, h), 4.0, t.subtle_hover);
                    s.fill_round(rect(x + 16.0, h - 3.0, x + button_width - 16.0, h), 1.0, t.accent);
                }
                if let Some(bitmap) = icon {
                    icons::draw(s, bitmap, x + 12.0, (h - 24.0) / 2.0, 24.0);
                }
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
            with_app_at(hwnd, App::render)
        }
        APPBAR_CALLBACK if wp.0 as u32 == ABN_POSCHANGED => with_app_at(hwnd, App::reposition),
        WM_LBUTTONDOWN => with_app_at(hwnd, |app| {
            let k = app.surface.scale();
            app.activate_button((lp.0 & 0xffff) as f32 / k)
        }),
        WM_RBUTTONDOWN => {
            let px = (lp.0 & 0xffff) as i32;
            let py = ((lp.0 >> 16) & 0xffff) as i32;
            let target = APP.with(|cell| {
                cell.borrow().iter().find(|app| app.hwnd == hwnd).and_then(|app| {
                    let k = app.surface.scale();
                    button_at(app, px as f32 / k)
                })
            });
            if let Some(target) = target {
                with_app_at(hwnd, |app| app.open_menu(px, py, target));
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == CLOCK_TIMER => with_app_at(hwnd, App::render),
        WM_DESTROY => unsafe {
            close_menu();
            _ = KillTimer(Some(hwnd), CLOCK_TIMER);
            // Dropping `App` unregisters the appbar through `AppBar::drop`.
            let last = APP.with(|cell| {
                let mut apps = cell.borrow_mut();
                apps.retain(|app| app.hwnd != hwnd);
                apps.is_empty()
            });
            if last {
                PostQuitMessage(0);
            }
            LRESULT(0)
        },
        _ if msg == window_list::shellhook_message() => with_apps(|apps| {
            let code = wp.0;
            let activated = HWND(lp.0 as *mut core::ffi::c_void);
            // EnumWindows is global: do it once, not once per monitor.
            let snapshot = window_list::enumerate()?;
            for app in apps.iter_mut() {
                app.apply_shell_event(code, activated, snapshot.clone())?;
            }
            Ok(())
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
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: w!("fluent-win-taskbar-menu"),
            lpfnWndProc: Some(menu_wndproc),
            ..Default::default()
        });
        let mut monitors = Vec::new();
        _ = EnumDisplayMonitors(None, None, Some(each_monitor),
            LPARAM(&mut monitors as *mut Vec<HMONITOR> as isize));
        let theme = Theme::system();
        let windows = window_list::enumerate()?;
        for (index, monitor) in monitors.iter().enumerate() {
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            _ = GetMonitorInfoW(*monitor, &mut info);
            let (mut dx, mut dy) = (96, 96);
            _ = GetDpiForMonitor(*monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let k = dx as f32 / 96.0;
            let width = info.rcMonitor.right - info.rcMonitor.left;
            // Height from the work area, not a fixed 48: follows DPI and the
            // user's taskbar settings.
            let bar_h = ((info.rcMonitor.bottom - info.rcWork.bottom) as f32 / k).round() as i32;
            let bar_h = if bar_h > 0 { bar_h } else { metrics::taskbar_height_dip().round() as i32 };
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Taskbar"),
                WS_POPUP | WS_VISIBLE, info.rcWork.left, info.rcWork.bottom - bar_h,
                width, bar_h, None, None, Some(instance.into()), None)?;
            let backdrop = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark).is_ok();
            let surface = Surface::new(hwnd)?;
            let formats = Formats::new(&surface)?;
            let bar = AppBar::new(hwnd, APPBAR_CALLBACK)?;
            // One global shell hook is enough; every bar refreshes on its messages.
            let hook = if index == 0 { Some(ShellHook::new(hwnd)?) } else { None };
            APP.with(|cell| cell.borrow_mut().push(App {
                hwnd, surface, theme, formats, bar, backdrop, _hook: hook,
                windows: windows.clone(), icons: Vec::new(),
                icon_cache: icons::IconCache::new(), active: None,
            }));
        }
        with_apps(|apps| {
            for app in apps.iter_mut() {
                app.refresh_windows();
                app.reposition()?;
                _ = SetTimer(Some(app.hwnd), CLOCK_TIMER, Duration::from_secs(1).as_millis() as u32, None);
            }
            Ok(())
        });
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
