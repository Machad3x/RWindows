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
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, TranslateMessage, WA_INACTIVE, WM_ACTIVATE,
    WM_DESTROY, WM_LBUTTONDOWN, WM_PAINT, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_POPUP, WS_VISIBLE,
};
use windows::core::{Result, w};

// Design tokens (DIPs), not screen measurements.
const W: f32 = 340.0;
/// Transparent margin around the panel for its drop shadow.
const SHADOW: f32 = 8.0;
const PAD: f32 = 12.0;
const CELL: f32 = 42.0;
const FOCUS_H: f32 = 28.0;
const CAL_HEADER: f32 = 36.0;
const MONTH_H: f32 = 26.0;
const WEEK_H: f32 = 24.0;

/// Calendar block height derived from its rows, not a fixed screenshot value.
fn calendar_height(expanded: bool) -> f32 {
    if expanded {
        CAL_HEADER + MONTH_H + WEEK_H + 6.0 * CELL + 8.0
    } else {
        CAL_HEADER
    }
}

/// Layout for a panel of height `h`: everything below the notification list
/// is anchored to the bottom edge, so the panel follows the work area.
fn layout(h: f32, expanded: bool) -> (f32, f32, f32, f32, f32) {
    let focus_top = h - PAD - FOCUS_H;
    let cal_top = focus_top - calendar_height(expanded) - 8.0;
    let month_top = cal_top + CAL_HEADER + 4.0;
    let week_top = month_top + MONTH_H + 4.0;
    let grid_top = week_top + WEEK_H + 4.0;
    (cal_top, month_top, week_top, grid_top, focus_top)
}

struct App {
    surface: Surface,
    theme: Theme,
    formats: Formats,
    /// Centred caption for weekday letters and grid numbers.
    centered: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
    today: Date,
    cells: Vec<(u32, bool)>,
    /// Calendar starts collapsed, like the real notification center.
    expanded: bool,
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (window_w, window_h) = self.surface.size();
        let w = window_w - 2.0 * SHADOW;
        let h = window_h - 2.0 * SHADOW;
        let r = |l: f32, t: f32, right: f32, bottom: f32| {
            rect(l + SHADOW, t + SHADOW, right + SHADOW, bottom + SHADOW)
        };
        let expanded = self.expanded;
        let (cal_top, month_top, week_top, grid_top, focus_top) = layout(h, expanded);
        let (s, t, f) = (&mut self.surface, &self.theme, &self.formats);
        let symbol = s.icon_format(14.0)?;
        s.begin();
        // Soft drop shadow: overlapping low-alpha rounded rects, widest first.
        for i in 1..=6 {
            let g = i as f32;
            s.fill_round(rect(SHADOW - g, SHADOW - g,
                    window_w - SHADOW + g, window_h - SHADOW + g),
                8.0 + g, theme::argb(0x0A000000));
        }
        s.fill_round(r(0.0, 0.0, w, h), 8.0, theme::acrylic_tint(t));
        s.fill_round(r(0.0, 0.0, w, h), 8.0, t.card);

        s.text_no_wrap("Notificações", &f.caption,
            r(PAD, 12.0, w - PAD - 110.0, 44.0), t.text_primary);
        s.text_no_wrap("\u{E74D}", &symbol,
            r(w - 138.0, 16.0, w - 114.0, 40.0), t.text_secondary);
        s.fill_round(r(w - 108.0, 16.0, w - PAD, 40.0), 12.0, t.control_alt_fill_secondary);
        s.text_no_wrap("Limpar tudo", &f.caption,
            r(w - 108.0, 16.0, w - PAD, 40.0), t.text_primary);

