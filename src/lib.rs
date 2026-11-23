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

pub mod app_bar;
pub mod app_list;
pub mod backdrop;
pub mod button;
pub mod calendar;
pub mod check_box;
pub mod easing;
pub mod flyout;
pub mod gfx;
pub mod grid_view;
pub mod hotkeys;
pub mod icons;
pub mod layout;
pub mod list_view;
pub mod menu_flyout;
pub mod panel;
pub mod quick_settings;
pub mod radios;
pub mod progress;
pub mod radio_button;
pub mod scroll_viewer;
pub mod session;
pub mod shell_registration;
pub mod slider;
pub mod text_box;
pub mod theme;
pub mod toggle_switch;
pub mod tooltip;
pub mod uia;
pub mod window_list;
