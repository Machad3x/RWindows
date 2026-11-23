//! Fase 3: notification center with a real calendar grid. Windows toast
//! notifications need a packaged app identity, so the list is a placeholder.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fluent_win::backdrop;
use fluent_win::calendar::{self, Date};
use fluent_win::gfx::{Surface, rect};
use fluent_win::metrics;
use fluent_win::panel::Formats;
use fluent_win::theme::Theme;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, TranslateMessage, WA_INACTIVE, WM_ACTIVATE,
    WM_DESTROY, WM_PAINT, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

const W: f32 = 360.0;
const H: f32 = 472.0;
const PAD: f32 = 16.0;
struct App {
    surface: Surface,
    theme: Theme,
    formats: Formats,
    today: Date,
    cells: Vec<Option<u32>>,
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (w, h) = self.surface.size();
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        s.begin();
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, t.card);
        let title = format!("{} de {} de {}", self.today.day,
            calendar::month_name(self.today.month), self.today.year);
        s.text_no_wrap(&title, &f.body, rect(PAD, 14.0, w - PAD, 48.0), t.text_primary);
        let cell_w = (w - 2.0 * PAD) / 7.0;
        for col in 0..7 {
            let left = PAD + col as f32 * cell_w;
            let name = calendar::weekday_name(col);
            s.text_no_wrap(&name, &f.caption, rect(left, 56.0, left + cell_w, 80.0), t.text_secondary);
        }
        for (index, day) in self.cells.iter().enumerate() {
            let Some(day) = day else { continue };
            let col = index % 7;
            let row = index / 7;
            let left = PAD + col as f32 * cell_w;
            let top = 84.0 + row as f32 * 34.0;
            if *day == self.today.day {
                s.fill_round(rect(left + 8.0, top + 2.0, left + cell_w - 8.0, top + 30.0), 12.0, t.accent);
            }
            s.text_no_wrap(&day.to_string(), &f.caption,
                rect(left, top, left + cell_w, top + 32.0),
                if *day == self.today.day { t.text_on_accent } else { t.text_primary });
        }
        s.text_no_wrap("Notificações", &f.body, rect(PAD, 296.0, w - PAD, 328.0), t.text_primary);
        s.text_no_wrap("Nenhuma nova notificação", &f.caption,
            rect(PAD, 334.0, w - PAD, 364.0), t.text_secondary);
        s.end()
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            // No shared helper: this example owns its thread-local state.
            APP.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut()
                    && let Some(app) = guard.as_mut()
                    && let Err(e) = app.render()
                {
                    eprintln!("fluent-win notifications: {e}");
                }
            });
            LRESULT(0)
        }
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

thread_local! {
    static APP: std::cell::RefCell<Option<App>> = const { std::cell::RefCell::new(None) };
}

fn main() -> Result<()> {
    unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-notifications");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let (x, y) = metrics::bottom_right(W, H, 16.0);
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class,
            w!("fluent-win · Notificações"), WS_POPUP | WS_VISIBLE,
            x, y, W as i32, H as i32, None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let today = calendar::today();
        let cells = calendar::grid(today.year, today.month);
        APP.with(|cell| *cell.borrow_mut() = Some(App { surface, theme, formats, today, cells }));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
