//! Win32 **appbar**: a window that reserves screen space like the taskbar.
//! `SHAppBarMessage(ABM_NEW/QUERYPOS/SETPOS/REMOVE)` follows the documented
//! sequence; the app owns the window and handles `ABN_POSCHANGED`.

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, APPBARDATA, SHAppBarMessage,
};
use windows::core::Result;

/// Height of the Windows 11 taskbar in DIPs.
pub const HEIGHT: f32 = 48.0;

/// Rectangle of a bottom appbar inside a monitor's work area, in pixels.
/// `MONITORINFO.rcWork` already excludes the taskbar and other appbars.
pub fn bottom_rect(work: RECT, height_px: i32) -> RECT {
    RECT {
        left: work.left,
        top: work.bottom - height_px,
        right: work.right,
        bottom: work.bottom,
    }
}

/// A registered appbar. Dropping it calls `ABM_REMOVE`, so a crashed or closed
/// shell window cannot leave a phantom reservation behind.
pub struct AppBar {
    hwnd: HWND,
}

impl AppBar {
    /// Registers `hwnd` as a bottom appbar. `callback` is the app-defined
    /// message used by the system for `ABN_*` notifications.
    pub fn new(hwnd: HWND, callback: u32) -> Result<Self> {
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: hwnd,
            uCallbackMessage: callback,
            uEdge: ABE_BOTTOM,
            ..Default::default()
        };
        unsafe { _ = SHAppBarMessage(ABM_NEW, &mut data) };
        Ok(Self { hwnd })
    }

    /// Asks the system for a bottom position and commits it. Returns the final
    /// rectangle in screen pixels.
    pub fn set_bottom(&mut self, desired: RECT) -> RECT {
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            uEdge: ABE_BOTTOM,
            rc: desired,
            ..Default::default()
        };
        unsafe {
            _ = SHAppBarMessage(ABM_QUERYPOS, &mut data);
            _ = SHAppBarMessage(ABM_SETPOS, &mut data);
        }
        data.rc
    }
}

impl Drop for AppBar {
    fn drop(&mut self) {
        let mut data = APPBARDATA {
            cbSize: size_of::<APPBARDATA>() as u32,
            hWnd: self.hwnd,
            ..Default::default()
        };
        unsafe { _ = SHAppBarMessage(ABM_REMOVE, &mut data) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_rect_should_hug_the_bottom_of_the_work_area() {
        let work = RECT { left: -1080, top: -498, right: 840, bottom: 1032 };
        let r = bottom_rect(work, 64);
        assert_eq!((r.left, r.top, r.right, r.bottom), (-1080, 968, 840, 1032));
    }

    #[test]
    fn a_bar_as_tall_as_the_work_area_should_start_at_the_top() {
        let work = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
        let r = bottom_rect(work, 1080);
        assert_eq!(r.top, 0);
    }
}
