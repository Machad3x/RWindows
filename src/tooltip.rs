//! WinUI `ToolTip`: the `ToolTipPresenter` look and the `ToolTipService` timing.
//! The window is a popup owned by the app, as with [`crate::flyout`].

use std::ptr;
use std::time::{Duration, Instant};

use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETMESSAGEDURATION, SPI_GETMOUSEHOVERTIME, SystemParametersInfoW,
};
use windows::core::Result;

use crate::flyout::FlyoutStyle;
use crate::gfx::Surface;
use crate::panel::Formats;
use crate::theme::{self, Theme};
use crate::gfx::rect;

/// `ToolTipBorderPadding` = 9,6,9,8 (left, top, right, bottom).
pub const PADDING: (f32, f32, f32, f32) = (9.0, 6.0, 9.0, 8.0);
/// `ToolTipMaxWidth`.
pub const MAX_WIDTH: f32 = 320.0;
/// WinUI fallbacks when `SystemParametersInfo` fails (`ToolTipService_Partial.h`).
pub const FALLBACK_HOVER: Duration = Duration::from_millis(400);
/// WinUI fallback for `SPI_GETMESSAGEDURATION` (`ToolTipService_Partial.h`).
pub const FALLBACK_DURATION: Duration = Duration::from_secs(5);
/// `BETWEEN_SHOW_DELAY_MS`: a tooltip closed this recently reopens on the reshow timer.
pub const RESHOW_WINDOW: Duration = Duration::from_millis(200);

/// The system values the `ToolTipService` timers use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    /// `SPI_GETMOUSEHOVERTIME` (400 ms fallback).
    pub hover: Duration,
    /// `SPI_GETMESSAGEDURATION` in seconds (5 s fallback).
    pub duration: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { hover: FALLBACK_HOVER, duration: FALLBACK_DURATION }
    }
}

impl Timing {
    /// Reads the two system parameters, falling back to WinUI's defaults.
    pub fn system() -> Self {
        let mut hover = 0u32;
        let mut duration = 0u32;
        let ok_hover = unsafe {
            SystemParametersInfoW(SPI_GETMOUSEHOVERTIME, 0, Some(ptr::from_mut(&mut hover).cast()), Default::default())
        }.is_ok();
        let ok_duration = unsafe {
            SystemParametersInfoW(SPI_GETMESSAGEDURATION, 0, Some(ptr::from_mut(&mut duration).cast()), Default::default())
        }.is_ok();
        Self {
            hover: if ok_hover && hover > 0 { Duration::from_millis(u64::from(hover)) } else { FALLBACK_HOVER },
            duration: if ok_duration && duration > 0 { Duration::from_secs(u64::from(duration)) } else { FALLBACK_DURATION },
        }
    }

    /// Mouse delay: 2x the hover time, or 1.5x on a reshow (`GetInitialShowDelay`).
    fn delay(self, reshow: bool) -> Duration {
        if reshow { self.hover * 3 / 2 } else { self.hover * 2 }
    }
}

/// Rectangle as `(left, top, right, bottom)`, in the same coordinate space as
/// the pointer passed to [`Host::pointer`].
type Bounds = (f32, f32, f32, f32);

fn in_rect(x: f32, y: f32, r: Bounds) -> bool {
    x >= r.0 && x <= r.2 && y >= r.1 && y <= r.3
}

fn cross(o: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// `ToolTipService::IsToolTipInSafeZone`: inside the owner, the tooltip, or the
/// convex hull of both (so moving towards the tooltip does not dismiss it).
/// Stack-only: no hull vector is allocated on each pointer move.
pub fn in_safe_zone(point: (f32, f32), owner: Bounds, tip: Bounds) -> bool {
    let (x, y) = point;
    if in_rect(x, y, owner) || in_rect(x, y, tip) {
        return true;
    }
    let mut pts = [
        (owner.0, owner.1), (owner.2, owner.1), (owner.2, owner.3), (owner.0, owner.3),
        (tip.0, tip.1), (tip.2, tip.1), (tip.2, tip.3), (tip.0, tip.3),
    ];
    pts.sort_unstable_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    // Andrew's monotone chain over the 8 corners.
    let mut hull = [(0.0f32, 0.0f32); 16];
    let mut n = 0;
    for &p in &pts {
        while n >= 2 && cross(hull[n - 2], hull[n - 1], p) <= 0.0 { n -= 1; }
        hull[n] = p;
        n += 1;
    }
    let lower = n;
    for i in (0..pts.len() - 1).rev() {
        let p = pts[i];
        while n > lower && cross(hull[n - 2], hull[n - 1], p) <= 0.0 { n -= 1; }
        hull[n] = p;
        n += 1;
    }
    n -= 1; // the last point repeats the first
    (0..n).all(|i| cross(hull[i], hull[(i + 1) % n], point) >= 0.0)
}

/// What the app must do after a [`Host`] state change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Show the tooltip for this control (owner index).
    Open(usize),
    /// Hide the tooltip.
    Close,
}

