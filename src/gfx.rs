//! Direct2D + DirectWrite on a DirectComposition swap chain.
//!
//! Composition (not HWND) swap chain with premultiplied alpha: transparent
//! pixels reveal the DWM backdrop (Mica/Acrylic). Text is grayscale-AA, like
//! WinUI; ClearType cannot blend onto a transparent surface.

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, E_POINTER, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_GRADIENT_STOP, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET, D2D1_BITMAP_PROPERTIES1,
    D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_EXTEND_MODE_CLAMP,
    D2D1_GAMMA_2_2, D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
    D2D1CreateFactory, ID2D1Brush, ID2D1DeviceContext, ID2D1RenderTarget, ID2D1Factory1, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS,
    D3D11_CREATE_DEVICE_SINGLETHREADED, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
};
use windows::Win32::Graphics::DirectComposition::{
    DCompositionCreateDevice, IDCompositionDevice, IDCompositionTarget, IDCompositionVisual,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_AUTOMATIC_FONT_AXES_OPTICAL_SIZE, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_AXIS_TAG_WEIGHT,
    DWRITE_FONT_AXIS_VALUE,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_TEXT_METRICS, DWriteCreateFactory, IDWriteFactory6, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN,
    DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS, DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET,
    DXGI_PRESENT, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_CHAIN_FLAG, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
    IDXGIDevice, IDXGIFactory2, IDXGISurface, IDXGISwapChain1,
};
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{Error, Interface, Result, w};
use windows_numerics::Vector2;

use crate::theme::{self, Ramp};

const TRANSPARENT: D2D1_COLOR_F = D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };

/// A window's drawing surface. Coordinates are DIPs (1/96 inch).
pub struct Surface {
    ctx: ID2D1DeviceContext,
    dwrite: IDWriteFactory6,
    brush: ID2D1SolidColorBrush,
    swap: IDXGISwapChain1,
    _dcomp: IDCompositionDevice,
    _target: Option<IDCompositionTarget>,
    _visual: IDCompositionVisual,
    hwnd: HWND,
    dpi: f32,
    px: (u32, u32),
}

