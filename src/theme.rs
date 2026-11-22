//! WinUI (Windows 11) design tokens and the system accent color.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_REGULAR, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD, RegGetValueW,
};
use windows::core::{PCWSTR, w};

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
    };

    /// Current user's app theme and accent, read from the registry.
    /// Falls back to [`Theme::LIGHT`] / default accent when keys are missing.
    pub fn system() -> Theme {
        let light = reg_dword(
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
        )
        .is_none_or(|v| v != 0);
        let mut theme = if light { Self::LIGHT } else { Self::DARK };
        if let Some(palette) = accent_palette() {
            // AccentPalette = Light3, Light2, Light1, Base, Dark1, Dark2, Dark3, unused.
            // WinUI AccentFillColorDefault: Light2 in dark theme, Dark1 in light theme.
            let [r, g, b, _] = palette[if light { 4 } else { 1 }];
            theme.accent = argb(0xFF00_0000 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b));
        }
        theme
    }
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

fn reg_dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut len = size_of::<u32>() as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key, name, RRF_RT_REG_DWORD, None,
            Some((&raw mut value).cast()), Some(&mut len))
    }
    .ok()
    .ok()?;
    Some(value)
}

fn accent_palette() -> Option<[[u8; 4]; 8]> {
    let mut palette = [[0u8; 4]; 8];
    let mut len = size_of_val(&palette) as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\Accent"),
            w!("AccentPalette"), RRF_RT_REG_BINARY, None,
            Some(palette.as_mut_ptr().cast()), Some(&mut len))
    }
    .ok()
    .ok()?;
    (len as usize == size_of_val(&palette)).then_some(palette)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argb_should_split_channels() {
        let c = argb(0x80FF0000);
        assert!((c.a - 128.0 / 255.0).abs() < 1e-6 && c.r == 1.0 && c.g == 0.0 && c.b == 0.0);
    }
}
