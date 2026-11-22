//! WinUI `ListView` on Mica: virtualized rows, selection, hover, wheel and
//! keyboard scrolling. Mirrors `DefaultListViewItemStyle` from WinUI XAML.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::button::{FocusKind, Input, Key};
use fluent_win::gfx::{Surface, rect};
use fluent_win::list_view::{ITEM_HEIGHT, ListView};
use fluent_win::panel::Formats;
use fluent_win::theme::Theme;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END,
    VK_HOME, VK_UP, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PostQuitMessage, RegisterClassW, SIZE_MINIMIZED,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_DESTROY, WM_DPICHANGED,
    WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_PAINT,
    WM_SETFOCUS, WM_SIZE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{Result, w};

const PAD: f32 = 16.0;
const LIST_WIDTH: f32 = 320.0;
/// Rows end at PAD + LIST_HEIGHT = 200; the status line starts 16 DIPs below.
const LIST_HEIGHT: f32 = 184.0;
const STATUS_TOP: f32 = 216.0;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    formats: Formats,
    list: ListView,
    tracking: bool,
}

fn loword(v: isize) -> i16 {
    (v & 0xffff) as i16
}

fn hiword(v: isize) -> i16 {
    ((v >> 16) & 0xffff) as i16
}

fn point(a: &App, lp: LPARAM) -> (f32, f32) {
    let k = a.surface.scale();
    (f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k)
}

fn with_app(f: impl FnOnce(&mut App) -> Result<()>) -> LRESULT {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win list: {e}");
        }
    });
    LRESULT(0)
}

impl App {
    fn handle(&mut self, input: Input) -> Result<()> {
        let out = self.list.handle(input, PAD, PAD, LIST_WIDTH);
        match out.capture {
            Some(true) => unsafe { _ = SetCapture(self.hwnd) },
            Some(false) => unsafe { ReleaseCapture()? },
            None => {}
        }
        if out.redraw {
            self.render()?;
        }
        Ok(())
    }

    fn key(&mut self, vk: VIRTUAL_KEY, down: bool) -> Result<()> {
        if !down {
            return Ok(());
        }
        let key = match vk {
            VK_UP => Key::Up,
            VK_DOWN => Key::Down,
            VK_HOME => Key::Home,
            VK_END => Key::End,
            _ => Key::Other,
        };
        self.handle(Input::KeyDown(key))
    }

    fn render(&mut self) -> Result<()> {
        let (s, t) = (&mut self.surface, &self.theme);
        s.begin();
        if let Some(c) = t.high_contrast {
            let (w, h) = s.size();
            s.fill_round(rect(0.0, 0.0, w, h), 0.0, c.window);
        }
        self.list.draw(s, t, &self.formats, PAD, PAD, LIST_WIDTH)?;
        let selected = self.list.selected
            .and_then(|i| self.list.item(i))
            .unwrap_or("nenhum");
        s.text(&format!("Selecionado: {selected}"), &self.formats.body,
            rect(PAD, STATUS_TOP, PAD + 400.0, STATUS_TOP + 24.0), t.text_secondary);
        s.end()
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::render)
        }
        WM_SIZE if wp.0 as u32 == SIZE_MINIMIZED => LRESULT(0),
        WM_SIZE => with_app(|a| {
            let w = loword(lp.0) as u16 as f32;
            let h = hiword(lp.0) as u16 as f32;
            a.surface.resize(w as u32, h as u32)?;
            let k = a.surface.scale();
            a.list.set_viewport(LIST_HEIGHT.min(h / k - PAD).max(0.0));
            a.render()
        }),
        WM_MOUSEMOVE => with_app(|a| {
            if !a.tracking {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                unsafe { TrackMouseEvent(&mut tme)? };
                a.tracking = true;
            }
            let (x, y) = point(a, lp);
            a.handle(Input::PointerMove(x, y))
        }),
        WM_MOUSELEAVE => with_app(|a| {
            a.tracking = false;
            a.handle(Input::PointerLeave)
        }),
        WM_LBUTTONDOWN => with_app(|a| {
            let (x, y) = point(a, lp);
            a.handle(Input::PointerDown(x, y))
        }),
        WM_LBUTTONUP => with_app(|a| {
            let (x, y) = point(a, lp);
            a.handle(Input::PointerUp(x, y))
        }),
        WM_MOUSEWHEEL => with_app(|a| {
            // Three rows per notch (ScrollViewer's default wheel scroll amount).
            let notches = hiword(wp.0 as isize) as i32 / 120;
            a.list.scroll_by(-notches as f32 * ITEM_HEIGHT * 3.0);
            a.render()
        }),
        WM_SETFOCUS => with_app(|a| a.handle(Input::Focus(FocusKind::Keyboard))),
        WM_KEYDOWN => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), true)),
        WM_KILLFOCUS => with_app(|a| a.handle(Input::Blur)),
        WM_DPICHANGED => {
            with_app(|a| a.surface.set_dpi(f32::from(loword(wp.0 as isize) as u16)));
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE);
            }
            with_app(|a| {
                let (_, h) = a.surface.size();
                a.list.set_viewport(LIST_HEIGHT.min(h - PAD).max(0.0));
                a.render()
            })
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
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-list");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · ListView"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            (400.0 * k) as i32, (320.0 * k) as i32, None, None, Some(instance.into()), None)?;
        let theme = Theme::system();
        _ = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark);
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let mut list = ListView::new(
            (0..500).map(|i| format!("Aplicativo {i}")).collect(),
        );
        list.set_viewport(LIST_HEIGHT);
        APP.with(|cell| {
            *cell.borrow_mut() = Some(App { hwnd, surface, theme, formats, list, tracking: false })
        });
        with_app(App::render);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        APP.with(|cell| drop(cell.borrow_mut().take()));
        Ok(())
    }
}