/// `ToolTipService` state for one window: one owner at a time, mouse and keyboard
/// share the same timers. The window schedules the next tick with `SetTimer`.
#[derive(Debug)]
pub struct Host {
    owner: Option<usize>,
    entered: Instant,
    opened: Option<Instant>,
    shown: bool,
    /// Captured on enter, as `ToolTipService::OnOwnerEnterInternal` does.
    reshow: bool,
    last_close: Option<Instant>,
    /// Owner and tooltip bounds in the pointer's coordinate space.
    bounds: Option<(Bounds, Bounds)>,
    timing: Timing,
}

impl Host {
    /// Starts with no owner and `timing`'s delay/duration.
    pub fn new(timing: Timing) -> Self {
        Self { owner: None, entered: Instant::now(), opened: None, shown: false, reshow: false, last_close: None, bounds: None, timing }
    }

    /// Pointer or keyboard focus entered `owner`.
    pub fn enter(&mut self, owner: usize, now: Instant) -> Option<Action> {
        if self.owner == Some(owner) {
            return None;
        }
        let close = self.opened.take().is_some();
        if close {
            self.last_close = Some(now);
        }
        self.owner = Some(owner);
        self.entered = now;
        self.shown = false;
        self.bounds = None;
        self.reshow = self.last_close.is_some_and(|t| now.duration_since(t) < RESHOW_WINDOW);
        close.then_some(Action::Close)
    }

    /// Pointer or focus left the owner: cancel a pending tooltip or close the open one.
    pub fn leave(&mut self, now: Instant) -> Option<Action> {
        let close = self.opened.take().is_some();
        self.owner = None;
        self.bounds = None;
        if close {
            self.last_close = Some(now);
        }
        close.then_some(Action::Close)
    }

    /// Ctrl dismisses a tooltip (Windows 11 21H2+).
    pub fn dismiss(&mut self, now: Instant) -> Option<Action> {
        self.leave(now)
    }

    /// True while the tooltip is visible.
    pub fn is_open(&self) -> bool {
        self.opened.is_some()
    }

    /// Bounds used by the safe zone, set when the tooltip is placed.
    pub fn set_bounds(&mut self, owner: Bounds, tip: Bounds) {
        self.bounds = Some((owner, tip));
    }

    /// True while an open tooltip still contains `point` in its safe zone.
    pub fn is_in_safe_zone(&self, x: f32, y: f32) -> bool {
        matches!(self.bounds, Some((owner, tip)) if in_safe_zone((x, y), owner, tip))
    }

    /// Pointer moved: dismiss when it leaves the safe zone around an open tooltip.
    pub fn pointer(&mut self, x: f32, y: f32, now: Instant) -> Option<Action> {
        if let (true, Some((owner, tip))) = (self.opened.is_some(), self.bounds)
            && !in_safe_zone((x, y), owner, tip)
        {
            return self.leave(now);
        }
        None
    }

    /// Delay the caller should wait before the next `tick`.
    pub fn next_delay(&self, now: Instant) -> Duration {
        if let Some(opened) = self.opened {
            self.timing.duration.saturating_sub(now.saturating_duration_since(opened))
        } else if self.owner.is_some() && !self.shown {
            self.timing.delay(self.reshow).saturating_sub(now.saturating_duration_since(self.entered))
        } else {
            Duration::MAX
        }
    }

    /// Advances the open/close timers; call again with [`Self::next_delay`].
    pub fn tick(&mut self, now: Instant) -> Option<Action> {
        if let Some(opened) = self.opened {
            if now.duration_since(opened) >= self.timing.duration {
                self.opened = None;
                self.last_close = Some(now);
                return Some(Action::Close);
            }
            return None;
        }
        let owner = self.owner?;
        if self.shown {
            return None;
        }
        if now.duration_since(self.entered) >= self.timing.delay(self.reshow) {
            self.opened = Some(now);
            self.shown = true;
            return Some(Action::Open(owner));
        }
        None
    }
}

/// Measured `ToolTip` content.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolTip {
    /// Text content (a plain string, no rich content).
    pub text: String,
    /// Outside size in DIPs, including [`PADDING`].
    pub size: (f32, f32),
}

impl ToolTip {
    /// Measures `text` in Caption, wrapping at [`MAX_WIDTH`].
    pub fn new(text: impl Into<String>, surface: &Surface, format: &windows::Win32::Graphics::DirectWrite::IDWriteTextFormat) -> Result<Self> {
        let text = text.into();
        let (w, h) = surface.measure_wrapped(&text, format, MAX_WIDTH)?;
        Ok(Self { text, size: (w + PADDING.0 + PADDING.2, h + PADDING.1 + PADDING.3) })
    }

