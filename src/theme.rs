//! WinUI (Windows 11) design tokens and the system accent color.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_REGULAR, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::UI::Color;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::core::{PCWSTR, Result, w};

/// WinUI `ControlCornerRadius` (buttons, text boxes, list items).
pub const CONTROL_RADIUS: f32 = 4.0;
/// WinUI `OverlayCornerRadius` (cards, flyouts, dialogs, menus).
pub const OVERLAY_RADIUS: f32 = 8.0;

/// `0xAARRGGBB` to a Direct2D color (straight alpha).
pub const fn argb(v: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        a: ((v >> 24) & 0xff) as f32 / 255.0,
        r: ((v >> 16) & 0xff) as f32 / 255.0,
        g: ((v >> 8) & 0xff) as f32 / 255.0,
        b: (v & 0xff) as f32 / 255.0,
    }
}

/// `color` with its alpha multiplied by `opacity`, like a XAML brush `Opacity`
/// (e.g. `AccentFillColorSecondaryBrush` = accent at 0.9).
pub const fn with_opacity(color: D2D1_COLOR_F, opacity: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a: color.a * opacity, ..color }
}

/// WinUI color tokens for one theme. Names follow the WinUI resource keys.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    /// True for the dark theme.
    pub dark: bool,
    /// `AccentFillColorDefault` (system accent shade for this theme).
    pub accent: D2D1_COLOR_F,
    /// `TextFillColorPrimary`.
    pub text_primary: D2D1_COLOR_F,
    /// `TextFillColorSecondary`.
    pub text_secondary: D2D1_COLOR_F,
    /// `TextFillColorTertiary`.
    pub text_tertiary: D2D1_COLOR_F,
    /// `ControlFillColorDefault` (button rest).
    pub control_fill: D2D1_COLOR_F,
    /// `ControlFillColorSecondary` (button hover).
    pub control_fill_hover: D2D1_COLOR_F,
    /// `ControlFillColorTertiary` (button pressed).
    pub control_fill_pressed: D2D1_COLOR_F,
    /// `CardBackgroundFillColorDefault`.
    pub card: D2D1_COLOR_F,
    /// `ControlStrokeColorDefault`.
    pub stroke: D2D1_COLOR_F,
    /// `SubtleFillColorSecondary` (list item / subtle button hover).
    pub subtle_hover: D2D1_COLOR_F,
    /// `SubtleFillColorTertiary` (list item / subtle button pressed).
    pub subtle_pressed: D2D1_COLOR_F,
    /// `TextFillColorDisabled`.
    pub text_disabled: D2D1_COLOR_F,
    /// `ControlFillColorDisabled`.
    pub control_fill_disabled: D2D1_COLOR_F,
    /// `ControlStrokeColorSecondary` (lit edge of `ControlElevationBorderBrush`).
    pub stroke_secondary: D2D1_COLOR_F,
    /// `ControlStrokeColorOnAccentDefault`.
    pub stroke_on_accent: D2D1_COLOR_F,
    /// `ControlStrokeColorOnAccentSecondary` (edge of `AccentControlElevationBorderBrush`).
    pub stroke_on_accent_secondary: D2D1_COLOR_F,
    /// `AccentFillColorDisabled`.
    pub accent_disabled: D2D1_COLOR_F,
    /// `TextOnAccentFillColorPrimary`.
    pub text_on_accent: D2D1_COLOR_F,
    /// `TextOnAccentFillColorSecondary` (accent button pressed).
    pub text_on_accent_secondary: D2D1_COLOR_F,
    /// `TextOnAccentFillColorDisabled`.
    pub text_on_accent_disabled: D2D1_COLOR_F,
    /// `FocusStrokeColorOuter` (2-DIP outer focus ring).
    pub focus_outer: D2D1_COLOR_F,
    /// `FocusStrokeColorInner` (1-DIP inner focus ring).
    pub focus_inner: D2D1_COLOR_F,
}

impl Theme {
    /// WinUI dark theme, default blue accent.
    pub const DARK: Theme = Theme {
        dark: true,
        accent: argb(0xFF60CDFF),
        text_primary: argb(0xFFFFFFFF),
        text_secondary: argb(0xC5FFFFFF),
        text_tertiary: argb(0x87FFFFFF),
        control_fill: argb(0x0FFFFFFF),
        control_fill_hover: argb(0x15FFFFFF),
        control_fill_pressed: argb(0x08FFFFFF),
        card: argb(0x0DFFFFFF),
        stroke: argb(0x12FFFFFF),
        subtle_hover: argb(0x0FFFFFFF),
        subtle_pressed: argb(0x0AFFFFFF),
        text_disabled: argb(0x5DFFFFFF),
        control_fill_disabled: argb(0x0BFFFFFF),
        stroke_secondary: argb(0x18FFFFFF),
        stroke_on_accent: argb(0x14FFFFFF),
        stroke_on_accent_secondary: argb(0x23000000),
        accent_disabled: argb(0x28FFFFFF),
        text_on_accent: argb(0xFF000000),
        text_on_accent_secondary: argb(0x80000000),
        text_on_accent_disabled: argb(0x87FFFFFF),
        focus_outer: argb(0xFFFFFFFF),
        focus_inner: argb(0xB3000000),
    };

