//! WinUI controls on Mica: buttons (default, accent, disabled, toggle, subtle),
//! a ToggleSwitch and CheckBoxes, routed by `fluent_win::panel::Panel`.
//! Tab / Shift+Tab move keyboard focus (focus ring), Space / Enter invoke, the
//! mouse captures while pressed. Idle = blocked in GetMessageW; frames render
//! only during transitions. UI Automation: the controls appear to Narrator /
//! NVDA / Inspect through `fluent_win::uia`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;

use fluent_win::backdrop;
use fluent_win::button::{Button, FocusKind, Input, Key};
use fluent_win::check_box::{CheckBox, CheckState};
use fluent_win::gfx::{Surface, rect};
use fluent_win::panel::{Control, Event, Formats, Outcome, Panel};
use fluent_win::theme::Theme;
use fluent_win::toggle_switch::ToggleSwitch;
use fluent_win::uia::{self, WM_UIA_FOCUS, WM_UIA_INVOKE};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::ValidateRect;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, ToggleState, ToggleState_Indeterminate, ToggleState_On,
    UIA_AutomationFocusChangedEventId, UIA_BoundingRectanglePropertyId, UIA_EVENT_ID, UIA_Invoke_InvokedEventId,
    UIA_ToggleToggleStatePropertyId, UiaClientsAreListening, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaReturnRawElementProvider, UiaRootObjectId,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForSystem, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VIRTUAL_KEY, VK_ADD, VK_RETURN, VK_SHIFT, VK_SPACE, VK_SUBTRACT, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SIZE_MINIMIZED,
    SPI_SETHIGHCONTRAST, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_CAPTURECHANGED,
    WM_DESTROY, WM_DPICHANGED, WM_GETOBJECT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_QUIT, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE,
    WNDCLASSW, WS_EX_NOREDIRECTIONBITMAP, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};
use windows::core::{PCWSTR, Result, w};

const PAD: f32 = 32.0;
const GAP: f32 = 8.0;

struct App {
    hwnd: HWND,
    surface: Surface,
    theme: Theme,
    formats: Formats,
    panel: Panel,
    status: String,
    tracking: bool,
    uia: uia::Shared,
    /// UIA events to raise once the APP borrow ends (see `with_app`).
    pending: Vec<(usize, UiaEvent)>,
}

