//! Fase 3: Start menu with a virtualized pinned grid, app icons and search.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::app_list::{self, AppInfo};
use fluent_win::backdrop;
use fluent_win::gfx::{Surface, rect};
use fluent_win::grid_view::{Grid, CELL, GAP};
use fluent_win::icons::{self, IconCache};
use fluent_win::panel::Formats;
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
const HEIGHT: f32 = 640.0;
const GRID_TOP: f32 = 96.0;

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
    search: TextBox,
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
        for (index, cell) in self.grid.arrange(24.0, GRID_TOP) {
            if let Some(icon) = self.icons.get(index).and_then(|icon| icon.as_ref()) {
                let size = 32.0;
                icons::draw(s, icon, cell.left + (CELL - size) / 2.0, cell.top + 8.0, size);
            }
            let label: String = self.apps[index].name.chars().take(12).collect();
            s.text_no_wrap(&label, &f.caption,
                rect(cell.left + 4.0, cell.top + 48.0, cell.right - 4.0, cell.bottom), t.text_primary);
        }
        s.end()
    }

    fn click(&mut self, x: f32, y: f32) -> Result<()> {
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
        let mut cache = IconCache::new();
        let icons = apps.iter()
            .map(|app| cache.get(&surface, &app.path.to_string_lossy(), 32).ok().cloned())
            .collect();
        let mut search = TextBox::new("");
        search.placeholder = app_list::SEARCH_PLACEHOLDER.to_owned();
        let mut grid = Grid::new(apps.len());
        grid.set_viewport(HEIGHT - GRID_TOP - GAP);
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { hwnd, surface, theme, formats, grid, apps, icons, search });
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
