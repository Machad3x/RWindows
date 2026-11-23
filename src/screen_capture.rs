//! Copies the pixels of another window into a 32-bpp DIB section. Used by the
//! taskbar example while Explorer is still the shell: its notification area is
//! drawn by XAML composition and exposes no icon toolbar to read, so mirroring
//! the region is the only way to show the real tray without stealing its
//! Shell_NotifyIcon messages. A real shell uses `crate::tray` instead.

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr;

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, GetDC, HBITMAP, ReleaseDC, SelectObject, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;
use windows::core::Result;

/// Grabs `hwnd`'s rectangle from the screen. The caller owns the returned
/// DIB section and should pass it to [`crate::icons::from_hbitmap`].
pub fn window_bits(hwnd: HWND) -> Result<HBITMAP> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect)?;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = ptr::null_mut();
        let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
        let handle = bitmap;
        let old = SelectObject(memory, handle.into());
        // The Windows 11 tray is drawn by DWM/XAML, so a plain BitBlt of the
        // screen DC misses it and captures the window behind. Ask the window
        // itself to render, then fall back to the screen copy.
        if !windows::Win32::Storage::Xps::PrintWindow(hwnd, memory,
            windows::Win32::Storage::Xps::PRINT_WINDOW_FLAGS(
                windows::Win32::UI::WindowsAndMessaging::PW_RENDERFULLCONTENT)).as_bool()
        {
            _ = BitBlt(memory, 0, 0, width, height, Some(screen), rect.left, rect.top, SRCCOPY);
        }
        _ = SelectObject(memory, old);
        _ = DeleteDC(memory);
        _ = ReleaseDC(None, screen);
        Ok(handle)
    }
}
