//! Fase 3: Start menu measured against the real Windows 11 panel on this
//! machine (834x835 px at 100% DPI). All positions below come from a pixel
//! analysis of a full-resolution capture, not from estimates.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::app_list::{self, AppInfo, Category};
use fluent_win::backdrop;
use fluent_win::gfx::{Surface, rect};
use fluent_win::metrics;
use fluent_win::panel::Formats;
use fluent_win::session::{self, PowerAction};
use fluent_win::text_box::TextBox;
use fluent_win::theme::{self, Theme};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, TranslateMessage, WA_INACTIVE,
    WM_ACTIVATE, WM_CHAR, WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSW,
    WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const WIDTH: f32 = 834.0;
const HEIGHT: f32 = 835.0;

// Measured positions in the real panel.
const PAD: f32 = 65.0;
const SEARCH: (f32, f32, f32, f32) = (32.0, 5.0, 802.0, 37.0);
const FIXED_LABEL_Y: f32 = 84.0;
const PIN_Y: f32 = 122.0;
const PIN_PITCH: f32 = 92.0;
const PIN_ICON: f32 = 40.0;
const RECENT_LABEL_Y: f32 = 240.0;
const RECENT_TOP: f32 = 282.0;
const RECENT_PITCH_X: f32 = 250.0;
const RECENT_PITCH_Y: f32 = 56.0;
const RECENT_ICON: f32 = 28.0;
const ALL_LABEL_Y: f32 = 425.0;
const CAT_TOP: f32 = 480.0;
const CAT_PITCH_X: f32 = 183.0;
const CAT_PITCH_Y: f32 = 208.0;
const CAT_W: f32 = 155.0;
const CAT_H: f32 = 108.0;
const BAR_TOP: f32 = 795.0;

const POWER_ACTIONS: [(&str, PowerAction); 5] = [
    ("Bloquear", PowerAction::Lock),
    ("Sair", PowerAction::Logoff),
    ("Dormir", PowerAction::Sleep),
    ("Reiniciar", PowerAction::Restart),
    ("Desligar", PowerAction::Shutdown),
];

type Bitmap = windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;

fn power_rect() -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    rect(WIDTH - 92.0, BAR_TOP + 4.0, WIDTH - 60.0, BAR_TOP + 36.0)
}
fn power_item_rect(index: usize) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    rect(WIDTH - 200.0, BAR_TOP - (5 - index) as f32 * 32.0, WIDTH - 52.0,
        BAR_TOP - (4 - index) as f32 * 32.0)
}
fn pin_rect(index: usize) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    let left = PAD + index as f32 * PIN_PITCH - 12.0;
    rect(left, PIN_Y, left + PIN_PITCH - 8.0, PIN_Y + 78.0)
}
fn recent_rect(index: usize) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    let left = PAD + (index % 3) as f32 * RECENT_PITCH_X;
    let top = RECENT_TOP + (index / 3) as f32 * RECENT_PITCH_Y;
    rect(left - 4.0, top, left + RECENT_PITCH_X - 12.0, top + RECENT_PITCH_Y)
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    formats: Formats,
    search: TextBox,
    pinned: Vec<AppInfo>,
    pinned_icons: Vec<Option<Bitmap>>,
    recent: Vec<AppInfo>,
    recent_icons: Vec<Option<Bitmap>>,
    categories: Vec<Category>,
    category_icons: Vec<Vec<Option<Bitmap>>>,
    user: String,
    power_open: bool,
    pending: Option<PowerAction>,
}

fn with_app(f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win start: {e}");
        }
    });
    LRESULT(0)
}

