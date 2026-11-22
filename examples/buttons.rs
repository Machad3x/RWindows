//! WinUI buttons on Mica: default, accent and disabled. Tab / Shift+Tab move
//! keyboard focus (focus ring), Space / Enter invoke, the mouse captures while
//! pressed. Idle = blocked in GetMessageW; frames render only during transitions.
//! UI Automation: the buttons appear to Narrator / NVDA / Inspect as Button
//! elements with Invoke, through `fluent_win::uia`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use fluent_win::backdrop;
use fluent_win::button::{Button, FocusKind, Input, Key, Response};
use fluent_win::gfx::{Surface, rect};
use fluent_win::theme::{self, Theme};
use fluent_win::uia::{self, WM_UIA_FOCUS, WM_UIA_INVOKE};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::DirectWrite::IDWriteTextFormat;
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, UIA_AutomationFocusChangedEventId, UIA_EVENT_ID, UIA_Invoke_InvokedEventId,
    UiaClientsAreListening, UiaRaiseAutomationEvent, UiaReturnRawElementProvider, UiaRootObjectId,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VIRTUAL_KEY, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SIZE_MINIMIZED,
    SPI_SETHIGHCONTRAST, WM_SYSCOLORCHANGE,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_CAPTURECHANGED, WM_DESTROY,
    WM_DPICHANGED, WM_GETOBJECT,
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
    uia: uia::Shared,
    /// UIA events to raise once the APP borrow ends (see `with_app`).
    pending: Vec<(usize, UIA_EVENT_ID)>,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    /// Kept outside APP: WM_GETOBJECT can arrive while APP is borrowed.
    static UIA: RefCell<Option<uia::Shared>> = const { RefCell::new(None) };
}

impl App {
    fn render(&mut self) -> Result<()> {
        self.sync_uia();
        let (s, t) = (&mut self.surface, &self.theme);
        s.begin();
        if let Some(c) = t.high_contrast {
            // HighContrast SolidBackgroundFillColorBase = Window: opaque page, no Mica.
            let (w, h) = s.size();
            s.fill_round(rect(0.0, 0.0, w, h), 0.0, c.window);
        }
        for b in &self.buttons {
            b.draw(s, t, &self.label)?;
        }
        s.text(&self.status, &self.body, rect(PAD, 96.0, PAD + 500.0, 120.0), t.text_secondary);
        s.end()
    }

    /// Copies what UI Automation exposes into the shared model.
    fn sync_uia(&self) {
        let mut m = self.uia.borrow_mut();
        m.hwnd = self.hwnd;
        m.scale = self.surface.scale();
        m.focused = self.focused;
        m.items = self.buttons.iter().map(|b| uia::Item {
            name: b.label.clone(),
            rect: b.rect,
            enabled: b.visual() != fluent_win::button::Visual::Disabled,
            toggle: b.is_checked(),
        }).collect();
    }

    /// Queues a UIA event on button `i`; `with_app` raises it after the APP borrow
    /// ends, since raising may pump STA messages that would otherwise be dropped.
    fn raise(&mut self, i: usize, event: UIA_EVENT_ID) {
        if !unsafe { UiaClientsAreListening() }.as_bool() {
            return;
        }
        self.sync_uia();
        self.pending.push((i, event));
    }

