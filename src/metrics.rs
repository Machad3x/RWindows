//! Shell metrics derived from the system instead of measured constants.
//! Everything returned here is in DIPs unless the name says `pixels`.

use std::mem::size_of;

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    HMONITOR, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, GetMonitorInfoW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

/// Scale of the primary monitor: 1.0 at 96 DPI.
pub fn scale() -> f32 {
    let monitor = primary_monitor();
    let (mut x, mut y) = (96, 96);
    unsafe { _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    (x.max(y) as f32 / 96.0).max(0.25)
}

/// Handle of the primary monitor.
pub fn primary_monitor() -> HMONITOR {
    unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) }
}

/// Monitor rectangle in screen pixels.
pub fn monitor_rect() -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { _ = GetMonitorInfoW(primary_monitor(), &mut info) };
    info.rcMonitor
}

/// Work area in screen pixels (excludes the taskbar and other appbars).
pub fn work_rect() -> RECT {
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe { _ = GetMonitorInfoW(primary_monitor(), &mut info) };
    info.rcWork
}

/// Height of the taskbar strip on the primary monitor, in DIPs.
/// Comes from the difference between the monitor and its work area, so it
/// follows DPI, scaling and the user's taskbar settings.
pub fn taskbar_height_dip() -> f32 {
    let monitor = monitor_rect();
    let work = work_rect();
    let pixels = [
        monitor.bottom - work.bottom,
        work.top - monitor.top,
        monitor.right - work.right,
        work.left - monitor.left,
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    (pixels as f32 / scale()).max(0.0)
}

/// Top-left screen position for a bottom-right flyout of `width` x `height` DIPs.
pub fn bottom_right(width: f32, height: f32, margin: f32) -> (i32, i32) {
    let work = work_rect();
    let k = scale();
    (
        work.right - (width * k).round() as i32 - (margin * k).round() as i32,
        work.bottom - (height * k).round() as i32 - (margin * k).round() as i32,
    )
}

/// Top-left screen position for a Start-like panel: horizontally centred in
/// the work area and resting on its bottom edge.
pub fn centered_bottom(width: f32, height: f32) -> (i32, i32) {
    let work = work_rect();
    let k = scale();
    let w = (width * k).round() as i32;
    let h = (height * k).round() as i32;
    (
        work.left + ((work.right - work.left - w) / 2).max(0),
        work.bottom - h,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_should_never_be_zero() {
        assert!(scale() > 0.0);
    }

    #[test]
    fn work_area_should_fit_inside_the_monitor() {
        let monitor = monitor_rect();
        let work = work_rect();
        assert!(work.left >= monitor.left && work.right <= monitor.right);
        assert!(work.top >= monitor.top && work.bottom <= monitor.bottom);
    }

    #[test]
    fn taskbar_height_should_be_reasonable() {
        let height = taskbar_height_dip();
        assert!((0.0..200.0).contains(&height));
    }
}