/// Splits a label into at most two lines, breaking near the middle.
fn label_lines(name: &str) -> Vec<String> {
    if name.len() <= 14 {
        return vec![name.to_owned()];
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() < 2 {
        return vec![name.chars().take(14).collect()];
    }
    let mut best = 0usize;
    let mut delta = usize::MAX;
    let mut used = 0usize;
    for (index, word) in words.iter().enumerate() {
        used += word.len() + 1;
        let diff = used.abs_diff(name.len() / 2);
        if diff < delta {
            delta = diff;
            best = index;
        }
    }
    vec![words[..=best].join(" "), words[best + 1..].join(" ")]
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (w, h) = self.surface.size();
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        s.begin();
        // DWM TransientWindow supplies the blurred Acrylic; the theme tint
        // keeps it subdued like the real Start, in light or dark mode.
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, theme::acrylic_tint(t));
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, t.card);
        self.search.draw(s, t, f, rect(SEARCH.0, SEARCH.1, SEARCH.2, SEARCH.3), false)?;

        s.text_no_wrap("Fixado", &f.body,
            rect(PAD, FIXED_LABEL_Y, w - PAD, FIXED_LABEL_Y + 20.0), t.text_primary);
        for (index, app) in self.pinned.iter().enumerate() {
            let r = pin_rect(index);
            let center = r.left + 12.0 + PIN_ICON / 2.0;
            if let Some(icon) = self.pinned_icons.get(index).and_then(|icon| icon.as_ref()) {
                icons_draw(s, icon, center - PIN_ICON / 2.0, PIN_Y, PIN_ICON);
            }
            for (line, text) in label_lines(&app.name).into_iter().enumerate() {
                s.text_no_wrap(&text, &f.caption,
                    rect(r.left, PIN_Y + 46.0 + line as f32 * 15.0, r.right, PIN_Y + 62.0 + line as f32 * 15.0),
                    t.text_primary);
            }
        }

        s.text_no_wrap("Recente", &f.body,
            rect(PAD, RECENT_LABEL_Y, w - PAD, RECENT_LABEL_Y + 20.0), t.text_primary);
        s.text_no_wrap("Mostrar tudo  ›", &f.caption,
            rect(w - 180.0, RECENT_LABEL_Y, w - PAD, RECENT_LABEL_Y + 20.0), t.text_secondary);
        for (index, app) in self.recent.iter().enumerate() {
            let r = recent_rect(index);
            if let Some(icon) = self.recent_icons.get(index).and_then(|icon| icon.as_ref()) {
                icons_draw(s, icon, PAD + (index % 3) as f32 * RECENT_PITCH_X, r.top + 2.0, RECENT_ICON);
            }
            let text_x = PAD + (index % 3) as f32 * RECENT_PITCH_X + 40.0;
            s.text_no_wrap(&app.name, &f.caption,
                rect(text_x, r.top, r.right, r.top + 18.0), t.text_primary);
            s.text_no_wrap("Adicionados recentemente", &f.caption,
                rect(text_x, r.top + 18.0, r.right, r.top + 36.0), t.text_secondary);
        }

        s.text_no_wrap("Todos", &f.body,
            rect(PAD, ALL_LABEL_Y, w - PAD, ALL_LABEL_Y + 20.0), t.text_primary);
        s.text_no_wrap("Exibição: Categoria  ⌄", &f.caption,
            rect(w - 220.0, ALL_LABEL_Y, w - PAD, ALL_LABEL_Y + 20.0), t.text_secondary);
        for (index, category) in self.categories.iter().enumerate().take(8) {
            let col = index % 4;
            let row = index / 4;
            let left = PAD + col as f32 * CAT_PITCH_X;
            let top = CAT_TOP + row as f32 * CAT_PITCH_Y;
            s.fill_round(rect(left, top, left + CAT_W, top + CAT_H), 8.0, t.control_alt_fill_secondary);
            for (slot, icon) in self.category_icons.get(index).into_iter().flatten().enumerate() {
                if let Some(bitmap) = icon.as_ref() {
                    let ix = left + 20.0 + (slot % 2) as f32 * 72.0;
                    let iy = top + 16.0 + (slot / 2) as f32 * 48.0;
                    icons_draw(s, bitmap, ix, iy, 30.0);
                }
            }
            let label: String = category.name.chars().take(20).collect();
            s.text_no_wrap(&label, &f.caption,
                rect(left, top + CAT_H + 6.0, left + CAT_W, top + CAT_H + 26.0), t.text_primary);
        }

        s.fill_round(rect(0.0, BAR_TOP, w, h), 0.0, t.control_alt_fill_secondary);
        s.fill_round(rect(PAD, BAR_TOP + 8.0, PAD + 30.0, BAR_TOP + 38.0), 15.0, t.text_secondary);
        s.text_no_wrap(&self.user, &f.body,
            rect(PAD + 40.0, BAR_TOP + 6.0, w - 100.0, BAR_TOP + 40.0), t.text_primary);
        let power = power_rect();
        s.text_no_wrap("⏻", &f.body, power, t.text_primary);
        if self.power_open {
            for (index, (label, action)) in POWER_ACTIONS.iter().enumerate() {
                let r = power_item_rect(index);
                s.fill_round(r, 4.0,
                    if self.pending == Some(*action) { t.accent } else { t.control_solid_fill });
                s.stroke_round(r, 4.0, 1.0, t.stroke);
                let text = if self.pending == Some(*action) {
                    format!("Confirmar: {}", label)
                } else {
                    (*label).to_owned()
                };
                s.text_no_wrap(&text, &f.body, r,
                    if self.pending == Some(*action) { t.text_on_accent } else { t.text_primary });
            }
        }
        s.end()
    }

    fn launch(&self, app: &AppInfo) -> Result<()> {
        unsafe {
            _ = ShellExecuteW(Some(self.hwnd), w!("open"),
                &windows::core::HSTRING::from(app.path.to_string_lossy().as_ref()),
                None, None, SW_SHOW);
            _ = DestroyWindow(self.hwnd);
        }
        Ok(())
    }

    fn click(&mut self, x: f32, y: f32) -> Result<()> {
        if self.power_open {
            for (index, (_, action)) in POWER_ACTIONS.iter().enumerate() {
                let r = power_item_rect(index);
                if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                    if self.pending == Some(*action) || *action == PowerAction::Lock {
                        return session::perform(*action);
                    }
                    self.pending = Some(*action);
                    return Ok(());
                }
            }
            self.power_open = false;
            self.pending = None;
            return Ok(());
        }
        let r = power_rect();
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            self.power_open = true;
            return Ok(());
        }
        for index in 0..self.pinned.len().min(6) {
            let r = pin_rect(index);
            if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                return self.launch(&self.pinned[index].clone());
            }
        }
        for index in 0..self.recent.len() {
            let r = recent_rect(index);
            if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                return self.launch(&self.recent[index].clone());
            }
        }
        for (index, category) in self.categories.iter().enumerate().take(8) {
            let left = PAD + (index % 4) as f32 * CAT_PITCH_X;
            let top = CAT_TOP + (index / 4) as f32 * CAT_PITCH_Y;
            if x >= left && x < left + CAT_W && y >= top && y < top + CAT_H
                && let Some(app) = category.apps.first()
            {
                return self.launch(&app.clone());
            }
        }
        Ok(())
    }
}

