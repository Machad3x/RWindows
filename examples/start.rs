//! Fase 3: Start menu with a virtualized pinned grid, app icons and search.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::app_list::{self, AppInfo};
use fluent_win::backdrop;
use fluent_win::gfx::{Surface, rect};
use fluent_win::grid_view::{Grid, CELL, GAP};
use fluent_win::icons::{self, IconCache};
use fluent_win::panel::Formats;
use fluent_win::session::{self, PowerAction};
use fluent_win::text_box::TextBox;
use fluent_win::theme::Theme;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, TranslateMessage,
    WA_INACTIVE, WM_ACTIVATE, WM_CHAR, WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSW,
    WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 700.0;
const GRID_TOP: f32 = 96.0;
/// Windows 11 shows three rows of pinned tiles before "Recente".
const PINNED_ROWS: usize = 3;
const PINNED_END: f32 = GRID_TOP + PINNED_ROWS as f32 * (CELL + GAP);
const RECENT_TOP: f32 = PINNED_END + 24.0;
const RECENT_GRID_TOP: f32 = RECENT_TOP + 28.0;
const RECENT_COLUMNS: usize = 3;
const RECENT_W: f32 = 190.0;
const RECENT_H: f32 = 60.0;
const ALL_TOP: f32 = RECENT_GRID_TOP + 2.0 * RECENT_H + 16.0;
const ALL_GRID_TOP: f32 = ALL_TOP + 28.0;
/// Bottom bar with the account and the power button, as in Windows 11.
const POWER_BAR: f32 = 56.0;
const POWER_ACTIONS: [(&str, PowerAction); 5] = [
    ("Bloquear", PowerAction::Lock),
    ("Sair", PowerAction::Logoff),
    ("Dormir", PowerAction::Sleep),
    ("Reiniciar", PowerAction::Restart),
    ("Desligar", PowerAction::Shutdown),
];
/// Power button at the bottom-right of the account bar.
fn power_rect(w: f32, h: f32) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    rect(w - 56.0, h - 48.0, w - 16.0, h - 8.0)
}
fn power_item_rect(w: f32, h: f32, index: usize) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    rect(w - 176.0, h - 48.0 - (5 - index) as f32 * 32.0, w - 16.0,
        h - 48.0 - (4 - index) as f32 * 32.0)
}
/// Cell of the recent-files grid.
fn recent_rect(index: usize) -> windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F {
    let column = index % RECENT_COLUMNS;
    let row = index / RECENT_COLUMNS;
    let left = 24.0 + column as f32 * RECENT_W;
    let top = RECENT_GRID_TOP + row as f32 * RECENT_H;
    rect(left, top, left + RECENT_W - 8.0, top + RECENT_H)
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    formats: Formats,
    grid: Grid,
    apps: Vec<AppInfo>,
    icons: Vec<Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>>,
    recent: Vec<AppInfo>,
    recent_icons: Vec<Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>>,
    search: TextBox,
    power_open: bool,
    /// Destructive actions need a second click on the same item.
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

impl App {
    fn render(&mut self) -> Result<()> {
        let (w, h) = self.surface.size();
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        s.begin();
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, t.card);
        self.search.draw(s, t, f, rect(24.0, 28.0, w - 24.0, 72.0), true)?;
        s.text_no_wrap("Fixados", &f.body, rect(24.0, 68.0, w - 24.0, GRID_TOP), t.text_primary);
        let cells = self.grid.arrange(24.0, GRID_TOP);
        s.with_clip(rect(0.0, GRID_TOP, w, PINNED_END), |s| {
            for (index, cell) in cells {
                if let Some(icon) = self.icons.get(index).and_then(|icon| icon.as_ref()) {
                    let size = 32.0;
                    icons::draw(s, icon, cell.left + (CELL - size) / 2.0, cell.top + 8.0, size);
                }
                let label: String = self.apps[index].name.chars().take(12).collect();
                s.text_no_wrap(&label, &f.caption,
                    rect(cell.left + 4.0, cell.top + 48.0, cell.right - 4.0, cell.bottom), t.text_primary);
            }
        });
        s.text_no_wrap("Recente", &f.body,
            rect(24.0, RECENT_TOP, w - 24.0, RECENT_TOP + 24.0), t.text_primary);
        for (index, item) in self.recent.iter().enumerate() {
            let r = recent_rect(index);
            if let Some(icon) = self.recent_icons.get(index).and_then(|icon| icon.as_ref()) {
                icons::draw(s, icon, r.left, r.top, 24.0);
            }
            let label: String = item.name.chars().take(22).collect();
            s.text_no_wrap(&label, &f.caption,
                rect(r.left + 32.0, r.top, r.right, r.top + 26.0), t.text_primary);
            s.text_no_wrap("Adicionados recentemente", &f.caption,
                rect(r.left + 32.0, r.top + 26.0, r.right, r.bottom), t.text_secondary);
        }
        s.text_no_wrap("Todos os aplicativos", &f.body,
            rect(24.0, ALL_TOP, w - 24.0, ALL_TOP + 24.0), t.text_primary);
        s.text_no_wrap("Exibir tudo ›", &f.caption,
            rect(w - 140.0, ALL_TOP, w - 24.0, ALL_TOP + 24.0), t.text_secondary);
        for (offset, index) in (PINNED_ROWS * fluent_win::grid_view::COLUMNS
            ..self.apps.len()).take(fluent_win::grid_view::COLUMNS).enumerate()
        {
            let left = 24.0 + offset as f32 * (CELL + GAP);
            let top = ALL_GRID_TOP;
            if let Some(icon) = self.icons.get(index).and_then(|icon| icon.as_ref()) {
                icons::draw(s, icon, left + (CELL - 32.0) / 2.0, top + 8.0, 32.0);
            }
            let label: String = self.apps[index].name.chars().take(12).collect();
            s.text_no_wrap(&label, &f.caption,
                rect(left + 4.0, top + 48.0, left + CELL - 4.0, top + CELL), t.text_primary);
        }
        s.fill_round(rect(0.0, h - POWER_BAR, w, h), 0.0, t.control_alt_fill_secondary);
        s.text_no_wrap("Usuário", &f.body,
            rect(24.0, h - POWER_BAR, w - 80.0, h), t.text_primary);
        let power = power_rect(w, h);
        s.fill_round(power, 8.0, t.card);
        s.text_no_wrap("⏻", &f.body, power, t.text_primary);
        if self.power_open {
            for (index, (label, action)) in POWER_ACTIONS.iter().enumerate() {
                let r = power_item_rect(w, h, index);
                // Opaque token: the grid is behind the menu and must not bleed through.
                s.fill_round(r, 4.0, if self.pending == Some(*action) { t.accent } else { t.control_solid_fill });
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

    fn click(&mut self, x: f32, y: f32) -> Result<()> {
        let (w, h) = self.surface.size();
        if self.power_open {
            for (index, (_, action)) in POWER_ACTIONS.iter().enumerate() {
                let r = power_item_rect(w, h, index);
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
        let r = power_rect(w, h);
        if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
            self.power_open = true;
            return Ok(());
        }
        for (index, item) in self.recent.iter().enumerate() {
            let r = recent_rect(index);
            if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                unsafe {
                    _ = ShellExecuteW(Some(self.hwnd), w!("open"),
                        &windows::core::HSTRING::from(item.path.to_string_lossy().as_ref()),
                        None, None, SW_SHOW);
                }
                unsafe { _ = DestroyWindow(self.hwnd) };
                return Ok(());
            }
        }
        let Some(index) = self.grid.hit(x, y, 24.0, GRID_TOP) else { return Ok(()) };
        let Some(app) = self.apps.get(index) else { return Ok(()) };
        unsafe {
            _ = ShellExecuteW(Some(self.hwnd), w!("open"),
                &windows::core::HSTRING::from(app.path.to_string_lossy().as_ref()),
                None, None, SW_SHOW);
        }
        unsafe { _ = DestroyWindow(self.hwnd) };
        Ok(())
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => with_app(App::render),
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
        WM_ACTIVATE if u32::from((wp.0 & 0xffff) as u16) == WA_INACTIVE => {
            unsafe { _ = DestroyWindow(hwnd) };
            LRESULT(0)
        }
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
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Iniciar"),
            WS_POPUP | WS_VISIBLE, 200, 120, WIDTH as i32, HEIGHT as i32,
            None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let apps = app_list::enumerate()?;
        let recent = app_list::recent().unwrap_or_default();
        let mut cache = IconCache::new();
        let icons = apps.iter()
            .map(|app| cache.get(&surface, &app.path.to_string_lossy(), 32).ok().cloned())
            .collect();
        let recent_icons = recent.iter()
            .map(|item| cache.get(&surface, &item.path.to_string_lossy(), 24).ok().cloned())
            .collect();
        let mut search = TextBox::new("");
        search.placeholder = app_list::SEARCH_PLACEHOLDER.to_owned();
        let mut grid = Grid::new(apps.len());
        grid.set_viewport(PINNED_ROWS as f32 * (CELL + GAP));
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { hwnd, surface, theme, formats, grid, apps, icons,
                recent, recent_icons, search, power_open: false, pending: None });
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
