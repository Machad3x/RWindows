//! Fase 3: Quick Settings flyout with real volume and brightness backends.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::button::{Input, Key};
use fluent_win::gfx::{Surface, rect};
use fluent_win::panel::Formats;
use fluent_win::quick_settings::{self, Volume};
use fluent_win::radios::{self, Radios, RadioInfo};
use fluent_win::slider::Slider;
use fluent_win::theme::Theme;
use windows::Devices::Radios::RadioKind;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RIGHT, VK_UP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, TranslateMessage, WA_INACTIVE, WM_ACTIVATE,
    WM_CAPTURECHANGED, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const W: f32 = 360.0;
const H: f32 = 388.0;
const PAD: f32 = 16.0;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct Tile {
    label: &'static str,
    rect: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
    on: bool,
}

struct App {
    surface: Surface,
    theme: Theme,
    formats: Formats,
    volume: Option<Volume>,
    brightness: Option<u8>,
    radios: Option<Radios>,
    radio_infos: Vec<RadioInfo>,
    volume_slider: Slider,
    brightness_slider: Slider,
    tiles: [Tile; 3],
    /// Which control holds capture, so WM_MOUSEMOVE/UP reaches the right one.
    captured: u8,
    /// Last slider used; receives arrow/Home/End keys.
    focus: u8,
}