    /// Button `i` was invoked (pointer, keyboard or UI Automation).
    fn clicked(&mut self, i: usize) {
        self.status = format!("Clicou: {}", self.buttons[i].label);
        self.raise(i, UIA_Invoke_InvokedEventId);
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
            self.clicked(i);
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
        let changed = self.focused != to;
        if let Some(new) = to {
            redraw |= self.buttons[new].handle(Input::Focus(kind)).redraw;
        }
        self.focused = to;
        if let Some(new) = to.filter(|_| changed) {
            self.raise(new, UIA_AutomationFocusChangedEventId);
        }
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

/// Runs `f` on the app, then raises the UIA events it queued outside the borrow.
/// Returns false only when the app is already borrowed (a re-entrant message).
fn try_with_app(f: impl FnOnce(&mut App) -> Result<()>) -> bool {
    let pending = APP.with(|cell| {
        let Ok(mut guard) = cell.try_borrow_mut() else { return None };
        let Some(app) = guard.as_mut() else { return Some(Vec::new()) };
        if let Err(e) = f(app) {
            eprintln!("fluent-win buttons: {e}");
        }
        Some(std::mem::take(&mut app.pending))
    });
    let Some(pending) = pending else { return false };
    if let Some(model) = UIA.with(|u| u.borrow().clone()) {
        for (i, event) in pending {
            if let Err(e) = unsafe { UiaRaiseAutomationEvent(&uia::item(&model, i), event) } {
                eprintln!("fluent-win buttons: UIA event: {e}");
            }
        }
    }
    true
}

/// [`try_with_app`] for messages that may be skipped when the app is busy.
fn with_app(f: impl FnOnce(&mut App) -> Result<()>) {
    try_with_app(f);
}

/// Re-posts a WM_UIA_* message that arrived while the app was busy, so it is not lost.
fn with_app_or_repost(hwnd: HWND, msg: u32, wp: WPARAM, f: impl FnOnce(&mut App) -> Result<()>) {
    if !try_with_app(f) {
        unsafe { _ = PostMessageW(Some(hwnd), msg, wp, LPARAM(0)) };
    }
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
        // Theme or accent changed (ImmersiveColorSet), high contrast toggled
        // (SPI_SETHIGHCONTRAST), or system colors changed (WM_SYSCOLORCHANGE, Learn:
        // "High contrast parameter").
        WM_SYSCOLORCHANGE | WM_SETTINGCHANGE
            if msg == WM_SYSCOLORCHANGE
                || wp.0 == SPI_SETHIGHCONTRAST.0 as usize
                || lp.0 != 0
                    && unsafe { PCWSTR(lp.0 as *const u16).to_string() }
                        .is_ok_and(|s| s == "ImmersiveColorSet") =>
        {
            with_app(|a| {
                a.theme = Theme::system();
                backdrop::set_dark(a.hwnd, a.theme.dark)?;
                a.render()
            });
        }
        WM_GETOBJECT if lp.0 as i32 == UiaRootObjectId => {
            let root = UIA.with(|u| u.borrow().as_ref().map(uia::root));
            if let Some(root) = root {
                return unsafe { UiaReturnRawElementProvider(hwnd, wp, lp, &root) };
            }
            return unsafe { DefWindowProcW(hwnd, msg, wp, lp) };
        }
        // Posted by IInvokeProvider::Invoke: run the click outside the UIA call.
        WM_UIA_INVOKE => with_app_or_repost(hwnd, msg, wp, |a| {
            let i = wp.0;
            if a.buttons.get(i).is_some_and(|b| b.visual() != fluent_win::button::Visual::Disabled) {
                a.clicked(i);
                a.render()?;
            }
            Ok(())
        }),
        // Posted by IRawElementProviderFragment::SetFocus (UIA already focused the window).
        WM_UIA_FOCUS => with_app_or_repost(hwnd, msg, wp, |a| {
            // Only when the window really has keyboard focus; otherwise keys go elsewhere
            // and no WM_KILLFOCUS would ever clear the button's focus.
            if unsafe { GetFocus() } != hwnd {
                return Ok(());
            }
            if wp.0 < a.buttons.len() && a.move_focus(Some(wp.0), FocusKind::Keyboard)? {
                a.render()?;
            }
            Ok(())
        }),
        WM_DESTROY => unsafe {
            // Providers a client still holds now answer UIA_E_ELEMENTNOTAVAILABLE.
            UIA.with(|u| {
                if let Some(m) = u.borrow().as_ref() {
                    let mut m = m.borrow_mut();
                    m.items.clear();
                    m.focused = None;
                }
            });
            // Releases the provider map UIA keeps for MSAA clients (Learn: UiaReturnRawElementProvider).
            UiaReturnRawElementProvider(hwnd, WPARAM(0), LPARAM(0), None::<&IRawElementProviderSimple>);
            PostQuitMessage(0);
        },
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
}

/// Balances `CoInitializeEx` with `CoUninitialize`.
struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

fn main() -> Result<()> {
    unsafe {
        // STA: UIA providers use ProviderOptions_UseComThreading and are called on this thread.
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        // Declared first so it drops last, on every return path.
        let _com = ComApartment;
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
            uia: Rc::new(RefCell::new(uia::Model::default())),
            pending: Vec::new(),
        };
        UIA.with(|u| *u.borrow_mut() = Some(app.uia.clone()));
        app.render()?;
        APP.with(|cell| *cell.borrow_mut() = Some(app));

        let result = message_loop();
        // Release GPU objects while the process is still running: dropped later, in
        // thread-local teardown, they turned the exit code into a DXGI HRESULT (0x87A).
        APP.with(|cell| drop(cell.borrow_mut().take()));
        result
    }
}

/// Pumps messages until WM_QUIT; renders frames only while a transition runs.
fn message_loop() -> Result<()> {
    unsafe {
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
