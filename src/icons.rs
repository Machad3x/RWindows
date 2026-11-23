//! Real Windows icons: `IShellItemImageFactory` → D2D bitmap, cached by path
//! and pixel size. The shell needs file/app icons for the taskbar and Start.

use std::collections::HashMap;
use std::ptr;

use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT, D2D_SIZE_U,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR, ID2D1Bitmap1,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, DeleteObject, GetDIBits,
    GetObjectW, HBITMAP,
};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SIIGBF_ICONONLY, SHCreateItemFromParsingName};
use windows::Win32::UI::WindowsAndMessaging::{GetIconInfo, HICON, ICONINFO};
use windows::core::{HSTRING, Result};

use crate::gfx::{Surface, rect};

/// D2D bitmaps keyed by (path, pixel size).
#[derive(Default)]
pub struct IconCache {
    entries: HashMap<(String, u32), ID2D1Bitmap1>,
    generation: u64,
}

impl IconCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cached icons (useful for tests and memory checks).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing has been loaded.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the icon for `path`, loading it on the first call.
    pub fn get(&mut self, surface: &Surface, path: &str, size: u32) -> Result<&ID2D1Bitmap1> {
        let generation = surface.generation();
        if generation != self.generation {
            self.entries.clear();
            self.generation = generation;
        }
        let key = (path.to_owned(), size);
        if !self.entries.contains_key(&key) {
            let bitmap = load(surface, path, size)?;
            self.entries.insert(key.clone(), bitmap);
        }
        Ok(&self.entries[&key])
    }

}

fn load(surface: &Surface, path: &str, size: u32) -> Result<ID2D1Bitmap1> {
    let item: IShellItemImageFactory = unsafe {
        SHCreateItemFromParsingName(&HSTRING::from(path), None)?
    };
    let handle = unsafe { item.GetImage(SIZE { cx: size as i32, cy: size as i32 }, SIIGBF_ICONONLY)? };
    from_hbitmap(surface, handle)
}

/// Wraps a 32-bpp DIB section into a D2D bitmap, then deletes the GDI handle.
/// Used by both file icons and window icons from `WM_GETICON`.
pub fn from_hbitmap(surface: &Surface, handle: HBITMAP) -> Result<ID2D1Bitmap1> {
    let mut info = BITMAP::default();
    let read = unsafe {
        GetObjectW(handle.into(), size_of::<BITMAP>() as i32, Some(ptr::from_mut(&mut info).cast()))
    };
    if read == 0 || info.bmBitsPixel != 32 {
        unsafe { _ = DeleteObject(handle.into()) };
        return Err(windows::Win32::Foundation::E_INVALIDARG.into());
    }
    let width = info.bmWidth.unsigned_abs();
    let height = info.bmHeight.unsigned_abs();
    // GetIconInfo returns plain compatible bitmaps, which have no bmBits.
    if info.bmBits.is_null() {
        return from_gdi_bits(surface, handle, width, height);
    }
    let props = bitmap_props();
    let stride = info.bmWidthBytes.unsigned_abs() as usize;
    let source = unsafe {
        std::slice::from_raw_parts(info.bmBits.cast::<u8>(), stride * height as usize)
    };
    let pixels = copy_rows(source, stride, height, info.bmHeight > 0);
    unsafe { _ = DeleteObject(handle.into()) };
    if pixels.is_empty() {
        return Err(windows::Win32::Foundation::E_INVALIDARG.into());
    }
    unsafe {
        surface.ctx().CreateBitmap(D2D_SIZE_U { width, height }, Some(pixels.as_ptr().cast()),
            info.bmWidthBytes.unsigned_abs(), &props)
    }
}

/// Wraps the colour bitmap of a real tray HICON.
pub fn from_hicon(surface: &Surface, icon: HICON) -> Result<ID2D1Bitmap1> {
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut info)?; }
    let handle = info.hbmColor;
    if handle.0.is_null() {
        unsafe { if !info.hbmMask.0.is_null() { _ = DeleteObject(info.hbmMask.into()); } }
        return Err(windows::Win32::Foundation::E_INVALIDARG.into());
    }
    let result = from_hbitmap(surface, handle);
    unsafe { if !info.hbmMask.0.is_null() { _ = DeleteObject(info.hbmMask.into()); } }
    result
}

fn bitmap_props() -> D2D1_BITMAP_PROPERTIES1 {
    D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        colorContext: Default::default(),
    }
}

/// Reads a compatible HBITMAP (no bmBits) through `GetDIBits` into a DIB.
fn from_gdi_bits(surface: &Surface, handle: HBITMAP, width: u32, height: u32) -> Result<ID2D1Bitmap1> {
    let stride = width as usize * 4;
    let mut pixels = vec![0u8; stride * height as usize];
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width as i32,
            biHeight: height as i32,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    unsafe {
        let dc = windows::Win32::Graphics::Gdi::GetDC(None);
        _ = GetDIBits(dc, handle, 0, height, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        _ = windows::Win32::Graphics::Gdi::ReleaseDC(None, dc);
        _ = DeleteObject(handle.into());
    }
    let rows = copy_rows(&pixels, stride, height, true);
    unsafe {
        surface.ctx().CreateBitmap(D2D_SIZE_U { width, height }, Some(rows.as_ptr().cast()),
            stride as u32, &bitmap_props())
    }
}

/// `IShellItemImageFactory` returns a bottom-up DIB: its first row is the
/// bottom of the picture. Direct2D reads rows top-down, so copy in reverse.
fn copy_rows(source: &[u8], stride: usize, height: u32, bottom_up: bool) -> Vec<u8> {
    let mut target = vec![0u8; stride * height as usize];
    for row in 0..height as usize {
        let source_row = if bottom_up { height as usize - 1 - row } else { row };
        target[row * stride..(row + 1) * stride]
            .copy_from_slice(&source[source_row * stride..(source_row + 1) * stride]);
    }
    target
}

/// Draws a cached icon at `left, top` with its natural pixel size in DIPs.
pub fn draw(surface: &Surface, bitmap: &ID2D1Bitmap1, left: f32, top: f32, size: f32) {
    let r = rect(left, top, left + size, top + size);
    unsafe {
        surface.ctx().DrawBitmap(bitmap, Some(&r), 1.0,
            D2D1_INTERPOLATION_MODE_LINEAR, None, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_up_dib_rows_should_be_flipped_for_direct2d() {
        let dib = [1u8, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(copy_rows(&dib, 4, 2, true), vec![5, 6, 7, 8, 1, 2, 3, 4]);
        assert_eq!(copy_rows(&dib, 4, 2, false), dib);
    }

    #[test]
    fn a_new_cache_should_be_empty() {
        let cache = IconCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }
}
