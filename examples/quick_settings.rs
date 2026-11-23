//! Fase 3: Quick Settings flyout measured against the real Windows 11 panel.
//! On this desktop there is no brightness-capable panel, so Windows hides that
//! row; we do the same. All metrics come from a full-resolution capture.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::button::Input;
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
    KillTimer, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage,
    WA_INACTIVE, WM_ACTIVATE, WM_TIMER,
    WM_CAPTURECHANGED, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_PAINT, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const WIDTH: f32 = 358.0;
const HEIGHT: f32 = 330.0;
const TILE_W: f32 = 94.0;
const TILE_H: f32 = 58.0;
const TILE_PITCH: f32 = 102.0;
const TILE_X: f32 = 24.0;
const ROW1_Y: f32 = 28.0;
const ROW2_Y: f32 = 122.0;
const DIVIDER_Y: f32 = 214.0;
/// Timer id for the marquee labels.
const SCROLL_TIMER: usize = 1;
/// Frames to travel, frames to pause at the end, then repeat.
const SCROLL_FRAMES: u32 = 160;
const SCROLL_PAUSE: u32 = 45;
const SCROLL_STEP: f32 = 0.25;

// Glyphs from the official Segoe Fluent Icons 1.56 table (Microsoft Learn).
const TILES: [(&str, &str); 6] = [
    ("\u{E701}", "Disponível"),    // Wifi
    ("\u{E702}", "Não conectado"), // Bluetooth
    ("\u{E709}", "Modo avião"),    // Airplane
    ("\u{E776}", "Acessibilidade"), // EaseOfAccess
    ("\u{F432}", "Economia de energia"), // BatterySaver
    ("\u{E7F0}", "Legendas ao vivo"), // CC
];

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy)]
struct Tile {
    on: bool,
}