        // Real toasts need a packaged identity; this card keeps the shape and
        // spacing of the real ones without inventing notifications.
        s.fill_round(r(PAD, 56.0, w - PAD, 148.0), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("\u{E7E7}", &symbol,
            r(PAD + 12.0, 72.0, PAD + 44.0, 104.0), t.text_secondary);
        s.text_no_wrap("Nenhuma nova notificação", &f.caption,
            r(PAD + 48.0, 72.0, w - PAD - 12.0, 96.0), t.text_primary);
        s.text_no_wrap("As notificações dos aplicativos aparecem aqui",
            &f.caption, r(PAD + 48.0, 96.0, w - PAD - 12.0, 132.0), t.text_secondary);

        s.fill_round(r(0.0, cal_top, w, cal_top + 1.0), 0.0, t.divider);
        let long_date = calendar::long_date(self.today);
        s.text_no_wrap(&long_date, &f.caption,
            r(PAD, cal_top + 8.0, w - PAD - 30.0, cal_top + CAL_HEADER), t.text_primary);
        s.text_no_wrap(if expanded { "\u{E70E}" } else { "\u{E70D}" }, &symbol,
            r(w - 32.0, cal_top + 8.0, w - PAD, cal_top + CAL_HEADER), t.text_secondary);

        if expanded {
        let month_title = calendar::month_year(self.today);
        s.text_no_wrap(&month_title, &f.caption,
            r(PAD, month_top, w - PAD, month_top + MONTH_H), t.text_primary);
        s.text_no_wrap("\u{E70E}", &symbol,
            r(w - 62.0, month_top, w - 38.0, month_top + MONTH_H), t.text_secondary);
        s.text_no_wrap("\u{E70D}", &symbol,
            r(w - 34.0, month_top, w - PAD, month_top + MONTH_H), t.text_secondary);

        let cell_w = (w - 2.0 * PAD) / 7.0;
        let first = calendar::first_day_of_week();
        for col in 0..7 {
            let left = PAD + col as f32 * cell_w;
            // Real panel uses single-letter weekdays from the locale.
            let name = calendar::weekday_name((first + col as u32) % 7);
            let initial: String = name.chars().take(1).flat_map(char::to_uppercase).collect();
            s.text_no_wrap(&initial, &self.centered,
                r(left, week_top, left + cell_w, week_top + WEEK_H), t.text_secondary);
        }
        for (index, (day, in_month)) in self.cells.iter().enumerate() {
            let col = index % 7;
            let row = index / 7;
            let left = PAD + col as f32 * cell_w;
            let top = grid_top + row as f32 * CELL;
            let today = *in_month && *day == self.today.day;
            if today {
                // Round marker centred on the cell, as in the real calendar.
                let (cx, cy) = (left + cell_w / 2.0, top + CELL / 2.0);
                let radius = 14.0;
                s.fill_round(r(cx - radius, cy - radius, cx + radius, cy + radius), radius, t.accent);
            }
            s.text_no_wrap(&day.to_string(), &self.centered,
                r(left, top, left + cell_w, top + CELL),
                if *in_month { t.text_primary } else { t.text_secondary });
        }
        }

        let pill_h = FOCUS_H;
        s.fill_round(r(PAD, focus_top, PAD + 24.0, focus_top + pill_h), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("−", &self.centered, r(PAD, focus_top, PAD + 24.0, focus_top + pill_h), t.text_primary);
        s.text_no_wrap("30 minutos", &f.caption,
            r(PAD + 32.0, focus_top, PAD + 120.0, focus_top + pill_h), t.text_primary);
        s.fill_round(r(PAD + 126.0, focus_top, PAD + 150.0, focus_top + pill_h), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("+", &self.centered,
            r(PAD + 126.0, focus_top, PAD + 150.0, focus_top + pill_h), t.text_primary);
        s.fill_round(r(w - 92.0, focus_top, w - PAD, focus_top + pill_h), 6.0, t.control_alt_fill_secondary);
        s.text_no_wrap("\u{E768}", &symbol,
            r(w - 88.0, focus_top, w - 66.0, focus_top + pill_h), t.text_primary);
        s.text_no_wrap("Foco", &f.caption,
            r(w - 64.0, focus_top, w - PAD - 4.0, focus_top + pill_h), t.text_primary);
        s.end()
    }
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
        WM_LBUTTONDOWN => {
            APP.with(|cell| {
                if let Ok(mut guard) = cell.try_borrow_mut()
                    && let Some(app) = guard.as_mut()
                {
                    let (_, window_h) = app.surface.size();
                    let h = window_h - 2.0 * SHADOW;
                    let (cal_top, _, _, _, _) = layout(h, app.expanded);
                    let x = (lp.0 & 0xffff) as f32 / app.surface.scale() - SHADOW;
                    let y = ((lp.0 >> 16) & 0xffff) as f32 / app.surface.scale() - SHADOW;
                    if (cal_top..=cal_top + CAL_HEADER).contains(&y)
                        && (PAD..=W - PAD).contains(&x)
                    {
                        app.expanded = !app.expanded;
                        _ = app.render();
                    }
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
        _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-notifications");
        RegisterClassW(&WNDCLASSW {
            // Popup shadow, as the real flyout presenter has.
            style: windows::Win32::UI::WindowsAndMessaging::CS_DROPSHADOW,
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        // Height from the work area, so the panel fills the screen like the
        // real notification centre at any resolution or DPI.
        let work = metrics::work_rect();
        let k = metrics::scale();
        let height = ((work.bottom - work.top) as f32 / k).round() as i32 - 16;
        let (x, y) = metrics::bottom_right(W + 2.0 * SHADOW, height as f32 + 2.0 * SHADOW, 0.0);
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class,
            w!("fluent-win · Notificações"), WS_POPUP | WS_VISIBLE,
            x, y, ((W + 2.0 * SHADOW) * k).round() as i32,
            ((height as f32 + 2.0 * SHADOW) * k).round() as i32,
            None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let centered = {
            let format = surface.format(&theme::CAPTION)?;
            format.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            format
        };
        let today = calendar::today();
        let cells = calendar::grid_full(today.year, today.month);
        APP.with(|cell| *cell.borrow_mut() = Some(App {
            surface, theme, formats, centered, today, cells, expanded: false,
        }));
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        Ok(())
    }
}
