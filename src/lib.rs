//! Windows 11 (Fluent) visuals for raw Win32 windows.
//!
//! - [`button`]: WinUI Button (default and accent) with WinUI input rules.
//! - [`backdrop`]: Mica/Acrylic, rounded corners, dark title bar via DWM.
//! - [`gfx`]: Direct2D + DirectWrite drawing on a DirectComposition swap chain,
//!   so transparent pixels show the DWM backdrop.
//! - [`theme`]: WinUI color tokens, type ramp, corner radii, system accent.
//! - [`easing`]: Win11 motion curves and durations.
//! - [`uia`]: UI Automation providers for custom-drawn controls.
#![deny(missing_docs)]

pub mod backdrop;
pub mod button;
pub mod check_box;
pub mod easing;
pub mod flyout;
pub mod gfx;
pub mod layout;
pub mod list_view;
pub mod menu_flyout;
pub mod panel;
pub mod slider;
pub mod theme;
pub mod toggle_switch;
pub mod tooltip;
pub mod uia;
