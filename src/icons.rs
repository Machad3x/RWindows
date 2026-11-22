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
use windows::Win32::Graphics::Gdi::{BITMAP, DeleteObject, GetObjectW};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SIIGBF_ICONONLY, SHCreateItemFromParsingName};
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
    let mut info = BITMAP::default();
    let read = unsafe {
        GetObjectW(handle.into(), size_of::<BITMAP>() as i32, Some(ptr::from_mut(&mut info).cast()))
    };
    if read == 0 || info.bmBits.is_null() || info.bmBitsPixel != 32 {
        unsafe { _ = DeleteObject(handle.into()) };
        return Err(windows::Win32::Foundation::E_INVALIDARG.into());
    }
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
        colorContext: Default::default(),
    };
    let dims = D2D_SIZE_U {
        width: info.bmWidth.unsigned_abs(),
        height: info.bmHeight.unsigned_abs(),
    };
    let result = unsafe {
        surface.ctx().CreateBitmap(dims, Some(info.bmBits.cast()), info.bmWidthBytes as u32, &props)
    };
    unsafe { _ = DeleteObject(handle.into()) };
    result
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
    fn a_new_cache_should_be_empty() {
        let cache = IconCache::new();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }
}