struct App {
    surface: Surface,
    theme: Theme,
    formats: Formats,
    volume: Option<Volume>,
    radios: Option<Radios>,
    radio_infos: Vec<RadioInfo>,
    volume_slider: Slider,
    tiles: [Tile; 6],
    captured: bool,
    focus: bool,
    scroll: u32,
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
        let symbol = s.icon_format(18.0)?;
        s.begin();
        // Real flyout renders as dark gray (33..44 on this desktop); the WinUI
        // control-fill token composites much lighter over our host window.
        let base = if t.dark {
            windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
                r: 38.0 / 255.0, g: 38.0 / 255.0, b: 38.0 / 255.0, a: 1.0,
            }
        } else {
            windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F {
                r: 243.0 / 255.0, g: 243.0 / 255.0, b: 243.0 / 255.0, a: 1.0,
            }
        };
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, base);
        for index in 0..6 {
            let col = index % 3;
            let row = index / 3;
            let left = TILE_X + col as f32 * TILE_PITCH;
            let top = if row == 0 { ROW1_Y } else { ROW2_Y };
            let r = rect(left, top, left + TILE_W, top + TILE_H);
            s.fill_round(r, 6.0, if self.tiles[index].on { t.accent } else { t.control_alt_fill_secondary });
            // Windows keeps the glyph on the left of an accent tile but
            // centres it on an inactive one.
            let glyph_left = if self.tiles[index].on { left + 12.0 } else { left + 30.0 };
            s.text_no_wrap(TILES[index].0, &symbol,
                rect(glyph_left, top + 4.0, glyph_left + 28.0, top + 36.0),
                if self.tiles[index].on { t.text_on_accent } else { t.text_primary });
            let small = s.icon_format(12.0)?;
            s.text_no_wrap("\u{E76C}", &small,
                rect(left + TILE_W - 26.0, top + 12.0, left + TILE_W - 8.0, top + 40.0),
                if self.tiles[index].on { t.text_on_accent } else { t.text_secondary });
            let label = TILES[index].1;
            let label_rect = rect(left - 4.0, top + TILE_H + 2.0,
                left + TILE_W - 4.0, top + TILE_H + 18.0);
            let available = label_rect.right - label_rect.left;
            let (text_w, _) = s.measure(label, &f.caption)?;
            let chars: Vec<char> = label.chars().collect();
            if text_w <= available || chars.is_empty() {
                s.text_no_wrap(label, &f.caption, label_rect, t.text_primary);
            } else {
                // Reveal the rest of a long label sideways, one character at a
                // time, then pause and start over. Drawing a substring avoids
                // bleeding into the neighbouring tile's label.
                let per_char = text_w / chars.len() as f32;
                let max_offset = text_w - available;
                let phase = self.scroll % (SCROLL_FRAMES + SCROLL_PAUSE);
                let offset = if phase < SCROLL_FRAMES {
                    (phase as f32 * SCROLL_STEP).min(max_offset)
                } else {
                    max_offset
                };
                let skip = (offset / per_char) as usize;
                let take = ((available + 1.0) / per_char) as usize;
                let shown: String = chars.iter().skip(skip).take(take).collect();
                s.text_no_wrap(&shown, &f.caption, label_rect, t.text_primary);
            }
        }
        s.fill_round(rect(0.0, DIVIDER_Y, w, DIVIDER_Y + 1.0), 0.0, t.divider);
        s.text_no_wrap("\u{E767}", &symbol, rect(24.0, 232.0, 56.0, 264.0), t.text_primary);
        self.volume_slider.draw(s, t)?;
        s.text_no_wrap("\u{E713}", &symbol, rect(320.0, 232.0, 348.0, 264.0), t.text_secondary);
        s.text_no_wrap("\u{E713}", &symbol, rect(w - 44.0, h - 36.0, w - 16.0, h - 8.0), t.text_secondary);
        s.end()
    }

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
            _ => return,
        }
        self.sync_tiles();
    }

    fn point(&self, lp: LPARAM) -> (f32, f32) {
        let k = self.surface.scale();
        ((lp.0 & 0xffff) as f32 / k, ((lp.0 >> 16) & 0xffff) as f32 / k)
    }

    fn down(&mut self, x: f32, y: f32) -> bool {
        for index in 0..6 {
            let col = index % 3;
            let row = index / 3;
            let left = TILE_X + col as f32 * TILE_PITCH;
            let top = if row == 0 { ROW1_Y } else { ROW2_Y };
            if x >= left && x < left + TILE_W && y >= top && y < top + TILE_H {
                self.toggle_tile(index);
                return false;
            }
        }
        let r = self.volume_slider.rect;
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            self.captured = true;
            self.focus = true;
            let before = self.volume_slider.value();
            let out = self.volume_slider.handle(Input::PointerDown(x, y));
            self.apply_volume(before);
            return out.capture == Some(true);
        }
        false
    }

    fn apply_volume(&mut self, before: f64) {
        if self.volume_slider.value() != before
            && let Some(volume) = &self.volume
        {
            _ = volume.set_level(quick_settings::level(self.volume_slider.value() as u32));
        }
    }

    fn move_to(&mut self, x: f32, y: f32) -> bool {
        if !self.captured {
            return false;
        }
        let before = self.volume_slider.value();
        _ = self.volume_slider.handle(Input::PointerMove(x, y));
        self.apply_volume(before);
        self.volume_slider.value() != before
    }

    fn up(&mut self, x: f32, y: f32) {
        if self.captured {
            let before = self.volume_slider.value();
            _ = self.volume_slider.handle(Input::PointerUp(x, y));
            self.apply_volume(before);
            self.captured = false;
        }
    }

    fn key(&mut self, vk: VIRTUAL_KEY, down: bool) {
        let key = match vk {
            VK_LEFT | VK_DOWN => fluent_win::button::Key::Left,
            VK_RIGHT | VK_UP => fluent_win::button::Key::Right,
            VK_HOME => fluent_win::button::Key::Home,
            VK_END => fluent_win::button::Key::End,
            _ => return,
        };
        if !self.focus {
            return;
        }
        let before = self.volume_slider.value();
        let input = if down { Input::KeyDown(key) } else { Input::KeyUp(key) };
        _ = self.volume_slider.handle(input);
        self.apply_volume(before);
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
            app.captured = false;
            Ok(())
        }),
        WM_TIMER if wp.0 == SCROLL_TIMER => with_app(|app| {
            app.scroll = app.scroll.wrapping_add(1);
            app.render()
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
            _ = KillTimer(Some(hwnd), SCROLL_TIMER);
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
        // Real flyout sits at the bottom-right of the primary monitor.
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class,
            w!("fluent-win · Configurações rápidas"), WS_POPUP | WS_VISIBLE,
            2211, 826, WIDTH as i32, HEIGHT as i32, None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let volume = Volume::new().ok();
        let level = volume.as_ref().and_then(|v| v.level().ok()).unwrap_or(0.5);
        let radios = Radios::load().ok();
        let radio_infos = radios.as_ref().and_then(|r| r.info().ok()).unwrap_or_default();
        let volume_slider = Slider::new("Volume", rect(62.0, 232.0, 306.0, 264.0),
            0.0, 100.0, quick_settings::percent(level) as f64);
        let mut app = App {
            surface, theme, formats, volume, radios, radio_infos, volume_slider,
            tiles: [Tile { on: false }; 6], captured: false, focus: false, scroll: 0,
        };
        app.sync_tiles();
        _ = SetTimer(Some(hwnd), SCROLL_TIMER, 40, None);
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
