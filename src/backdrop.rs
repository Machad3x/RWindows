//! DWM window attributes: system backdrop, corners, dark mode.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{
    DWM_SYSTEMBACKDROP_TYPE, DWM_WINDOW_CORNER_PREFERENCE, DWMWA_SYSTEMBACKDROP_TYPE,
    DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWINDOWATTRIBUTE,
    DwmExtendFrameIntoClientArea, DwmSetWindowAttribute,
};
use windows::Win32::UI::Controls::MARGINS;
use windows::core::Result;

/// Applies a system backdrop (e.g. `DWMSBT_MAINWINDOW` = Mica) behind the whole
/// client area, plus corner style and dark mode. Needs Windows 11 22H2+.
///
/// The client area must be transparent for the backdrop to show: draw with
/// [`crate::gfx::Surface`] on a window created with `WS_EX_NOREDIRECTIONBITMAP`.
pub fn apply(
    hwnd: HWND,
    backdrop: DWM_SYSTEMBACKDROP_TYPE,
    corners: DWM_WINDOW_CORNER_PREFERENCE,
    dark: bool,
) -> Result<()> {
    set_dark(hwnd, dark)?;
    unsafe {
        set(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &corners)?;
        // -1 margins: "sheet of glass", the backdrop covers the whole client area.
        let all = MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 };
        DwmExtendFrameIntoClientArea(hwnd, &all)?;
        set(hwnd, DWMWA_SYSTEMBACKDROP_TYPE, &backdrop)
    }
}

/// Switches the title bar and backdrop tint between dark and light.
pub fn set_dark(hwnd: HWND, dark: bool) -> Result<()> {
    unsafe { set(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &i32::from(dark)) }
}

unsafe fn set<T>(hwnd: HWND, attr: DWMWINDOWATTRIBUTE, value: &T) -> Result<()> {
    unsafe {
        DwmSetWindowAttribute(hwnd, attr, (value as *const T).cast(), size_of::<T>() as u32)
    }
}
