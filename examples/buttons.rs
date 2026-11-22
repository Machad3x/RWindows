//! WinUI buttons on Mica: default, accent and disabled. Tab / Shift+Tab move
//! keyboard focus (focus ring), Space / Enter invoke, the mouse captures while
//! pressed. Idle = blocked in GetMessageW; frames render only during transitions.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;

use fluent_win::backdrop;
use fluent_win::button::{Button, FocusKind, Input, Key, Response};
use fluent_win::gfx::{Surface, rect};
use fluent_win::theme::{self, Theme};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VIRTUAL_KEY, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostQuitMessage, RegisterClassW, SIZE_MINIMIZED,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_CAPTURECHANGED, WM_DESTROY,
    WM_DPICHANGED,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT,
    WM_QUIT, WM_SETTINGCHANGE, WM_SIZE, WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};
use windows::core::{PCWSTR, Result, w};

const PAD: f32 = 32.0;
const GAP: f32 = 8.0;

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    label: IDWriteTextFormat,
    body: IDWriteTextFormat,
    buttons: Vec<Button>,
    focused: Option<usize>,
    status: String,
    tracking: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

impl App {
    fn render(&mut self) -> Result<()> {
        let (s, t) = (&mut self.surface, &self.theme);
        s.begin();
        for b in &self.buttons {
            b.draw(s, t, &self.label)?;
        }
        s.text(&self.status, &self.body, rect(PAD, 96.0, PAD + 500.0, 120.0), t.text_secondary);
        s.end()
    }

    fn animating(&self) -> bool {
        self.buttons.iter().any(Button::animating)
    }

    /// Sends `input` to the buttons and applies capture, focus and click effects.
    /// While one button holds the capture, pointer input goes to it alone (as XAML
    /// routes captured pointer events), so dragging across others does not light them.
    fn dispatch(&mut self, input: Input) -> Result<()> {
        let pointer = matches!(input, Input::PointerMove(..) | Input::PointerUp(..) | Input::PointerLeave);
        let captor = self.buttons.iter().position(Button::is_captured).filter(|_| pointer);
        let mut redraw = false;
        for i in 0..self.buttons.len() {
            if captor.is_some_and(|c| c != i) {
                continue;
            }
            let r = self.buttons[i].handle(input);
            redraw |= self.apply(i, r)?;
        }
        if redraw {
            self.render()?;
        }
        Ok(())
    }

    fn capture(&self, request: Option<bool>) -> Result<()> {
        match request {
            Some(true) => unsafe { _ = SetCapture(self.hwnd) },
            // Our own WM_CAPTURECHANGED re-enters with_app and is skipped by the borrow check.
            Some(false) => unsafe { ReleaseCapture()? },
            None => {}
        }
        Ok(())
    }

    fn apply(&mut self, i: usize, r: Response) -> Result<bool> {
        self.capture(r.capture)?;
        let mut redraw = r.redraw;
        if r.take_focus {
            redraw |= self.move_focus(Some(i), FocusKind::Pointer)?;
        }
        if r.lost_focus && self.focused == Some(i) {
            self.focused = None;
        }
        if r.clicked {
            self.status = format!("Clicou: {}", self.buttons[i].label);
            redraw = true;
        }
        Ok(redraw)
    }

    /// Moves focus to `to`, blurring the previous button (which releases a capture
    /// it held: Tab while the mouse is down). Returns true if anything changed.
    fn move_focus(&mut self, to: Option<usize>, kind: FocusKind) -> Result<bool> {
        let mut redraw = false;
        if let Some(old) = self.focused.filter(|&old| Some(old) != to) {
            let blur = self.buttons[old].handle(Input::Blur);
            self.capture(blur.capture)?;
            redraw |= blur.redraw;
        }
        if let Some(new) = to {
            redraw |= self.buttons[new].handle(Input::Focus(kind)).redraw;
        }
        self.focused = to;
        Ok(redraw)
    }

    /// Tab order over enabled buttons (disabled ones are skipped, like IsTabStop on a disabled control).
    fn tab(&mut self, back: bool) -> Result<()> {
        let n = self.buttons.len();
        let start = self.focused.unwrap_or(if back { 0 } else { n - 1 });
        let next = (1..=n)
            .map(|k| if back { (start + n - k) % n } else { (start + k) % n })
            .find(|&i| self.buttons[i].visual() != fluent_win::button::Visual::Disabled);
        if self.move_focus(next, FocusKind::Keyboard)? {
            self.render()?;
        }
        Ok(())
    }

    fn key(&mut self, vk: VIRTUAL_KEY, down: bool) -> Result<()> {
        if down && vk == VK_TAB {
            let back = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
            return self.tab(back);
        }
        let Some(i) = self.focused else { return Ok(()) };
        let key = match vk {
            VK_SPACE => Key::Space,
            VK_RETURN => Key::Enter,
            _ => Key::Other,
        };
        let r = self.buttons[i].handle(if down { Input::KeyDown(key) } else { Input::KeyUp(key) });
        if self.apply(i, r)? {
            self.render()?;
        }
        Ok(())
    }
}