/// A UIA notification queued until the APP borrow ends.
#[derive(Clone, Copy)]
enum UiaEvent {
    /// `UiaRaiseAutomationEvent`.
    Event(UIA_EVENT_ID),
    /// `ToggleState` property changed from the first value to the second.
    Toggled(ToggleState, ToggleState),
    /// `BoundingRectangle` changed (DPI change moved the control in screen pixels).
    Bounds,
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
        self.panel.draw(s, t, &self.formats)?;
        s.text(&self.status, &self.formats.body, rect(PAD, 200.0, PAD + 500.0, 224.0), t.text_secondary);
        s.end()
    }

    /// Copies what UI Automation exposes into the shared model.
    fn sync_uia(&self) {
        let mut m = self.uia.borrow_mut();
        m.hwnd = self.hwnd;
        m.scale = self.surface.scale();
        m.focused = self.panel.focused();
        m.items = self.panel.uia_items();
    }

    /// Queues a UIA event on control `i`; `with_app` raises it after the APP borrow
    /// ends, since raising may pump STA messages that would otherwise be dropped.
    fn raise(&mut self, i: usize, event: UiaEvent) {
        if !unsafe { UiaClientsAreListening() }.as_bool() {
            return;
        }
        self.sync_uia();
        self.pending.push((i, event));
    }

    /// Applies a panel outcome: capture, status line, UIA events, repaint.
    fn apply(&mut self, out: Outcome) -> Result<()> {
        match out.capture {
            Some(true) => unsafe { _ = SetCapture(self.hwnd) },
            // Our own WM_CAPTURECHANGED re-enters with_app and is skipped by the borrow check.
            Some(false) => unsafe { ReleaseCapture()? },
            None => {}
        }
        for event in &out.events {
            match *event {
                Event::Invoked(i) => {
                    self.status = format!("Clicou: {}", self.panel.controls[i].name());
                    self.raise(i, UiaEvent::Event(UIA_Invoke_InvokedEventId));
                }
                // Toggle controls raise ToggleState changed, not Invoked (their peers have no Invoke).
                Event::Toggled { index, old, new } => {
                    let state = if new == ToggleState_On {
                        "ligado"
                    } else if new == ToggleState_Indeterminate {
                        "indeterminado"
                    } else {
                        "desligado"
                    };
                    self.status = format!("{}: {state}", self.panel.controls[index].name());
                    self.raise(index, UiaEvent::Toggled(old, new));
                }
                Event::Focused(i) => self.raise(i, UiaEvent::Event(UIA_AutomationFocusChangedEventId)),
            }
        }
        if out.redraw {
            self.render()?;
        }
        Ok(())
    }

    fn input(&mut self, input: Input) -> Result<()> {
        let out = self.panel.handle(input);
        self.apply(out)
    }

    fn key(&mut self, vk: VIRTUAL_KEY, down: bool) -> Result<()> {
        if down && vk == VK_TAB {
            let back = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
            let out = self.panel.tab(back);
            return self.apply(out);
        }
        let key = match vk {
            VK_SPACE => Key::Space,
            VK_RETURN => Key::Enter,
            VK_ADD => Key::Add,
            VK_SUBTRACT => Key::Subtract,
            _ => Key::Other,
        };
        let out = self.panel.key(key, down);
        self.apply(out)
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
            let provider = uia::item(&model, i);
            let raised = match event {
                UiaEvent::Event(id) => unsafe { UiaRaiseAutomationEvent(&provider, id) },
                UiaEvent::Toggled(old, new) => unsafe {
                    let (old, new) = (VARIANT::from(old.0), VARIANT::from(new.0));
                    UiaRaiseAutomationPropertyChangedEvent(&provider, UIA_ToggleToggleStatePropertyId, &old, &new)
                },
                // The old pixel rectangle is not tracked: VT_EMPTY.
                UiaEvent::Bounds => uia::bounds_variant(&model, i).and_then(|new| unsafe {
                    UiaRaiseAutomationPropertyChangedEvent(&provider, UIA_BoundingRectanglePropertyId, &VARIANT::default(), &new)
                }),
            };
            if let Err(e) = raised {
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
            a.input(Input::PointerMove(x, y))
        }),
        WM_MOUSELEAVE => with_app(|a| {
            a.tracking = false;
            a.input(Input::PointerLeave)
        }),
        WM_LBUTTONDOWN => with_app(|a| {
            let (x, y) = point(a, lp);
            a.input(Input::PointerDown(x, y))
        }),
        WM_LBUTTONUP => with_app(|a| {
            let (x, y) = point(a, lp);
            a.input(Input::PointerUp(x, y))
        }),
        WM_KEYDOWN => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), true)),
        WM_KEYUP => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), false)),
        // Window lost focus: the focused control loses it too (OnLostFocus).
        WM_KILLFOCUS => with_app(|a| {
            let out = a.panel.focus(None, FocusKind::Keyboard);
            a.apply(out)
        }),
        // Another window took the capture: abandon any press (OnPointerCaptureLost).
        WM_CAPTURECHANGED if lp.0 != hwnd.0 as isize => with_app(|a| a.input(Input::CaptureLost)),
        WM_DPICHANGED => {
            with_app(|a| a.surface.set_dpi(f32::from(loword(wp.0 as isize) as u16)));
            // Outside with_app: SetWindowPos sends WM_SIZE synchronously.
            let r = unsafe { &*(lp.0 as *const RECT) };
            unsafe {
                _ = SetWindowPos(hwnd, None, r.left, r.top, r.right - r.left, r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE);
            }
            // Every control now has a new BoundingRectangle in screen pixels.
            with_app(|a| {
                (0..a.panel.controls.len()).for_each(|i| a.raise(i, UiaEvent::Bounds));
                Ok(())
            });
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
        // Posted by IInvokeProvider::Invoke / IToggleProvider::Toggle: run the click
        // outside the UIA call (controls ignore it while disabled).
        WM_UIA_INVOKE => with_app_or_repost(hwnd, msg, wp, |a| {
            let out = a.panel.invoke(wp.0);
            a.apply(out)
        }),
        // Posted by IRawElementProviderFragment::SetFocus (UIA already focused the window).
        WM_UIA_FOCUS => with_app_or_repost(hwnd, msg, wp, |a| {
            // Only when the window really has keyboard focus; otherwise keys go elsewhere
            // and no WM_KILLFOCUS would ever clear the control's focus.
            if unsafe { GetFocus() } != hwnd {
                return Ok(());
            }
            let out = a.panel.focus(Some(wp.0), FocusKind::Keyboard);
            a.apply(out)
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

/// The demo's controls, laid out by hand in three rows.
fn controls(surface: &Surface, f: &Formats) -> Result<Vec<Box<dyn Control>>> {
    let mut controls: Vec<Box<dyn Control>> = Vec::new();
    // First row: auto-size each button from its label, left to right.
    let mut x = PAD;
    for (text, accent, enabled) in [("Configurações", false, true), ("Salvar", true, true),
        ("Desabilitado", false, false), ("Accent off", true, false)]
    {
        let (w, h) = Button::desired_size(surface, &f.label, text)?;
        let mut b = Button::new(text, rect(x, PAD, x + w, PAD + h), accent);
        b.set_enabled(enabled);
        x += w + GAP;
        controls.push(Box::new(b));
    }
    // Second row: a ToggleButton (Quick Settings tile) and a SubtleButton (taskbar).
    let (w, h) = Button::desired_size(surface, &f.label, "Wi-Fi")?;
    let top = PAD + h + GAP * 2.0;
    controls.push(Box::new(Button::toggle("Wi-Fi", rect(PAD, top, PAD + w, top + h), true)));
    let x = PAD + w + GAP;
    let (w, h) = Button::desired_size(surface, &f.label, "Iniciar")?;
    controls.push(Box::new(Button::subtle("Iniciar", rect(x, top, x + w, top + h))));
    // Third row: a ToggleSwitch with header (Settings style) and two CheckBoxes
    // centered on the switch row, the second one three-state.
    let top = top + h + GAP;
    let switch = ToggleSwitch::layout(surface, &f.switch, "Modo avião", "Ativado", "Desativado", PAD, top, false)?;
    let (mut x, cy) = (switch.rect.right + GAP * 3.0, switch.rect.bottom - 20.0);
    controls.push(Box::new(switch));
    for (text, state) in [("Lembrar de mim", None), ("Selecionar tudo", Some(CheckState::Indeterminate))] {
        let (cw, ch) = CheckBox::desired_size(surface, &f.body, text)?;
        let r = rect(x, cy - ch / 2.0, x + cw, cy + ch / 2.0);
        controls.push(Box::new(match state {
            None => CheckBox::new(text, r, true),
            Some(state) => CheckBox::three_state(text, r, state),
        }));
        x += cw + GAP;
    }
    Ok(controls)
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
            (520.0 * k) as i32, (300.0 * k) as i32, None, None, Some(instance.into()), None)?;

        let theme = Theme::system();
        if let Err(e) = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark) {
            eprintln!("fluent-win buttons: backdrop unavailable: {e}");
        }
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let panel = Panel::new(controls(&surface, &formats)?);

        let mut app = App {
            hwnd, surface, theme, formats, panel,
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
                animating = a.panel.animating();
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
