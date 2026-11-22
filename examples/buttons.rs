//! WinUI controls on Mica: buttons (default, accent, disabled, toggle, subtle),
//! a ToggleSwitch and CheckBoxes, routed by `fluent_win::panel::Panel`.
//! Tab / Shift+Tab move keyboard focus (focus ring), Space / Enter invoke, the
//! mouse captures while pressed. Idle = blocked in GetMessageW; frames render
//! only during transitions. UI Automation: the controls appear to Narrator /
//! NVDA / Inspect through `fluent_win::uia`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use fluent_win::backdrop;
use fluent_win::button::{Button, FocusKind, Input, Key};
use fluent_win::check_box::{CheckBox, CheckState};
use fluent_win::flyout;
use fluent_win::gfx::{Surface, rect};
use fluent_win::layout::{Orientation, Stack};
use fluent_win::menu_flyout::{Entry, Item, MenuFlyout};
use fluent_win::panel::{Control, Event, Formats, Outcome, Panel};
use fluent_win::slider::{self, Slider};
use fluent_win::theme::Theme;
use fluent_win::toggle_switch::ToggleSwitch;
use fluent_win::tooltip::{self, Action, Timing, ToolTip};
use fluent_win::uia::{self, WM_UIA_FOCUS, WM_UIA_INVOKE, WM_UIA_SET_VALUE};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Dwm::{DWMSBT_MAINWINDOW, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect, ValidateRect,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    IRawElementProviderSimple, ToggleState, ToggleState_Indeterminate, ToggleState_On,
    UIA_AutomationFocusChangedEventId, UIA_BoundingRectanglePropertyId, UIA_EVENT_ID, UIA_Invoke_InvokedEventId,
    UIA_RangeValueValuePropertyId, UIA_ToggleToggleStatePropertyId, UiaClientsAreListening, UiaRaiseAutomationEvent,
    UiaRaiseAutomationPropertyChangedEvent, UiaReturnRawElementProvider, UiaRootObjectId,
};
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, GetDpiForSystem, MDT_EFFECTIVE_DPI,
    SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
    VIRTUAL_KEY, VK_ADD, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
    VK_CONTROL, VK_ESCAPE, VK_SUBTRACT, VK_TAB, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, IDC_ARROW,
    LoadCursorW, MSG, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW, SIZE_MINIMIZED,
    SPI_SETHIGHCONTRAST, SWP_NOACTIVATE, SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_CAPTURECHANGED,
    WM_DESTROY, WM_DPICHANGED, WM_GETOBJECT, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_QUIT, WM_SETTINGCHANGE, WM_SIZE, WM_SYSCOLORCHANGE,
    WM_TIMER, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_NOREDIRECTIONBITMAP, WS_EX_TRANSPARENT,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE, DestroyWindow, SW_SHOW, KillTimer, SetTimer,
    SWP_NOSIZE, ShowWindow, WA_INACTIVE, WM_ACTIVATE, WM_CLOSE, WM_NULL, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP,
};
use windows::core::{PCWSTR, Result, w};

const PAD: f32 = 32.0;
const GAP: f32 = 8.0;
/// Id of the single one-shot tooltip timer.
const TIP_TIMER: usize = 1;
/// Pointer placement rect half-size in DIPs.
const TIP_POINT: f32 = 8.0;

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
    /// The open flyout, menu or tooltip, if any.
    popup: Option<Popup>,
    /// `ToolTipService` state (delays come from this machine's system settings).
    tip: tooltip::Host,
    /// System delays, read once at startup instead of on every tooltip.
    timing: Timing,
    /// Armed one-shot timer, in milliseconds (None = no timer).
    tip_timer: Option<u32>,
    /// Pointer position in client DIPs, used as the tooltip's placement target.
    tip_at: (f32, f32),
}

/// Label of the button that opens the flyout.
const MORE: &str = "Mais opções";
/// Label of the button that opens the menu.
const MENU: &str = "Menu";

/// A flyout: its own popup window (acrylic backdrop), surface and content.
struct Popup {
    hwnd: HWND,
    surface: Surface,
    content: Content,
    /// Size in DIPs.
    size: (f32, f32),
    /// Final top-left in screen pixels; the transition offsets it.
    origin: (i32, i32),
    transition: flyout::Transition,
    closing: bool,
    /// True when the DWM acrylic backdrop is active (else the fallback color fills).
    backdrop: bool,
}

