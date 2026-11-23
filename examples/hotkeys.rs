//! Fase 3: global Win+E / Win+D shortcuts through the low-level keyboard hook.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::hotkeys::{Action, KeyboardHook, WM_SHORTCUT};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, GetMessageW, GetWindow,
    GetWindowTextW, GW_OWNER, IsIconic, IsWindow, IsWindowVisible, MSG, PostQuitMessage,
    RegisterClassW, SW_MINIMIZE, SW_RESTORE, ShowWindow, WNDCLASSW, WS_EX_TOOLWINDOW,
};
use windows::core::{Result, w};

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Windows minimized by the first Win+D, restored by the second.
    static MINIMIZED: RefCell<Vec<HWND>> = const { RefCell::new(Vec::new()) };
}

struct App {
    /// Kept alive: dropping unregisters the hook.
    _hook: KeyboardHook,
}

extern "system" fn each_window(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
    unsafe {
        // Our own window is hidden, so IsWindowVisible already excludes it.
        // Already-minimized windows must stay minimized after the next Win+D.
        if !IsWindowVisible(hwnd).as_bool()
            || IsIconic(hwnd).as_bool()
            || GetWindow(hwnd, GW_OWNER).is_ok()
        {
            return windows::core::BOOL(1);
        }
        let mut title = [0u16; 256];
        if GetWindowTextW(hwnd, &mut title) == 0 {
            return windows::core::BOOL(1);
        }
        let ex = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            hwnd, windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE,
        );
        // shortcut: ignores WS_EX_NOACTIVATE and cloaked windows; add those
        // filters if a shell must match Win+D exactly on virtual desktops.
        if ex & (WS_EX_TOOLWINDOW.0 as isize) != 0 {
            return windows::core::BOOL(1);
        }
        let windows = &mut *(data.0 as *mut Vec<HWND>);
        windows.push(hwnd);
    }
    windows::core::BOOL(1)
}

fn show_desktop() {
    let restore = MINIMIZED.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
    if !restore.is_empty() {
        for hwnd in restore {
            if unsafe { IsWindow(Some(hwnd)).as_bool() && IsIconic(hwnd).as_bool() } {
                unsafe { _ = ShowWindow(hwnd, SW_RESTORE) };
            }
        }
        return;
    }
    let mut found: Vec<HWND> = Vec::new();
    unsafe { _ = EnumWindows(Some(each_window), LPARAM(&mut found as *mut Vec<HWND> as isize)) };
    for hwnd in &found {
        unsafe { _ = ShowWindow(*hwnd, SW_MINIMIZE) };
    }
    MINIMIZED.with(|cell| *cell.borrow_mut() = found);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_SHORTCUT {
        match Action::from_usize(wp.0) {
            Some(Action::Explorer) => unsafe {
                _ = ShellExecuteW(None, w!("open"), w!("explorer.exe"), None, None,
                    windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL);
            },
            Some(Action::ShowDesktop) => show_desktop(),
            None => {}
        }
        return LRESULT(0);
    }
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => unsafe {
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn main() -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-hotkeys");
        RegisterClassW(&WNDCLASSW {
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, class, w!("fluent-win · Atalhos"),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(0),
            0, 0, 0, 0, None, None, Some(instance.into()), None)?;
        let hook = KeyboardHook::new(hwnd)?;
        eprintln!("hotkeys installed on hwnd {:?}", hwnd.0);
        APP.with(|cell| *cell.borrow_mut() = Some(App { _hook: hook }));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = DispatchMessageW(&msg);
        }
        Ok(())
    }
}