    /// Draws the presenter: acrylic/background, 1 DIP `SurfaceStrokeColorFlyout`
    /// border, `ControlCornerRadius`, then the Caption text. The same brushes as
    /// the flyout, but the smaller corner radius of `DefaultToolTipStyle`.
    pub fn draw(&self, s: &Surface, style: &FlyoutStyle, f: &Formats, t: &Theme) {
        crate::flyout::draw_chrome_r(s, style, self.size, theme::CONTROL_RADIUS);
        s.text(&self.text, &f.caption,
            rect(PADDING.0, PADDING.1, self.size.0 - PADDING.2, self.size.1 - PADDING.3),
            t.text_primary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing() -> Timing {
        Timing { hover: Duration::from_millis(400), duration: Duration::from_secs(5) }
    }

    #[test]
    fn mouse_delay_should_be_twice_the_hover_time_and_reshow_one_and_a_half() {
        let t = timing();
        assert_eq!(t.delay(false), Duration::from_millis(800));
        assert_eq!(t.delay(true), Duration::from_millis(600));
    }

    #[test]
    fn hovering_should_open_after_the_mouse_delay() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(3, now);
        assert_eq!(h.tick(now + Duration::from_millis(799)), None);
        assert_eq!(h.tick(now + Duration::from_millis(800)), Some(Action::Open(3)));
        assert!(h.is_open());
    }

    #[test]
    fn leaving_before_the_delay_should_cancel() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        h.leave(now + Duration::from_millis(100));
        assert_eq!(h.tick(now + Duration::from_secs(1)), None);
    }

    #[test]
    fn the_show_duration_should_close_and_not_reopen_while_still_hovered() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        h.tick(now + Duration::from_millis(800));
        let close = now + Duration::from_millis(800) + Duration::from_secs(5);
        assert_eq!(h.tick(close), Some(Action::Close));
        assert_eq!(h.next_delay(close), Duration::MAX);
        assert_eq!(h.tick(close + Duration::from_secs(1)), None);
        assert!(!h.is_open());
    }

    #[test]
    fn entering_another_owner_should_close_the_first_tooltip() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        h.tick(now + Duration::from_millis(800));
        assert_eq!(h.enter(2, now + Duration::from_secs(1)), Some(Action::Close));
        assert!(!h.is_open());
        assert_eq!(h.tick(now + Duration::from_secs(2)), Some(Action::Open(2)));
    }

    #[test]
    fn reentering_the_same_owner_should_not_restart_the_delay() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        assert_eq!(h.enter(1, now + Duration::from_millis(500)), None);
        assert_eq!(h.tick(now + Duration::from_millis(800)), Some(Action::Open(1)));
    }

    #[test]
    fn a_reshow_within_200_ms_should_use_the_shorter_delay() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        let open = now + Duration::from_millis(800);
        h.tick(open);
        h.leave(open + Duration::from_millis(100));
        h.enter(2, open + Duration::from_millis(200));
        assert_eq!(h.tick(open + Duration::from_millis(800)), Some(Action::Open(2)));
    }

    #[test]
    fn ctrl_should_dismiss_an_open_tooltip() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        h.tick(now + Duration::from_millis(800));
        assert_eq!(h.dismiss(now + Duration::from_secs(1)), Some(Action::Close));
        assert!(!h.is_open());
    }

    #[test]
    fn next_delay_should_cover_the_remaining_time() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        assert_eq!(h.next_delay(now), Duration::MAX);
        h.enter(1, now);
        assert_eq!(h.next_delay(now + Duration::from_millis(100)), Duration::from_millis(700));
        h.tick(now + Duration::from_millis(800));
        assert_eq!(h.next_delay(now + Duration::from_millis(900)), Duration::from_secs(5) - Duration::from_millis(100));
    }

    #[test]
    fn safe_zone_should_include_the_owner_the_tooltip_and_the_path_between() {
        let owner = (0.0, 0.0, 10.0, 10.0);
        let tip = (0.0, -20.0, 10.0, -10.0);
        assert!(in_safe_zone((5.0, 5.0), owner, tip));
        assert!(in_safe_zone((5.0, -15.0), owner, tip));
        assert!(in_safe_zone((5.0, -5.0), owner, tip));
        assert!(!in_safe_zone((20.0, -5.0), owner, tip));
        assert!(!in_safe_zone((-20.0, -15.0), owner, tip));
    }

    #[test]
    fn safe_zone_should_bridge_diagonally_placed_rects() {
        let owner = (0.0, 0.0, 10.0, 10.0);
        let tip = (30.0, -20.0, 40.0, -10.0);
        assert!(in_safe_zone((20.0, -5.0), owner, tip));
        assert!(!in_safe_zone((40.0, 0.0), owner, tip));
    }

    #[test]
    fn moving_out_of_the_safe_zone_should_close_an_open_tooltip() {
        let mut h = Host::new(timing());
        let now = Instant::now();
        h.enter(1, now);
        h.tick(now + Duration::from_millis(800));
        h.set_bounds((0.0, 0.0, 10.0, 10.0), (0.0, -20.0, 10.0, -10.0));
        assert_eq!(h.pointer(5.0, -5.0, now + Duration::from_secs(1)), None);
        assert_eq!(h.pointer(50.0, -5.0, now + Duration::from_secs(1)), Some(Action::Close));
        assert!(!h.is_open());
    }

    #[test]
    fn system_timing_should_return_positive_values() {
        let t = Timing::system();
        assert!(t.hover > Duration::ZERO);
        assert!(t.duration > Duration::ZERO);
    }
}