impl Surface {
    /// Binds a surface to `hwnd`. Create the window with `WS_EX_NOREDIRECTIONBITMAP`
    /// so GDI does not paint an opaque layer under the composition visual.
    pub fn new(hwnd: HWND) -> Result<Self> {
        unsafe {
            // WARP (software) when there is no usable GPU: headless VMs, broken drivers.
            let d3d = d3d_device(D3D_DRIVER_TYPE_HARDWARE).or_else(|_| d3d_device(D3D_DRIVER_TYPE_WARP))?;
            let dxgi: IDXGIDevice = d3d.cast()?;

            let d2d: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let ctx = d2d.CreateDevice(&dxgi)?.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            ctx.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);

            let px = client_px(hwnd)?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: px.0,
                Height: px.1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
                AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
                ..Default::default()
            };
            let factory: IDXGIFactory2 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0))?;
            let swap = factory.CreateSwapChainForComposition(&d3d, &desc, None)?;

            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let target = dcomp.CreateTargetForHwnd(hwnd, true)?;
            let visual = dcomp.CreateVisual()?;
            visual.SetContent(&swap)?;
            target.SetRoot(&visual)?;
            dcomp.Commit()?;

            let surface = Self {
                brush: ctx.CreateSolidColorBrush(&TRANSPARENT, None)?,
                dwrite: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
                ctx,
                swap,
                _dcomp: dcomp,
                _target: Some(target),
                _visual: visual,
                hwnd,
                dpi: GetDpiForWindow(hwnd) as f32,
                px,
            };
            surface.bind()?;
            Ok(surface)
        }
    }

    fn bind(&self) -> Result<()> {
        unsafe {
            let buffer: IDXGISurface = self.swap.GetBuffer(0)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: self.dpi,
                dpiY: self.dpi,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                ..Default::default()
            };
            let bitmap = self.ctx.CreateBitmapFromDxgiSurface(&buffer, Some(&props))?;
            self.ctx.SetTarget(&bitmap);
            self.ctx.SetDpi(self.dpi, self.dpi);
            Ok(())
        }
    }

    /// Call on `WM_SIZE` with the new client size in pixels.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let px = (width.max(1), height.max(1));
        if px == self.px {
            return Ok(());
        }
        unsafe {
            self.ctx.SetTarget(None);
            self.swap.ResizeBuffers(0, px.0, px.1, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        self.bind()?;
        // Only after success: a failed resize must be retried by the next WM_SIZE.
        self.px = px;
        Ok(())
    }

    /// Call on `WM_DPICHANGED` with the new DPI (low word of `wParam`).
    pub fn set_dpi(&mut self, dpi: f32) -> Result<()> {
        self.dpi = dpi;
        unsafe { self.ctx.SetTarget(None) };
        self.bind()
    }

    /// Pixels per DIP.
    pub fn scale(&self) -> f32 {
        self.dpi / 96.0
    }

    /// Client size in DIPs.
    pub fn size(&self) -> (f32, f32) {
        (self.px.0 as f32 / self.scale(), self.px.1 as f32 / self.scale())
    }

    /// Raw device context, for drawing the helpers do not cover.
    pub fn ctx(&self) -> &ID2D1DeviceContext {
        &self.ctx
    }

    /// Text format for a type-ramp step. Matches XAML: typographic family
    /// `Segoe UI Variable`, optical size chosen automatically from the size.
    pub fn format(&self, ramp: &Ramp) -> Result<IDWriteTextFormat> {
        text_format(&self.dwrite, ramp)
    }

    /// Starts a frame and clears to transparent (backdrop shows through).
    pub fn begin(&self) {
        unsafe {
            self.ctx.BeginDraw();
            self.ctx.Clear(Some(&TRANSPARENT));
        }
    }

    /// Ends the frame and presents it, waiting for vsync.
    ///
    /// If the GPU device was lost (driver update, adapter reset, RDP), every
    /// device resource is recreated and a repaint is requested; the lost frame
    /// is dropped. Text formats stay valid: DirectWrite is device-independent.
    pub fn end(&mut self) -> Result<()> {
        let presented = unsafe {
            self.ctx.EndDraw(None, None).and_then(|()| self.swap.Present(1, DXGI_PRESENT(0)).ok())
        };
        match presented {
            Err(e) if is_device_lost(&e) => {
                let recreated = self.recreate();
                // Repaint either way: on success to draw the dropped frame, on
                // failure so the next WM_PAINT retries the recreation.
                unsafe { _ = InvalidateRect(Some(self.hwnd), None, false) };
                recreated
            }
            other => other,
        }
    }

    /// Rounds edges to whole device pixels, like WinUI layout rounding, so
    /// fills stay crisp at 125%/150% scaling.
    fn snap(&self, r: D2D_RECT_F) -> D2D_RECT_F {
        let k = self.scale();
        let s = |v: f32| (v * k).round() / k;
        rect(s(r.left), s(r.top), s(r.right), s(r.bottom))
    }

    /// Rebuilds every device resource on the same window.
    fn recreate(&mut self) -> Result<()> {
        // DirectComposition allows one target per HWND: release the old one first.
        self._target = None;
        // Best effort: on a lost device Commit may fail, the release still happens.
        unsafe { _ = self._dcomp.Commit() };
        *self = Self::new(self.hwnd)?;
        Ok(())
    }

    /// Fills a rounded rectangle (edges snapped to device pixels).
    pub fn fill_round(&self, rect: D2D_RECT_F, radius: f32, color: D2D1_COLOR_F) {
        unsafe {
            self.brush.SetColor(&color);
            self.ctx.FillRoundedRectangle(&rounded(self.snap(rect), radius), &self.brush);
        }
    }

    /// Draws a `width`-DIP border inside `rect`, like a XAML `Border`
    /// (`radius` is the outer corner radius; edges snapped to device pixels).
    pub fn stroke_round(&self, rect: D2D_RECT_F, radius: f32, width: f32, color: D2D1_COLOR_F) {
        unsafe { self.brush.SetColor(&color) };
        self.stroke_with(rect, radius, width, &self.brush);
    }

    fn stroke_with(&self, rect: D2D_RECT_F, radius: f32, width: f32, brush: &ID2D1Brush) {
        let r = self.snap(rect);
        let h = width / 2.0;
        let center = D2D_RECT_F { left: r.left + h, top: r.top + h, right: r.right - h, bottom: r.bottom - h };
        unsafe { self.ctx.DrawRoundedRectangle(&rounded(center, (radius - h).max(0.0)), brush, width, None) };
    }

    /// Size of `text` laid out on one line, in DIPs (width, line height).
    pub fn measure(&self, text: &str, format: &IDWriteTextFormat) -> Result<(f32, f32)> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe {
            let layout = self.dwrite.CreateTextLayout(&wide, format, f32::MAX, f32::MAX)?;
            layout.GetMetrics(&mut m)?;
        }
        Ok((m.widthIncludingTrailingWhitespace, m.height))
    }

    /// 1-DIP border with WinUI's `ControlElevationBorderBrush` shape: a linear
    /// gradient over the 3 DIPs next to `edge` (stops 0.33 = `edge_color`,
    /// 1.0 = `rest_color`), solid `rest_color` elsewhere.
    // shortcut: gradient brush built per call (two small device objects); cache per theme if profiling shows it.
    pub fn stroke_elevation(&self, rect: D2D_RECT_F, radius: f32, edge: Edge,
        edge_color: D2D1_COLOR_F, rest_color: D2D1_COLOR_F) -> Result<()> {
        let r = self.snap(rect);
        // XAML: MappingMode=Absolute, StartPoint 0,0, EndPoint 0,3 (flipped for Edge::Bottom).
        let (start, end) = match edge {
            Edge::Top => (r.top, r.top + 3.0),
            Edge::Bottom => (r.bottom, r.bottom - 3.0),
        };
        let stops = [
            D2D1_GRADIENT_STOP { position: 0.33, color: edge_color },
            D2D1_GRADIENT_STOP { position: 1.0, color: rest_color },
        ];
        unsafe {
            // ID2D1RenderTarget overload (gamma + extend); DeviceContext shadows it with a 6-arg one.
            let stops = ID2D1RenderTarget::CreateGradientStopCollection(&self.ctx, &stops, D2D1_GAMMA_2_2, D2D1_EXTEND_MODE_CLAMP)?;
            let props = D2D1_LINEAR_GRADIENT_BRUSH_PROPERTIES {
                startPoint: Vector2::new(r.left, start),
                endPoint: Vector2::new(r.left, end),
            };
            let brush = self.ctx.CreateLinearGradientBrush(&props, None, &stops)?;
            self.stroke_with(r, radius, 1.0, &brush.cast()?);
        }
        Ok(())
    }

    /// Draws `text` laid out inside `rect`.
    // shortcut: one UTF-16 allocation per call; cache IDWriteTextLayout per label if profiling shows it.
    pub fn text(&self, text: &str, format: &IDWriteTextFormat, rect: D2D_RECT_F, color: D2D1_COLOR_F) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe {
            self.brush.SetColor(&color);
            self.ctx.DrawText(&wide, format, &rect, &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, DWRITE_MEASURING_MODE_NATURAL);
        }
    }
}