/// Runs `f` on the app unless it is missing or already borrowed (re-entrant message).
fn with_app(f: impl FnOnce(&mut App) -> Result<()>) {
    APP.with(|cell| {
        if let Ok(mut guard) = cell.try_borrow_mut()
            && let Some(app) = guard.as_mut()
            && let Err(e) = f(app)
        {
            eprintln!("fluent-win buttons: {e}");
        }
    });
}

fn loword(v: isize) -> i16 {
    (v & 0xffff) as i16
}

fn hiword(v: isize) -> i16 {
    ((v >> 16) & 0xffff) as i16
}

/// Pointer position in DIPs.
fn point(a: &App, lp: LPARAM) -> (f32, f32) {
    let k = a.surface.scale();
    (f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k)
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::render);
        }
        WM_SIZE if wp.0 as u32 == SIZE_MINIMIZED => {}
        WM_SIZE => with_app(|a| {
            a.surface.resize(loword(lp.0) as u16 as u32, hiword(lp.0) as u16 as u32)?;
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
            a.dispatch(Input::PointerMove(x, y))
        }),
        WM_MOUSELEAVE => with_app(|a| {
            a.tracking = false;
            a.dispatch(Input::PointerLeave)
        }),
        WM_LBUTTONDOWN => with_app(|a| {
            let (x, y) = point(a, lp);
            a.dispatch(Input::PointerDown(x, y))
        }),
        WM_LBUTTONUP => with_app(|a| {
            let (x, y) = point(a, lp);
            a.dispatch(Input::PointerUp(x, y))
        }),
        WM_KEYDOWN => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), true)),
        WM_KEYUP => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), false)),
        WM_KILLFOCUS => with_app(|a| {
            // Window lost focus: the focused button loses it too (OnLostFocus).
            if a.move_focus(None, FocusKind::Keyboard)? {
                a.render()?;
            }
            Ok(())
        }),
        // Another window took the capture: abandon any press (OnPointerCaptureLost).
        WM_CAPTURECHANGED if lp.0 != hwnd.0 as isize => with_app(|a| a.dispatch(Input::CaptureLost)),
        WM_DPICHANGED => {
            with_app(|a| a.surface.set_dpi(f32::from(loword(wp.0 as isize) as u16)));
            // Outside with_app: SetWindowPos sends WM_SIZE synchronously.
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        WM_SETTINGCHANGE
            if lp.0 != 0
                && unsafe { PCWSTR(lp.0 as *const u16).to_string() }
                    .is_ok_and(|s| s == "ImmersiveColorSet") =>
        {
            with_app(|a| {
                a.theme = Theme::system();
                backdrop::set_dark(a.hwnd, a.theme.dark)?;
                a.render()
            });
        }
        WM_DESTROY => unsafe { PostQuitMessage(0) },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

fn main() -> Result<()> {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)?;
        let instance = GetModuleHandleW(None)?;
        let class = w!("fluent-win-buttons");
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: class,
            lpfnWndProc: Some(wndproc),
            ..Default::default()
        });
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Button"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            (520.0 * k) as i32, (200.0 * k) as i32, None, None, Some(instance.into()), None)?;

        let theme = Theme::system();
        if let Err(e) = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark) {
            eprintln!("fluent-win buttons: backdrop unavailable: {e}");
        }
        let surface = Surface::new(hwnd)?;
        let label = Button::text_format(&surface)?;
        let body = surface.format(&theme::BODY)?;

        // Auto-size each button from its label, left to right.
        let mut x = PAD;
        let mut buttons = Vec::new();
        for (text, accent, enabled) in [("Configurações", false, true), ("Salvar", true, true),
            ("Desabilitado", false, false), ("Accent off", true, false)]
        {
            let (w, h) = Button::desired_size(&surface, &label, text)?;
            let mut b = Button::new(text, rect(x, PAD, x + w, PAD + h), accent);
            b.set_enabled(enabled);
            x += w + GAP;
            buttons.push(b);
        }

        let mut app = App {
            hwnd, surface, theme, label, body, buttons,
            focused: None,
            status: "Tab para focar, Espaço/Enter para clicar".to_owned(),
            tracking: false,
        };
        app.render()?;
        APP.with(|cell| *cell.borrow_mut() = Some(app));

        let mut msg = MSG::default();
        let mut was_animating = false;
        loop {
            let mut animating = false;
            with_app(|a| {
                animating = a.animating();
                Ok(())
            });
            if animating || was_animating {
                // Last frame after a transition lands exactly on its target color.
                with_app(App::render);
            }
            was_animating = animating;
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return Ok(());
                    }
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    return Ok(());
                }
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
