//! Fase 3: notification center measured against the real Windows 11 panel
//! (340x835 at 100% DPI on this machine). Real toasts need a packaged app
//! identity, so the list is a placeholder; the calendar and focus row are real.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use fluent_win::backdrop;
use fluent_win::calendar::{self, Date};
use fluent_win::gfx::{Surface, rect};
use fluent_win::metrics;
use fluent_win::panel::Formats;
use fluent_win::theme::{self, Theme};
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

const W: f32 = 340.0;
const H: f32 = 835.0;
const PAD: f32 = 12.0;
const CAL_TOP: f32 = 380.0;
const MONTH_TOP: f32 = 432.0;
const WEEK_TOP: f32 = 492.0;
const GRID_TOP: f32 = 536.0;
const CELL: f32 = 40.0;
const FOCUS_TOP: f32 = 792.0;

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
        let symbol = s.icon_format(14.0)?;
        s.begin();
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, theme::acrylic_tint(t));
        s.fill_round(rect(0.0, 0.0, w, h), 8.0, t.card);

        s.text_no_wrap("Notificações", &f.body,
            rect(PAD, 12.0, w - PAD - 70.0, 44.0), t.text_primary);
        s.fill_round(rect(w - 108.0, 16.0, w - PAD, 40.0), 12.0, t.control_alt_fill_secondary);
        s.text_no_wrap("Limpar tudo", &f.caption,
            rect(w - 104.0, 16.0, w - PAD - 4.0, 40.0), t.text_secondary);

        s.fill_round(rect(PAD, 56.0, w - PAD, 148.0), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("Nenhuma nova notificação", &f.caption,
            rect(PAD + 12.0, 88.0, w - PAD - 12.0, 116.0), t.text_secondary);

        s.fill_round(rect(0.0, CAL_TOP, w, CAL_TOP + 1.0), 0.0, t.divider);
        let long_date = format!("{}, {} de {}",
            weekday_long(calendar::weekday(self.today)), self.today.day,
            calendar::month_name(self.today.month));
        s.text_no_wrap(&long_date, &f.body,
            rect(PAD, CAL_TOP + 8.0, w - PAD - 30.0, CAL_TOP + 36.0), t.text_primary);
        s.text_no_wrap("\u{E70D}", &symbol,
            rect(w - 32.0, CAL_TOP + 8.0, w - PAD, CAL_TOP + 36.0), t.text_secondary);

        let month_title = format!("{} de {}", calendar::month_name(self.today.month), self.today.year);
        s.text_no_wrap(&month_title, &f.body,
            rect(PAD, MONTH_TOP, w - PAD, MONTH_TOP + 26.0), t.text_primary);
        s.text_no_wrap("\u{E70E}", &symbol,
            rect(w - 62.0, MONTH_TOP, w - 38.0, MONTH_TOP + 26.0), t.text_secondary);
        s.text_no_wrap("\u{E70D}", &symbol,
            rect(w - 34.0, MONTH_TOP, w - PAD, MONTH_TOP + 26.0), t.text_secondary);

        let cell_w = (w - 2.0 * PAD) / 7.0;
        for col in 0..7 {
            let left = PAD + col as f32 * cell_w;
            // Real panel uses single-letter weekdays from the locale.
            let name = calendar::weekday_name(col);
            let initial: String = name.chars().take(1).collect();
            s.text_no_wrap(&initial, &f.caption,
                rect(left, WEEK_TOP, left + cell_w, WEEK_TOP + 24.0), t.text_secondary);
        }
        for (index, day) in self.cells.iter().enumerate() {
            let Some(day) = day else { continue };
            let col = index % 7;
            let row = index / 7;
            let left = PAD + col as f32 * cell_w;
            let top = GRID_TOP + row as f32 * CELL;
            if *day == self.today.day {
                s.fill_round(rect(left + 8.0, top + 2.0, left + cell_w - 8.0, top + CELL - 6.0),
                    12.0, t.accent);
            }
            s.text_no_wrap(&day.to_string(), &f.caption,
                rect(left, top, left + cell_w, top + CELL - 4.0),
                if *day == self.today.day { t.text_on_accent } else { t.text_primary });
        }

        s.fill_round(rect(PAD, FOCUS_TOP, 26.0, FOCUS_TOP + 28.0), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("−", &f.body, rect(PAD, FOCUS_TOP, PAD + 26.0, FOCUS_TOP + 28.0), t.text_primary);
        s.text_no_wrap("30 minutos", &f.caption,
            rect(PAD + 32.0, FOCUS_TOP, PAD + 120.0, FOCUS_TOP + 28.0), t.text_primary);
        s.fill_round(rect(PAD + 126.0, FOCUS_TOP, PAD + 152.0, FOCUS_TOP + 28.0), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("+", &f.body,
            rect(PAD + 126.0, FOCUS_TOP, PAD + 152.0, FOCUS_TOP + 28.0), t.text_primary);
        s.fill_round(rect(w - 92.0, FOCUS_TOP, w - PAD, FOCUS_TOP + 28.0), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("Foco", &f.caption,
            rect(w - 88.0, FOCUS_TOP, w - PAD - 4.0, FOCUS_TOP + 28.0), t.text_primary);
        s.end()
    }
}

fn weekday_long(weekday: u32) -> &'static str {
    const NAMES: [&str; 7] = [
        "domingo", "segunda-feira", "terça-feira", "quarta-feira",
        "quinta-feira", "sexta-feira", "sábado",
    ];
    NAMES.get(weekday as usize).copied().unwrap_or("")
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
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
        let (x, y) = metrics::bottom_right(W, H, 8.0);
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