fn with_app(f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win quick settings: {e}");
        }
    });
    LRESULT(0)
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (w, h) = self.surface.size();
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        s.begin();
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, t.card);
        s.text_no_wrap("Configurações rápidas", &f.body,
            rect(PAD, 14.0, w - PAD, 46.0), t.text_primary);
        for tile in &self.tiles {
            let r = tile.rect;
            s.fill_round(r, 8.0, if tile.on { t.accent } else { t.control_alt_fill_secondary });
            s.text_no_wrap(tile.label, &f.body,
                rect(r.left, r.top + 18.0, r.right, r.bottom - 6.0),
                if tile.on { t.text_on_accent } else { t.text_primary });
        }
        let (bl, vl) = (&self.brightness_slider, &self.volume_slider);
        let label = if self.brightness.is_some() { "Brilho" } else { "Brilho (indisponível)" };
        s.text_no_wrap(label, &f.caption, rect(PAD, 128.0, w - PAD, 152.0), t.text_secondary);
        s.text_no_wrap("Volume", &f.caption, rect(PAD, 214.0, w - PAD, 238.0), t.text_secondary);
        bl.draw(s, t)?;
        vl.draw(s, t)?;
        s.end()
    }

    fn point(&self, lp: LPARAM) -> (f32, f32) {
        let k = self.surface.scale();
        ((lp.0 & 0xffff) as f32 / k, ((lp.0 >> 16) & 0xffff) as f32 / k)
    }

    fn toggle_tile(&mut self, index: usize) {
        let Some(radios) = &self.radios else { return };
        match index {
            0 | 1 => {
                let kind = if index == 0 { RadioKind::WiFi } else { RadioKind::Bluetooth };
                let position = self.radio_infos.iter().position(|info| info.kind == kind);
                if let Some(position) = position {
                    let on = self.radio_infos[position].state == windows::Devices::Radios::RadioState::On;
                    let target = if on {
                        windows::Devices::Radios::RadioState::Off
                    } else {
                        windows::Devices::Radios::RadioState::On
                    };
                    if radios.set(position, target).unwrap_or(false) {
                        self.radio_infos[position].state = target;
                    }
                }
            }
            2 => {
                let airplane = !radios::is_airplane(&self.radio_infos);
                if radios.set_airplane(airplane).is_ok()
                    && let Ok(infos) = radios.info()
                {
                    self.radio_infos = infos;
                }
            }
            _ => {}
        }
        self.sync_tiles();
    }

    /// Mirrors radio state into the three tiles.
    fn sync_tiles(&mut self) {
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            tile.on = match index {
                0 => self.radio_infos.iter().any(|info| {
                    info.kind == RadioKind::WiFi
                        && info.state == windows::Devices::Radios::RadioState::On
                }),
                1 => self.radio_infos.iter().any(|info| {
                    info.kind == RadioKind::Bluetooth
                        && info.state == windows::Devices::Radios::RadioState::On
                }),
                2 => radios::is_airplane(&self.radio_infos),
                _ => false,
            };
        }
    }

    /// Pushes changed slider values to the system. A click without a drag must
    /// apply too, so every input path calls this.
    fn apply_values(&mut self, before_v: f64, before_b: f64) {
        if self.volume_slider.value() != before_v {
            let value = self.volume_slider.value() as u32;
            if let Some(volume) = &self.volume {
                _ = volume.set_level(quick_settings::level(value));
            }
        }
        if self.brightness_slider.value() != before_b {
            let value = self.brightness_slider.value() as u8;
            if quick_settings::set_brightness(value) {
                self.brightness = Some(value);
            }
        }
    }

    fn down(&mut self, x: f32, y: f32) -> bool {
        for (index, tile) in self.tiles.iter_mut().enumerate() {
            let r = tile.rect;
            if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                self.toggle_tile(index);
                return true;
            }
        }
        let (before_v, before_b) = (self.volume_slider.value(), self.brightness_slider.value());
        let r = self.volume_slider.rect;
        if y >= r.top && y < r.bottom && x >= r.left && x < r.right {
            self.captured = 1;
            self.focus = 1;
            let out = self.volume_slider.handle(Input::PointerDown(x, y));
            self.apply_values(before_v, before_b);
            return out.capture == Some(true);
        }
        let r = self.brightness_slider.rect;
        if y >= r.top && y < r.bottom && x >= r.left && x < r.right {
            self.captured = 2;
            self.focus = 2;
            let out = self.brightness_slider.handle(Input::PointerDown(x, y));
            self.apply_values(before_v, before_b);
            return out.capture == Some(true);
        }
        false
    }

    /// Returns whether a slider value changed, so the caller can skip a full
    /// Direct2D repaint on mouse moves that touch no control.
    fn move_to(&mut self, x: f32, y: f32) -> bool {
        let (before_v, before_b) = (self.volume_slider.value(), self.brightness_slider.value());
        match self.captured {
            1 => { _ = self.volume_slider.handle(Input::PointerMove(x, y)); }
            2 => { _ = self.brightness_slider.handle(Input::PointerMove(x, y)); }
            _ => return false,
        }
        self.apply_values(before_v, before_b);
        self.volume_slider.value() != before_v || self.brightness_slider.value() != before_b
    }

    fn up(&mut self, x: f32, y: f32) {
        let (before_v, before_b) = (self.volume_slider.value(), self.brightness_slider.value());
        match self.captured {
            1 => { _ = self.volume_slider.handle(Input::PointerUp(x, y)); }
            2 => { _ = self.brightness_slider.handle(Input::PointerUp(x, y)); }
            _ => {}
        }
        self.apply_values(before_v, before_b);
        self.captured = 0;
    }

    fn key(&mut self, vk: VIRTUAL_KEY, down: bool) {
        let key = match vk {
            VK_LEFT | VK_DOWN => Key::Left,
            VK_RIGHT | VK_UP => Key::Right,
            VK_HOME => Key::Home,
            VK_END => Key::End,
            _ => return,
        };
        let (before_v, before_b) = (self.volume_slider.value(), self.brightness_slider.value());
        let input = if down { Input::KeyDown(key) } else { Input::KeyUp(key) };
        match self.focus {
            1 => { _ = self.volume_slider.handle(input); }
            2 => { _ = self.brightness_slider.handle(input); }
            _ => return,
        }
        self.apply_values(before_v, before_b);
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::render)
        }
        WM_LBUTTONDOWN => with_app(|app| {
            let (x, y) = app.point(lp);
            if app.down(x, y) {
                unsafe { _ = SetCapture(hwnd) };
            }
            app.render()
        }),
        WM_MOUSEMOVE => with_app(|app| {
            let (x, y) = app.point(lp);
            if app.move_to(x, y) {
                app.render()?;
            }
            Ok(())
        }),
        WM_LBUTTONUP => with_app(|app| {
            let (x, y) = app.point(lp);
            app.up(x, y);
            unsafe { _ = ReleaseCapture() };
            app.render()
        }),
        WM_CAPTURECHANGED => with_app(|app| {
            app.captured = 0;
            Ok(())
        }),
        WM_KEYDOWN | WM_KEYUP => with_app(|app| {
            app.key(VIRTUAL_KEY(wp.0 as u16), msg == WM_KEYDOWN);
            app.render()
        }),
        WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => unsafe {
            _ = DestroyWindow(hwnd);
            LRESULT(0)
        },
        WM_DESTROY => unsafe {
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn main() -> Result<()> {
    quick_settings::init_com();
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-quick-settings");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class,
            w!("fluent-win · Configurações rápidas"), WS_POPUP | WS_VISIBLE,
            1500, 500, W as i32, H as i32, None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;

        let volume = Volume::new().ok();
        let level = volume.as_ref().and_then(|v| v.level().ok()).unwrap_or(0.5);
        let brightness = quick_settings::brightness();
        let radios = Radios::load().ok();
        let radio_infos = radios.as_ref().and_then(|r| r.info().ok()).unwrap_or_default();
        eprintln!("volume={} brightness={:?} radios={:?}",
            quick_settings::percent(level), brightness,
            radio_infos.iter().map(|i| (&i.name, i.state)).collect::<Vec<_>>());

        let volume_slider = Slider::new("Volume", rect(PAD + 6.0, 238.0, W - PAD - 6.0, 270.0),
            0.0, 100.0, quick_settings::percent(level) as f64);
        let mut brightness_slider = Slider::new("Brilho", rect(PAD + 6.0, 152.0, W - PAD - 6.0, 184.0),
            0.0, 100.0, brightness.unwrap_or(80) as f64);
        if brightness.is_none() {
            brightness_slider.set_enabled(false);
        }
        let tile_w = (W - 2.0 * PAD - 16.0) / 3.0;
        let tiles = ["Wi-Fi", "Bluetooth", "Modo avião"].map(|label| Tile {
            label,
            rect: rect(PAD, 56.0, PAD + tile_w, 120.0),
            on: false,
        });
        let mut tiles = tiles;
        for (i, tile) in tiles.iter_mut().enumerate() {
            tile.rect.left = PAD + i as f32 * (tile_w + 8.0);
            tile.rect.right = tile.rect.left + tile_w;
        }
        let mut app = App {
            surface, theme, formats, volume, brightness, radios, radio_infos,
            volume_slider, brightness_slider, tiles, captured: 0, focus: 0,
        };
        app.sync_tiles();
        APP.with(|cell| *cell.borrow_mut() = Some(app));
        with_app(App::render);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