/// What a popup hosts: a content flyout, a menu or a tooltip.
enum Content {
    Panel(Panel),
    Menu(MenuFlyout),
    Tip(ToolTip),
}

impl Content {
    fn draw(&self, s: &Surface, style: &flyout::FlyoutStyle, size: (f32, f32), t: &Theme, f: &Formats) -> Result<()> {
        match self {
            Content::Tip(tip) => {
                // `DefaultToolTipStyle` uses the flyout brushes with `ControlCornerRadius`.
                tip.draw(s, style, f, t);
                Ok(())
            }
            Content::Panel(p) => {
                flyout::draw_chrome(s, style, size);
                p.draw(s, t, f)
            }
            Content::Menu(m) => {
                flyout::draw_chrome(s, style, size);
                m.draw(s, t, f);
                Ok(())
            }
        }
    }

    fn animating(&self) -> bool {
        match self {
            Content::Panel(p) => p.animating(),
            Content::Menu(_) | Content::Tip(_) => false,
        }
    }
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
    /// RangeValue `Value` property changed from the first value to the second.
    Value(f64, f64),
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
        s.text(&self.status, &self.formats.body, rect(PAD, 236.0, PAD + 500.0, 260.0), t.text_secondary);
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
                    if self.panel.controls[i].name() == MORE {
                        self.open_flyout(i)?;
                    } else if self.panel.controls[i].name() == MENU {
                        self.open_menu(i)?;
                    }
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
                Event::Focused(i) => {
                    self.raise(i, UiaEvent::Event(UIA_AutomationFocusChangedEventId));
                    // Keyboard focus also opens a tooltip (ToolTipService hooks GotFocus).
                    let r = self.panel.controls[i].rect();
                    self.tip_at = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    let action = self.tip.enter(i, Instant::now());
                    self.tip_after(action)?;
                }
                Event::ValueChanged { index, old, new } => {
                    self.status = format!("{}: {new}", self.panel.controls[index].name());
                    self.raise(index, UiaEvent::Value(old, new));
                }
            }
        }
        if out.redraw {
            self.render()?;
        }
        Ok(())
    }

    /// Opens the flyout under control `anchor` (`FlyoutPlacementMode.Bottom`).
    fn open_flyout(&mut self, anchor: usize) -> Result<()> {
        // Content inside FlyoutContentPadding: a ToggleSwitch and a Slider.
        let (pl, pt, pr, pb) = flyout::PADDING;
        let switch = ToggleSwitch::layout(&self.surface, &self.formats.switch, "Luz noturna", "Ativado", "Desativado", pl, pt, false)?;
        let inner = (switch.rect.right - switch.rect.left).max(248.0);
        let top = switch.rect.bottom + GAP;
        let slider = Slider::new("Brilho", rect(pl, top, pl + inner, top + slider::HEIGHT), 0.0, 100.0, 70.0);
        let size = (
            (pl + inner + pr).clamp(flyout::WIDTH_RANGE.0, flyout::WIDTH_RANGE.1),
            (top + slider::HEIGHT + pb).clamp(flyout::HEIGHT_RANGE.0, flyout::HEIGHT_RANGE.1),
        );
        let content = Content::Panel(Panel::new(vec![Box::new(switch), Box::new(slider)]));
        self.open_popup(anchor, content, size, w!("Mais opções"), false).map(|_| ())
    }

    /// Opens a `MenuFlyout` under control `anchor`, like `Button.Flyout`.
    fn open_menu(&mut self, anchor: usize) -> Result<()> {
        let mut copy = Item::new("Copiar");
        copy.accelerator = Some("Ctrl+C".to_owned());
        let mut top = Item::new("Mover para o topo");
        top.enabled = false;
        let entries = vec![
            Entry::Item(Item::new("Abrir")),
            Entry::Item(Item::new("Fixar na barra de tarefas")),
            Entry::Separator,
            Entry::Item(copy),
            Entry::Item(top),
        ];
        let sizes = entries.iter().map(|e| e.measure(&self.surface, &self.formats)).collect::<Result<Vec<_>>>()?;
        let menu = MenuFlyout::new(entries, &sizes);
        let size = menu.size();
        self.open_popup(anchor, Content::Menu(menu), size, w!("Menu"), false).map(|_| ())
    }

    /// Tooltip text attached to a control, like `ToolTipService.ToolTip`.
    fn tip_text(panel: &Panel, index: usize) -> Option<&'static str> {
        Some(match panel.controls.get(index)?.name() {
            "Configurações" => "Abre as configurações rápidas",
            "Salvar" => "Salva o documento atual",
            "Wi-Fi" => "Gerencia as conexões de rede",
            MENU => "Mostra as ações disponíveis",
            MORE => "Opções adicionais de configuração",
            _ => return None,
        })
    }

    /// Destroys a visible (or closing) tooltip popup without waiting for the fade.
    fn destroy_tip_popup(&mut self) {
        let hwnd = self.popup.as_ref()
            .filter(|p| matches!(p.content, Content::Tip(_)))
            .map(|p| p.hwnd);
        if let Some(hwnd) = hwnd {
            self.popup = None;
            _ = unsafe { DestroyWindow(hwnd) };
        }
    }

    /// Cancels a pending/open tooltip, e.g. when a click opens a flyout or menu.
    fn cancel_tip(&mut self) {
        unsafe { _ = KillTimer(Some(self.hwnd), TIP_TIMER) };
        self.tip_timer = None;
        self.destroy_tip_popup();
        self.tip = tooltip::Host::new(self.timing);
    }

    fn open_tooltip(&mut self, owner: usize) -> Result<()> {
        let Some(text) = Self::tip_text(&self.panel, owner) else { return Ok(()) };
        // A flyout/menu owns the popup slot; a previous tooltip is replaced at once.
        if self.popup.as_ref().is_some_and(|p| !matches!(p.content, Content::Tip(_))) {
            return Ok(());
        }
        self.destroy_tip_popup();
        let tip = ToolTip::new(text, &self.surface, &self.formats.caption)?;
        let size = tip.size;
        let placed = self.open_popup(owner, Content::Tip(tip), size, w!("Dica"), true)?;
        if let Some(t) = placed {
            let r = self.panel.controls[owner].rect();
            self.tip.set_bounds((r.left, r.top, r.right, r.bottom), (t.left, t.top, t.right, t.bottom));
        }
        Ok(())
    }

    fn tip_move(&mut self, x: f32, y: f32) -> Result<()> {
        self.tip_at = (x, y);
        let safe = self.tip.pointer(x, y, Instant::now());
        let index = self.panel.controls.iter().position(|c| {
            let r = c.rect();
            x >= r.left && x < r.right && y >= r.top && y < r.bottom
        });
        // The safe zone wins: moving towards the tooltip must not re-enter or leave its owner.
        let action = if safe.is_some() { safe } else if self.tip.is_in_safe_zone(x, y) { None } else {
            match index {
                Some(i) => self.tip.enter(i, Instant::now()),
                None => self.tip.leave(Instant::now()),
            }
        };
        self.tip_after(action)
    }

    fn tip_leave(&mut self) -> Result<()> {
        // The tooltip is its own HWND: TME_LEAVE fires on the main window when it
        // appears under the cursor. Keep the tooltip while the pointer is in the safe zone.
        let (x, y) = self.tip_at;
        if self.tip.is_in_safe_zone(x, y) {
            return Ok(());
        }
        let action = self.tip.leave(Instant::now());
        self.tip_after(action)
    }

    fn tip_dismiss(&mut self) -> Result<()> {
        let action = self.tip.dismiss(Instant::now());
        self.tip_after(action)
    }

    fn tip_tick(&mut self) -> Result<()> {
        let action = self.tip.tick(Instant::now());
        self.tip_after(action)
    }

    /// Applies a service action and schedules the next one-shot timer.
    fn tip_after(&mut self, action: Option<Action>) -> Result<()> {
        if let Some(action) = action {
            match action {
                Action::Open(owner) => self.open_tooltip(owner)?,
                Action::Close if self.popup.as_ref().is_some_and(|p| matches!(p.content, Content::Tip(_))) => self.close_popup(),
                Action::Close => {}
            }
        }
        let delay = self.tip.next_delay(Instant::now());
        let ms = (delay != Duration::MAX).then(|| delay.as_millis().min(u32::MAX as u128) as u32);
        // Pointer moves keep arriving while the same delay is armed: re-arm only on change.
        if ms != self.tip_timer {
            unsafe {
                match ms {
                    Some(ms) => { SetTimer(Some(self.hwnd), TIP_TIMER, ms, None); }
                    None => { _ = KillTimer(Some(self.hwnd), TIP_TIMER); }
                }
            }
            self.tip_timer = ms;
        }
        Ok(())
    }

    /// Opens `content` in a popup window. A tooltip is placed above the pointer,
    /// fades in and never takes focus; flyouts and menus open below `anchor`.
    fn open_popup(&mut self, anchor: usize, content: Content, size: (f32, f32), title: PCWSTR, tip: bool) -> Result<Option<D2D_RECT_F>> {
        if !tip {
            // A tooltip opened by the focus event of this very click must give up
            // the single popup slot before the flyout/menu is created.
            self.cancel_tip();
        }
        if self.popup.is_some() {
            return Ok(None);
        }
        // The anchor in screen pixels (main window scale), then everything in the DIPs
        // of the monitor under it: the popup is created there and draws at its DPI.
        let km = self.surface.scale();
        let a = self.panel.controls[anchor].rect();
        let mut o = POINT::default();
        unsafe { _ = ClientToScreen(self.hwnd, &mut o) };
        let px = |x: f32, origin: i32| origin + (x * km).round() as i32;
        let anchor_px = if tip {
            // `ToolTipService` docks to the pointer (a small rect) for mouse input.
            let (x, y) = self.tip_at;
            RECT { left: px(x - TIP_POINT, o.x), top: px(y - TIP_POINT, o.y), right: px(x + TIP_POINT, o.x), bottom: px(y + TIP_POINT, o.y) }
        } else {
            RECT { left: px(a.left, o.x), top: px(a.top, o.y), right: px(a.right, o.x), bottom: px(a.bottom, o.y) }
        };
        let (work, k) = unsafe {
            let monitor = MonitorFromRect(&anchor_px, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
            _ = GetMonitorInfoW(monitor, &mut info);
            let (mut dx, mut dy) = (96, 96);
            _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            (info.rcWork, dx as f32 / 96.0)
        };
        let dips = |r: RECT| rect(r.left as f32 / k, r.top as f32 / k, r.right as f32 / k, r.bottom as f32 / k);
        let (target, container) = (dips(anchor_px), dips(work));
        let (w, h) = size;
        let (r, placement) = flyout::place(target, w, h, container,
            if tip { flyout::Placement::Top } else { flyout::Placement::Bottom });
        let origin = ((r.left * k).round() as i32, (r.top * k).round() as i32);
        let transition = if tip { flyout::Transition::fade() } else { flyout::Transition::open(placement) };
        // Created at the first frame's position, so DWM never composes it at the final spot first.
        let start = transition.at(Duration::ZERO).offset;
        let (x0, y0) = (origin.0 + (start.0 * k).round() as i32, origin.1 + (start.1 * k).round() as i32);
        let mut style = WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
        if tip {
            // Click-through and no activation: a tooltip must not steal focus.
            style |= WS_EX_NOACTIVATE | WS_EX_TRANSPARENT;
        }
        let hwnd = unsafe {
            CreateWindowExW(style, w!("fluent-win-flyout"), title, WS_POPUP, x0, y0,
                (w * k).round() as i32, (h * k).round() as i32, Some(self.hwnd), None, None, None)?
        };
        let backdrop = backdrop::apply(hwnd, DWMSBT_TRANSIENTWINDOW, DWMWCP_ROUND, self.theme.dark).is_ok();
        let surface = Surface::new(hwnd)?;
        self.popup = Some(Popup {
            hwnd, surface, content, size, origin, transition, closing: false, backdrop,
        });
        if !tip {
            // Keyboard focus moves into the popup; the WM_KILLFOCUS that ShowWindow sends
            // the main window arrives while APP is borrowed and is skipped, so blur here.
            let out = self.panel.focus(None, FocusKind::Keyboard);
            if out.redraw {
                self.render()?;
            }
        }
        self.frame_popup()?;
        unsafe { _ = ShowWindow(hwnd, SW_SHOW) };
        // The final rect, in main-window client DIPs, feeds the tooltip safe zone.
        Ok(tip.then(|| {
            let (x, y) = (origin.0 - o.x, origin.1 - o.y);
            let (right, bottom) = (x + (w * k).round() as i32, y + (h * k).round() as i32);
            rect(x as f32 / km, y as f32 / km, right as f32 / km, bottom as f32 / km)
        }))
    }

    /// Positions and draws the flyout for the current transition frame; destroys
    /// it once the close fade ends.
    fn frame_popup(&mut self) -> Result<()> {
        let Some(p) = self.popup.as_mut() else { return Ok(()) };
        let f = p.transition.now();
        if p.closing && !f.running {
            let hwnd = p.hwnd;
            self.popup = None;
            return unsafe { DestroyWindow(hwnd) };
        }
        let k = p.surface.scale();
        let (x, y) = (p.origin.0 + (f.offset.0 * k).round() as i32, p.origin.1 + (f.offset.1 * k).round() as i32);
        unsafe { SetWindowPos(p.hwnd, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)? };
        let style = flyout::style(&self.theme, p.backdrop);
        let Popup { surface, content, size, .. } = p;
        surface.begin();
        surface.with_opacity(f.opacity, |s| content.draw(s, &style, *size, &self.theme, &self.formats))?;
        surface.end()
    }

    /// Light dismiss / Esc: starts the close fade.
    fn close_popup(&mut self) {
        if let Some(p) = self.popup.as_mut().filter(|p| !p.closing) {
            p.closing = true;
            // From wherever the open animation is, without a jump.
            p.transition = flyout::Transition::close_from(p.transition.now());
            // A cross-process deactivation is a sent message handled inside a blocked
            // GetMessageW, which does not return for it: post something to wake the loop.
            unsafe { _ = PostMessageW(Some(p.hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
        }
    }

    fn popup_animating(&self) -> bool {
        self.popup.as_ref().is_some_and(|p| p.closing || p.transition.now().running || p.content.animating())
    }

    /// Routes input to the popup's content and shows changes in the status line.
    fn popup_input(&mut self, input: Input) -> Result<()> {
        let Some(p) = self.popup.as_mut() else { return Ok(()) };
        let (capture, redraw, status, close) = match &mut p.content {
            Content::Panel(panel) => {
                let out = panel.handle(input);
                let status = out.events.iter().filter_map(|event| match *event {
                    Event::Toggled { index, new, .. } => {
                        let state = if new == ToggleState_On { "ligado" } else { "desligado" };
                        Some(format!("{}: {state}", panel.controls[index].name()))
                    }
                    Event::ValueChanged { index, new, .. } => Some(format!("{}: {new}", panel.controls[index].name())),
                    _ => None,
                }).next_back();
                (out.capture, out.redraw, status, false)
            }
            Content::Menu(menu) => {
                let out = menu.handle(input);
                let close = out.invoked.is_some();
                let status = out.invoked.map(|i| format!("Clicou: {}", menu.text(i).unwrap_or_default()));
                (out.capture, out.redraw, status, close)
            }
            // Click-through: a tooltip never receives pointer or keyboard input.
            Content::Tip(_) => (None, false, None, false),
        };
        match capture {
            Some(true) => unsafe { _ = SetCapture(p.hwnd) },
            Some(false) => unsafe { ReleaseCapture()? },
            None => {}
        }
        if redraw {
            self.frame_popup()?;
        }
        if let Some(status) = status {
            self.status = status;
            self.render()?;
        }
        if close {
            // An invoked menu item closes the menu, as MenuFlyout does.
            self.close_popup();
        }
        Ok(())
    }

    fn popup_key(&mut self, vk: VIRTUAL_KEY, down: bool) -> Result<()> {
        if down && vk == VK_ESCAPE {
            self.close_popup();
            return Ok(());
        }
        let Some(p) = self.popup.as_mut() else { return Ok(()) };
        if let Content::Panel(panel) = &mut p.content
            && down
            && vk == VK_TAB
        {
            let back = unsafe { GetKeyState(i32::from(VK_SHIFT.0)) } < 0;
            if panel.tab(back).redraw {
                self.frame_popup()?;
            }
            return Ok(());
        }
        self.popup_input(if down { Input::KeyDown(key_of(vk)) } else { Input::KeyUp(key_of(vk)) })
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
        let out = self.panel.key(key_of(vk), down);
        self.apply(out)
    }
}

/// The control key for a virtual key.
fn key_of(vk: VIRTUAL_KEY) -> Key {
    match vk {
        VK_SPACE => Key::Space,
        VK_RETURN => Key::Enter,
        VK_ADD => Key::Add,
        VK_SUBTRACT => Key::Subtract,
        VK_LEFT => Key::Left,
        VK_RIGHT => Key::Right,
        VK_UP => Key::Up,
        VK_DOWN => Key::Down,
        VK_HOME => Key::Home,
        VK_END => Key::End,
        _ => Key::Other,
    }
}

/// Window procedure of the flyout popup.
extern "system" fn popup_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let point = |a: &App| {
        let k = a.popup.as_ref().map_or(1.0, |p| p.surface.scale());
        (f32::from(loword(lp.0)) / k, f32::from(hiword(lp.0)) / k)
    };
    match msg {
        WM_PAINT => {
            unsafe { _ = ValidateRect(Some(hwnd), None) };
            with_app(App::frame_popup);
        }
        // Light dismiss: losing activation (a click elsewhere) closes the flyout.
        WM_ACTIVATE if u32::from(loword(wp.0 as isize) as u16) == WA_INACTIVE => with_app(|a| {
            a.close_popup();
            Ok(())
        }),
        WM_MOUSEMOVE => with_app(|a| {
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            unsafe { TrackMouseEvent(&mut tme)? };
            let (x, y) = point(a);
            a.popup_input(Input::PointerMove(x, y))
        }),
        WM_MOUSELEAVE => with_app(|a| a.popup_input(Input::PointerLeave)),
        WM_LBUTTONDOWN => with_app(|a| {
            let (x, y) = point(a);
            a.popup_input(Input::PointerDown(x, y))
        }),
        WM_LBUTTONUP => with_app(|a| {
            let (x, y) = point(a);
            a.popup_input(Input::PointerUp(x, y))
        }),
        WM_KEYDOWN => with_app(|a| a.popup_key(VIRTUAL_KEY(wp.0 as u16), true)),
        WM_KEYUP => with_app(|a| a.popup_key(VIRTUAL_KEY(wp.0 as u16), false)),
        WM_CAPTURECHANGED if lp.0 != hwnd.0 as isize => with_app(|a| a.popup_input(Input::CaptureLost)),
        // Alt+F4, or a move to a monitor with another DPI: close through the fade, so
        // App.popup and the window are dropped together.
        WM_CLOSE | WM_DPICHANGED => with_app(|a| {
            a.close_popup();
            Ok(())
        }),
        _ => return unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
    LRESULT(0)
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
                UiaEvent::Value(old, new) => unsafe {
                    let (old, new) = (VARIANT::from(old), VARIANT::from(new));
                    UiaRaiseAutomationPropertyChangedEvent(&provider, UIA_RangeValueValuePropertyId, &old, &new)
                },
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
    with_app_or_repost_lp(hwnd, msg, wp, LPARAM(0), f);
}

/// [`with_app_or_repost`] keeping `lParam` (WM_UIA_SET_VALUE carries the value there).
fn with_app_or_repost_lp(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, f: impl FnOnce(&mut App) -> Result<()>) {
    if !try_with_app(f) {
        unsafe { _ = PostMessageW(Some(hwnd), msg, wp, lp) };
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
            a.input(Input::PointerMove(x, y))?;
            a.tip_move(x, y)
        }),
        WM_MOUSELEAVE => with_app(|a| {
            a.tracking = false;
            a.input(Input::PointerLeave)?;
            a.tip_leave()
        }),
        WM_LBUTTONDOWN => with_app(|a| {
            a.cancel_tip();
            let (x, y) = point(a, lp);
            a.input(Input::PointerDown(x, y))
        }),
        WM_LBUTTONUP => with_app(|a| {
            let (x, y) = point(a, lp);
            a.input(Input::PointerUp(x, y))
        }),
        // Ctrl dismisses a tooltip (Windows 11 21H2+).
        WM_KEYDOWN if wp.0 as u16 == VK_CONTROL.0 => with_app(App::tip_dismiss),
        WM_KEYDOWN => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), true)),
        WM_KEYUP => with_app(|a| a.key(VIRTUAL_KEY(wp.0 as u16), false)),
        // Window lost focus: the focused control loses it too (OnLostFocus).
        WM_KILLFOCUS => with_app(|a| {
            let out = a.panel.focus(None, FocusKind::Keyboard);
            a.apply(out)?;
            a.tip_leave()
        }),
        WM_TIMER if wp.0 == TIP_TIMER => with_app(App::tip_tick),
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
                if let Some(p) = &a.popup {
                    backdrop::set_dark(p.hwnd, a.theme.dark)?;
                }
                a.render()?;
                a.frame_popup()
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
        // Posted by IRangeValueProvider::SetValue (the provider already validated the range).
        WM_UIA_SET_VALUE => with_app_or_repost_lp(hwnd, msg, wp, lp, |a| {
            let out = a.panel.set_value(wp.0, uia::set_value_of(lp));
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

/// The demo's controls, in three rows; the two button rows use `layout::Stack`.
fn controls(surface: &Surface, f: &Formats) -> Result<Vec<Box<dyn Control>>> {
    let mut controls: Vec<Box<dyn Control>> = Vec::new();
    // First row: measured buttons, stacked left to right by `layout::Stack`.
    let first = Stack::new(Orientation::Horizontal).with_spacing(GAP);
    let row = [("Configurações", false, true), ("Salvar", true, true),
        ("Desabilitado", false, false), ("Accent off", true, false)];
    let sizes = row.iter()
        .map(|&(text, _, _)| Button::desired_size(surface, &f.label, text))
        .collect::<Result<Vec<_>>>()?;
    for (&(text, accent, enabled), r) in row.iter().zip(first.arrange(&sizes, PAD, PAD)) {
        let mut b = Button::new(text, r, accent);
        b.set_enabled(enabled);
        controls.push(Box::new(b));
    }
    // Second row: a ToggleButton (Quick Settings tile) and a SubtleButton (taskbar).
    // Row height is the tallest child, like XAML; equal here since all use `f.label`.
    let second = Stack::new(Orientation::Horizontal).with_spacing(GAP);
    let top = PAD + first.desired_size(&sizes).1 + GAP * 2.0;
    let row = ["Wi-Fi", "Iniciar", MORE, MENU];
    let sizes = row.iter()
        .map(|&text| Button::desired_size(surface, &f.label, text))
        .collect::<Result<Vec<_>>>()?;
    let rects = second.arrange(&sizes, PAD, top);
    controls.push(Box::new(Button::toggle(row[0], rects[0], true)));
    controls.push(Box::new(Button::subtle(row[1], rects[1])));
    // Opens a Flyout below itself.
    controls.push(Box::new(Button::new(row[2], rects[2], false)));
    // Opens a MenuFlyout below itself.
    controls.push(Box::new(Button::new(row[3], rects[3], false)));
    // Third row: a ToggleSwitch with header (Settings style) and two CheckBoxes
    // centered on the switch row, the second one three-state.
    let top = top + second.desired_size(&sizes).1 + GAP;
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
    // Fourth row: a Slider (Quick Settings volume).
    let top = cy + 20.0 + GAP;
    controls.push(Box::new(Slider::new("Volume", rect(PAD, top, PAD + 240.0, top + slider::HEIGHT), 0.0, 100.0, 40.0)));
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
        RegisterClassW(&WNDCLASSW {
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            hInstance: instance.into(),
            lpszClassName: w!("fluent-win-flyout"),
            lpfnWndProc: Some(popup_proc),
            ..Default::default()
        });
        let k = GetDpiForSystem() as f32 / 96.0;
        let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP, class, w!("fluent-win · Button"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE, CW_USEDEFAULT, CW_USEDEFAULT,
            (520.0 * k) as i32, (330.0 * k) as i32, None, None, Some(instance.into()), None)?;

        let theme = Theme::system();
        if let Err(e) = backdrop::apply(hwnd, DWMSBT_MAINWINDOW, DWMWCP_ROUND, theme.dark) {
            eprintln!("fluent-win buttons: backdrop unavailable: {e}");
        }
        let surface = Surface::new(hwnd)?;
        let formats = Formats::new(&surface)?;
        let panel = Panel::new(controls(&surface, &formats)?);
        let timing = Timing::system();

        let mut app = App {
            hwnd, surface, theme, formats, panel,
            status: "Tab para focar, Espaço/Enter para clicar".to_owned(),
            tracking: false,
            uia: Rc::new(RefCell::new(uia::Model::default())),
            pending: Vec::new(),
            popup: None,
            timing,
            tip: tooltip::Host::new(timing),
            tip_timer: None,
            tip_at: (0.0, 0.0),
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
                animating = a.panel.animating() || a.popup_animating();
                Ok(())
            });
            if animating || was_animating {
                // Last frame after a transition lands exactly on its target color.
                with_app(|a| {
                    a.render()?;
                    a.frame_popup()
                });
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
