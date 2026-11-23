//! Top-level windows that belong on a taskbar: `EnumWindows` plus the filter
//! used by shell task buttons. `ShellHook` receives the dynamic `SHELLHOOK`
//! message so the list is refreshed only when windows change.

use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, RECT, WPARAM};
use windows::core::BOOL;
use windows::Win32::UI::WindowsAndMessaging::{
    DeregisterShellHookWindow, EnumWindows, GWL_EXSTYLE, GetWindow, GetWindowLongPtrW,
    GW_OWNER, IsWindowVisible, RegisterShellHookWindow, RegisterWindowMessageW,
    SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW, WINDOW_EX_STYLE, WM_GETTEXT,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::{Result, w};

/// One task-button candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    /// Window handle; stable while the window lives.
    pub hwnd: HWND,
    /// `GetWindowTextW` title, possibly empty for windows without a caption.
    pub title: String,
}

/// Win32 filter for a taskbar button: visible, no owner, not a tool window,
/// not no-activate. These windows are what the shell shows as task buttons.
pub fn is_task_window(visible: bool, owner: Option<HWND>, style: WINDOW_EX_STYLE) -> bool {
    visible
        && owner.is_none()
        && !style.contains(WS_EX_TOOLWINDOW)
        && !style.contains(WS_EX_NOACTIVATE)
}

/// Message id of the registered `"SHELLHOOK"` notification.
pub fn shellhook_message() -> u32 {
    unsafe { RegisterWindowMessageW(w!("SHELLHOOK")) }
}

/// Registers `hwnd` for shell notifications. The id comes from
/// [`shellhook_message`]; `Drop` always deregisters.
pub struct ShellHook {
    hwnd: HWND,
}

impl ShellHook {
    /// Registers the window; fails when another shell hook owns the desktop.
    pub fn new(hwnd: HWND) -> Result<Self> {
        if unsafe { RegisterShellHookWindow(hwnd) }.as_bool() {
            Ok(Self { hwnd })
        } else {
            Err(E_FAIL.into())
        }
    }
}

impl Drop for ShellHook {
    fn drop(&mut self) {
        unsafe { _ = DeregisterShellHookWindow(self.hwnd) };
    }
}

extern "system" fn each_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };
    let visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) }.ok();
    let style = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32);
    if is_task_window(visible, owner, style) {
        let mut title = [0u16; 512];
        let mut copied = 0usize;
        // A hung app must not freeze the taskbar: WM_GETTEXT is cross-process,
        // so it gets a short timeout instead of blocking EnumWindows forever.
        let sent = unsafe {
            SendMessageTimeoutW(hwnd, WM_GETTEXT, WPARAM(title.len()),
                LPARAM(title.as_mut_ptr() as isize), SMTO_ABORTIFHUNG | SMTO_BLOCK,
                100, Some(&mut copied))
        };
        let n = if sent.0 == 0 { 0 } else { copied.min(title.len()) };
        list.push(WindowInfo {
            hwnd,
            title: String::from_utf16_lossy(&title[..n]),
        });
    }
    BOOL(1)
}

/// Snapshot of all current task-window candidates.
pub fn enumerate() -> Result<Vec<WindowInfo>> {
    let mut list = Vec::new();
    unsafe { EnumWindows(Some(each_window), LPARAM(&mut list as *mut Vec<WindowInfo> as isize))? };
    Ok(list)
}

/// Side of the work area a bar can attach to (used by the example's layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Bottom of the monitor work area.
    Bottom,
}

/// Rectangle for `n` evenly spaced buttons of `width` DIPs inside `area`,
/// stopping at the right edge so no button lands outside the bar.
pub fn button_rects(area: RECT, n: usize, width: f32, gap: f32) -> Vec<RECT> {
    let pitch = width + gap;
    let room = ((area.right - area.left) as f32 / pitch).floor().max(0.0) as usize;
    (0..n.min(room))
        .map(|i| {
            let x = area.left + (i as f32 * pitch).round() as i32;
            RECT { left: x, top: area.top, right: x + width.round() as i32, bottom: area.bottom }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_should_exclude_tool_windows_and_owned_windows() {
        assert!(is_task_window(true, None, WINDOW_EX_STYLE(0)));
        assert!(!is_task_window(false, None, WINDOW_EX_STYLE(0)));
        let owner = HWND(core::ptr::null_mut());
        assert!(!is_task_window(true, Some(owner), WINDOW_EX_STYLE(0)));
        assert!(!is_task_window(true, None, WS_EX_TOOLWINDOW));
        assert!(!is_task_window(true, None, WS_EX_NOACTIVATE));
    }

    #[test]
    fn buttons_should_be_spaced_and_clamped_to_the_area_height() {
        let area = RECT { left: 8, top: 984, right: 1920, bottom: 1032 };
        let rects = button_rects(area, 3, 160.0, 8.0);
        assert_eq!((rects[0].left, rects[0].right), (8, 168));
        assert_eq!((rects[1].left, rects[2].left), (176, 344));
        assert!(rects.iter().all(|r| r.top == 984 && r.bottom == 1032));
    }

    #[test]
    fn buttons_should_stop_at_the_right_edge() {
        let area = RECT { left: 0, top: 0, right: 100, bottom: 48 };
        let rects = button_rects(area, 10, 48.0, 0.0);
        assert_eq!(rects.len(), 2);
        assert!(rects.iter().all(|r| r.right <= area.right));
    }

    #[test]
    fn shellhook_message_should_be_a_registered_id() {
        assert_ne!(shellhook_message(), 0);
    }
}
