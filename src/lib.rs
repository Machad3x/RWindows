//! Windows 11 (Fluent) visuals for raw Win32 windows.
//!
//! - [`backdrop`]: Mica/Acrylic, rounded corners, dark title bar via DWM.
//! - [`gfx`]: Direct2D + DirectWrite drawing on a DirectComposition swap chain,
//!   so transparent pixels show the DWM backdrop.
//! - [`theme`]: WinUI color tokens, type ramp, corner radii, system accent.
//! - [`easing`]: Win11 motion curves and durations.
#![deny(missing_docs)]

pub mod backdrop;
pub mod easing;