/// Which edge of a control carries the lit/shaded line of an elevation border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Top edge (dark-theme `ControlElevationBorderBrush`).
    Top,
    /// Bottom edge (light theme, and every `AccentControlElevationBorderBrush`:
    /// the XAML brush is flipped with `ScaleY=-1`).
    Bottom,
}

/// `D2D_RECT_F` from edges.
pub const fn rect(left: f32, top: f32, right: f32, bottom: f32) -> D2D_RECT_F {
    D2D_RECT_F { left, top, right, bottom }
}

fn rounded(rect: D2D_RECT_F, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect, radiusX: radius, radiusY: radius }
}

/// True for errors that mean the GPU device is gone and every device resource
/// must be recreated (driver update/crash, adapter removed, RDP switch).
fn is_device_lost(e: &Error) -> bool {
    [DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, D2DERR_RECREATE_TARGET].contains(&e.code())
}

fn text_format(dwrite: &IDWriteFactory6, ramp: &Ramp) -> Result<IDWriteTextFormat> {
    let axes = [DWRITE_FONT_AXIS_VALUE { axisTag: DWRITE_FONT_AXIS_TAG_WEIGHT, value: ramp.weight }];
    unsafe {
        // Null collection = system collection grouped by typographic family (IDWriteFactory6).
        let format = dwrite.CreateTextFormat(theme::FAMILY, None, &axes, ramp.size, w!(""))?;
        format.SetAutomaticFontAxes(DWRITE_AUTOMATIC_FONT_AXES_OPTICAL_SIZE)?;
        format.cast()
    }
}