fn icons_draw(surface: &Surface, bitmap: &Bitmap, x: f32, y: f32, size: f32) {
    fluent_win::icons::draw(surface, bitmap, x, y, size);
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = windows::Win32::Graphics::Gdi::ValidateRect(Some(hwnd), None) };
            with_app(App::render)
        }
        WM_CHAR => with_app(|app| {
            if let Some(c) = char::from_u32(wp.0 as u32)
                && !c.is_control()
            {
                app.search.insert(&c.to_string());
            }
            app.render()
        }),
        WM_LBUTTONDOWN => with_app(|app| {
            let k = app.surface.scale();
            app.click((lp.0 & 0xffff) as f32 / k, ((lp.0 >> 16) & 0xffff) as f32 / k)
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
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-start");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let (x, y) = metrics::centered_bottom(WIDTH, HEIGHT);
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Iniciar"),
            WS_POPUP | WS_VISIBLE, x, y, WIDTH as i32, HEIGHT as i32,
            None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let apps = app_list::enumerate()?;
        let recent = app_list::recent().unwrap_or_default();
        let categories = app_list::categories().unwrap_or_default();
        let mut cache = fluent_win::icons::IconCache::new();
        let mut load = |path: &std::path::Path, size: u32| {
            cache.get(&surface, &path.to_string_lossy(), size).ok().cloned()
        };
        let pinned: Vec<AppInfo> = apps.iter().take(6).cloned().collect();
        let pinned_icons = pinned.iter().map(|app| load(&app.path, 40)).collect();
        let recent_icons = recent.iter().map(|app| load(&app.path, 28)).collect();
        let category_icons = categories.iter().map(|category| {
            category.apps.iter().map(|app| load(&app.path, 30)).collect()
        }).collect();
        let mut search = TextBox::new("");
        search.placeholder = app_list::SEARCH_PLACEHOLDER.to_owned();
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "Usuário".to_owned());
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App {
                hwnd, surface, theme, formats, search, pinned, pinned_icons,
                recent, recent_icons, categories, category_icons, user,
                power_open: false, pending: None,
            });
        });
        with_app(App::render);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
