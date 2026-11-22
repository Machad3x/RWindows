//! WinUI RadioButton, ProgressBar and ProgressRing on Mica.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::button::Input;
use fluent_win::gfx::{Surface, rect};
use fluent_win::panel::Formats;
use fluent_win::progress::{ProgressBar, ProgressRing};
use fluent_win::radio_button::{MIN_WIDTH, ROW_HEIGHT, RadioButton, check_one};
use fluent_win::theme::Theme;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    KillTimer, LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SetTimer, TranslateMessage,
    WM_DESTROY, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{Result, w};

const PAD: f32 = 32.0;
const GAP: f32 = 12.0;
const TIMER: usize = 1;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    surface: Surface,
    theme: Theme,
    formats: Formats,
    radios: Vec<RadioButton>,
    bar: ProgressBar,
    ring: ProgressRing,
    time: f32,
}

fn loword(v: isize) -> i16 {
    (v & 0xffff) as i16
}

fn hiword(v: isize) -> i16 {
    ((v >> 16) & 0xffff) as i16
}

fn with_app(f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win progress: {e}");
        }
    });
    LRESULT(0)
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (s, t) = (&mut self.surface, &self.theme);
        s.begin();
        if let Some(c) = t.high_contrast {
            let (w, h) = s.size();
            s.fill_round(rect(0.0, 0.0, w, h), 0.0, c.window);
        }
        for (i, radio) in self.radios.iter().enumerate() {
            let y = PAD + i as f32 * (ROW_HEIGHT + GAP);
            radio.draw(s, t, &self.formats, rect(PAD, y, PAD + MIN_WIDTH, y + ROW_HEIGHT));
        }
        let y = PAD + 3.0 * (ROW_HEIGHT + GAP) + GAP;
        self.bar.draw(s, t, rect(PAD, y, PAD + 240.0, y + 3.0));
        self.ring.draw(s, t, self.time);
        s.end()
    }

    fn input(&mut self, x: f32, y: f32, down: bool) -> Result<()> {
        if !down {
            let mut clicked = None;
            for (i, radio) in self.radios.iter_mut().enumerate() {
                if radio.handle(Input::PointerUp(x, y)) {
                    clicked = Some(i);
                }
            }
            if let Some(i) = clicked {
                check_one(&mut self.radios, i);
            }
            return self.render();
        }
        let mut target = None;
        for (i, radio) in self.radios.iter_mut().enumerate() {
            let top = PAD + i as f32 * (ROW_HEIGHT + GAP);
            let local_x = x - PAD;
            let local_y = y - top;
            radio.handle(Input::PointerDown(local_x, local_y));
            if (0.0..MIN_WIDTH).contains(&local_x) && (0.0..ROW_HEIGHT).contains(&local_y) {
                target = Some(i);
            }
        }
        if let Some(i) = target {
            check_one(&mut self.radios, i);
        }
        self.render()
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::render)
        }
        WM_LBUTTONDOWN => with_app(|a| {
            let k = a.surface.scale();
            a.input(f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k, true)
        }),
        WM_LBUTTONUP => with_app(|a| {
            let k = a.surface.scale();
            a.input(f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k, false)
        }),
        WM_TIMER if wp.0 == TIMER => with_app(|a| {
            a.time += 1.0 / 30.0;
            a.render()
        }),
        WM_DESTROY => unsafe {
            _ = KillTimer(Some(hwnd), TIMER);
            PostQuitMessage(0);
            LRESULT(0)
        },
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-progress");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Progress"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            (400.0 * k) as i32, (320.0 * k) as i32, None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let radios = vec![
            RadioButton::new("Claro", true),
            RadioButton::new("Escuro", false),
            RadioButton::new("Sistema", false),
        ];
        let bar = ProgressBar::new(60.0, 100.0);
        let ring = ProgressRing { center: (300.0, 200.0), radius: 24.0, value: None };
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { surface, theme, formats, radios, bar, ring, time: 0.0 })
        });
        with_app(App::render);
        _ = SetTimer(Some(hwnd), TIMER, 33, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|cell| drop(cell.borrow_mut().take()));
        Ok(())
    }
}