    /// WinUI light theme, default blue accent.
    pub const LIGHT: Theme = Theme {
        dark: false,
        accent: argb(0xFF005FB8),
        text_primary: argb(0xE4000000),
        text_secondary: argb(0x9E000000),
        text_tertiary: argb(0x72000000),
        control_fill: argb(0xB3FFFFFF),
        control_fill_hover: argb(0x80F9F9F9),
        control_fill_pressed: argb(0x4DF9F9F9),
        card: argb(0xB3FFFFFF),
        stroke: argb(0x0F000000),
        subtle_hover: argb(0x09000000),
        subtle_pressed: argb(0x06000000),
        text_disabled: argb(0x5C000000),
        control_fill_disabled: argb(0x4DF9F9F9),
        stroke_secondary: argb(0x29000000),
        stroke_on_accent: argb(0x14FFFFFF),
        stroke_on_accent_secondary: argb(0x66000000),
        accent_disabled: argb(0x37000000),
        text_on_accent: argb(0xFFFFFFFF),
        text_on_accent_secondary: argb(0xB3FFFFFF),
        text_on_accent_disabled: argb(0xFFFFFFFF),
        focus_outer: argb(0xE4000000),
        focus_inner: argb(0xB3FFFFFF),
    };

    /// Current app theme and accent from `UISettings`, the documented source
    /// WinUI itself uses. Falls back to [`Theme::LIGHT`] if WinRT is unavailable.
    pub fn system() -> Theme {
        Self::from_ui_settings().unwrap_or(Self::LIGHT)
    }

    fn from_ui_settings() -> Result<Theme> {
        // No apartment setup needed: windows-core joins the MTA on CO_E_NOTINITIALIZED.
        let ui = UISettings::new()?;
        // Documented check: a light foreground means the dark theme is active.
        let dark = is_light(ui.GetColorValue(UIColorType::Foreground)?);
        let mut theme = if dark { Self::DARK } else { Self::LIGHT };
        // WinUI AccentFillColorDefault: SystemAccentColorLight2 (dark), SystemAccentColorDark1 (light).
        let shade = if dark { UIColorType::AccentLight2 } else { UIColorType::AccentDark1 };
        theme.accent = color(ui.GetColorValue(shade)?);
        Ok(theme)
    }
}

/// `IsColorLight` from "Apply Windows themes" (Learn): perceived brightness check.
fn is_light(c: Color) -> bool {
    5 * u32::from(c.G) + 2 * u32::from(c.R) + u32::from(c.B) > 8 * 128
}

fn color(c: Color) -> D2D1_COLOR_F {
    argb(u32::from_be_bytes([c.A, c.R, c.G, c.B]))
}

/// Typographic family used by WinUI (`XamlAutoFontFamily` resolves to it).
/// One variable font; the optical-size instance (Small/Text/Display) is picked
/// from the font size, as XAML does (see [`crate::gfx::Surface::format`]).
pub const FAMILY: PCWSTR = w!("Segoe UI Variable");

/// One step of the WinUI type ramp (`TextBlock_themeresources.xaml`).
///
/// WinUI styles set no `LineHeight`: lines use the font's own metrics
/// (`LineStackingStrategy=MaxHeight`), so neither does this crate.
#[derive(Clone, Copy, Debug)]
pub struct Ramp {
    /// Size in DIPs.
    pub size: f32,
    /// Weight (`wght` axis value: 400 regular, 600 semibold).
    pub weight: f32,
}

const REGULAR: f32 = DWRITE_FONT_WEIGHT_REGULAR.0 as f32;
const SEMIBOLD: f32 = DWRITE_FONT_WEIGHT_SEMI_BOLD.0 as f32;

/// `CaptionTextBlockStyle`: 12 regular (design line height 16).
pub const CAPTION: Ramp = Ramp { size: 12.0, weight: REGULAR };
/// `BodyTextBlockStyle`: 14 regular (design line height 20).
pub const BODY: Ramp = Ramp { size: 14.0, weight: REGULAR };
/// `BodyStrongTextBlockStyle`: 14 semibold (design line height 20).
pub const BODY_STRONG: Ramp = Ramp { size: 14.0, weight: SEMIBOLD };
/// `BodyLargeTextBlockStyle`: 18 regular (design line height 24).
pub const BODY_LARGE: Ramp = Ramp { size: 18.0, weight: REGULAR };
/// `SubtitleTextBlockStyle`: 20 semibold (design line height 28).
pub const SUBTITLE: Ramp = Ramp { size: 20.0, weight: SEMIBOLD };
/// `TitleTextBlockStyle`: 28 semibold (design line height 36).
pub const TITLE: Ramp = Ramp { size: 28.0, weight: SEMIBOLD };
/// `TitleLargeTextBlockStyle`: 40 semibold (design line height 52).
pub const TITLE_LARGE: Ramp = Ramp { size: 40.0, weight: SEMIBOLD };
/// `DisplayTextBlockStyle`: 68 semibold (design line height 92).
pub const DISPLAY: Ramp = Ramp { size: 68.0, weight: SEMIBOLD };

/// Every ramp step, smallest first.
pub const RAMP: [Ramp; 8] = [CAPTION, BODY, BODY_STRONG, BODY_LARGE, SUBTITLE, TITLE, TITLE_LARGE, DISPLAY];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_should_split_channels() {
        let c = argb(0x80FF0000);
        assert!((c.a - 128.0 / 255.0).abs() < 1e-6 && c.r == 1.0 && c.g == 0.0 && c.b == 0.0);
    }

    #[test]
    fn is_light_should_split_white_and_black_foregrounds() {
        let rgb = |v| Color { A: 255, R: v, G: v, B: v };
        assert!(is_light(rgb(255)) && !is_light(rgb(0)));
    }
}