fn d3d_device(driver: D3D_DRIVER_TYPE) -> Result<ID3D11Device> {
    let mut d3d = None;
    unsafe {
        D3D11CreateDevice(None, driver, HMODULE::default(),
            // Single-threaded device + no driver worker threads: fewer threads and less private memory.
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_SINGLETHREADED
                | D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS, None, D3D11_SDK_VERSION,
            Some(&mut d3d), None, None)?;
    }
    d3d.ok_or_else(|| Error::from(E_POINTER))
}

fn client_px(hwnd: HWND) -> Result<(u32, u32)> {
    let mut r = RECT::default();
    unsafe { GetClientRect(hwnd, &mut r)? };
    Ok(((r.right - r.left).max(1) as u32, (r.bottom - r.top).max(1) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Theme;
    use windows::Win32::Graphics::DirectWrite::{DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC, DWRITE_LINE_METRICS};

    /// Natural line height and baseline of one ramp step, as DirectWrite lays it out.
    fn line_metrics(ramp: &Ramp) -> (f32, f32) {
        unsafe {
            let dwrite: IDWriteFactory6 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let format = text_format(&dwrite, ramp).unwrap();
            let text: Vec<u16> = "Ag".encode_utf16().collect();
            let layout = dwrite.CreateTextLayout(&text, &format, 1000.0, 1000.0).unwrap();
            let mut m = [DWRITE_LINE_METRICS::default()];
            let mut n = 0;
            layout.GetLineMetrics(Some(&mut m), &mut n).unwrap();
            (m[0].height, m[0].baseline)
        }
    }

    /// Invisible top-level window, enough to host a composition target.
    fn hidden_window() -> HWND {
        use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP};
        unsafe {
            CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, w!("STATIC"), w!(""), WS_POPUP,
                0, 0, 64, 64, None, None, None, None).unwrap()
        }
    }

    #[test]
    fn measure_should_grow_with_text_and_use_the_natural_line_height() {
        let surface = Surface::new(hidden_window()).unwrap();
        let body = surface.format(&theme::BODY).unwrap();
        let (short, h) = surface.measure("Ok", &body).unwrap();
        let (long, _) = surface.measure("Configurações", &body).unwrap();
        assert!(short > 0.0 && long > short && (h - 18.62).abs() < 0.05, "{short} {long} {h}");
    }

    #[test]
    fn strokes_should_draw_on_a_real_surface() {
        let mut surface = Surface::new(hidden_window()).unwrap();
        let r = rect(4.0, 4.0, 60.0, 36.0);
        surface.begin();
        surface.stroke_round(r, theme::CONTROL_RADIUS, 2.0, Theme::DARK.focus_outer);
        let drawn = surface.stroke_elevation(r, theme::CONTROL_RADIUS, Edge::Bottom,
            Theme::DARK.stroke_secondary, Theme::DARK.stroke);
        assert_eq!((drawn, surface.end()), (Ok(()), Ok(())));
    }

    #[test]
    fn recreate_should_rebind_the_same_window() {
        let mut surface = Surface::new(hidden_window()).unwrap();
        assert_eq!(surface.recreate(), Ok(()));
    }

    #[test]
    fn device_lost_should_be_detected_for_removed_reset_and_recreate_target() {
        let lost = [DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, D2DERR_RECREATE_TARGET];
        assert!(lost.into_iter().all(|hr| is_device_lost(&Error::from(hr))));
    }

    #[test]
    fn other_errors_should_not_count_as_device_lost() {
        assert!(!is_device_lost(&Error::from(windows::Win32::Foundation::E_INVALIDARG)));
    }

    #[test]
    fn system_collection_should_have_typographic_segoe_ui_variable() {
        let exists = unsafe {
            let dwrite: IDWriteFactory6 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let fonts = dwrite.GetSystemFontCollection(false, DWRITE_FONT_FAMILY_MODEL_TYPOGRAPHIC).unwrap();
            let (mut index, mut exists) = (0, windows::core::BOOL(0));
            fonts.FindFamilyName(theme::FAMILY, &mut index, &mut exists).unwrap();
            exists
        };
        assert!(exists.as_bool());
    }

    #[test]
    fn every_ramp_step_should_have_a_natural_line_taller_than_its_size() {
        // Prints the natural metrics documented in the README (cargo test -- --nocapture).
        for ramp in theme::RAMP {
            let (height, baseline) = line_metrics(&ramp);
            println!("size {:>4} weight {} -> line {height:.2} baseline {baseline:.2}", ramp.size, ramp.weight);
            assert!(height > ramp.size && baseline < height);
        }
    }
}
