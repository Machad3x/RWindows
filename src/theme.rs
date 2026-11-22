//! WinUI (Windows 11) design tokens and the system accent color.

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_WEIGHT_REGULAR, DWRITE_FONT_WEIGHT_SEMI_BOLD,
};
use windows::Win32::Graphics::Gdi::{
    COLOR_BTNFACE, COLOR_BTNTEXT, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, GetSysColor,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
};
use windows::UI::Color;
use windows::UI::ViewManagement::{UIColorType, UISettings};
use windows::core::{PCWSTR, Result, w};

/// WinUI `ControlCornerRadius` (buttons, text boxes, list items).
pub const CONTROL_RADIUS: f32 = 4.0;
/// WinUI `OverlayCornerRadius` (cards, flyouts, dialogs, menus).
pub const OVERLAY_RADIUS: f32 = 8.0;

/// `SubtleFillColorTransparent` (same in dark and light).
pub const SUBTLE_TRANSPARENT: D2D1_COLOR_F = argb(0x00FFFFFF);

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
    /// `ControlAltFillColorSecondary` (ToggleSwitch track, off).
    pub control_alt_fill_secondary: D2D1_COLOR_F,
    /// `ControlAltFillColorTertiary` (track off, pointer over).
    pub control_alt_fill_tertiary: D2D1_COLOR_F,
    /// `ControlAltFillColorQuarternary` (track off, pressed).
    pub control_alt_fill_quarternary: D2D1_COLOR_F,
    /// `ControlAltFillColorDisabled`.
    pub control_alt_fill_disabled: D2D1_COLOR_F,
    /// `ControlStrongStrokeColorDefault` (track outline, off).
    pub control_strong_stroke: D2D1_COLOR_F,
    /// `ControlStrongStrokeColorDisabled`.
    pub control_strong_stroke_disabled: D2D1_COLOR_F,
    /// The user's high contrast colors, when high contrast is on. Controls then
    /// use their `HighContrast` theme dictionary instead of the tokens above.
    pub high_contrast: Option<SystemColors>,
}

/// The `GetSysColor` colors WinUI's `HighContrast` dictionaries are built from
/// (`SystemColor*Color` resources).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SystemColors {
    /// `COLOR_WINDOW`: page background.
    pub window: D2D1_COLOR_F,
    /// `COLOR_WINDOWTEXT`: text on the page.
    pub window_text: D2D1_COLOR_F,
    /// `COLOR_BTNFACE`: button background.
    pub button_face: D2D1_COLOR_F,
    /// `COLOR_BTNTEXT`: button text and border.
    pub button_text: D2D1_COLOR_F,
    /// `COLOR_HIGHLIGHT`: hovered / selected background.
    pub highlight: D2D1_COLOR_F,
    /// `COLOR_HIGHLIGHTTEXT`: text on `highlight`.
    pub highlight_text: D2D1_COLOR_F,
    /// `COLOR_GRAYTEXT`: disabled text.
    pub gray_text: D2D1_COLOR_F,
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
        control_alt_fill_secondary: argb(0x19000000),
        control_alt_fill_tertiary: argb(0x0BFFFFFF),
        control_alt_fill_quarternary: argb(0x12FFFFFF),
        control_alt_fill_disabled: argb(0x00FFFFFF),
        control_strong_stroke: argb(0x8BFFFFFF),
        control_strong_stroke_disabled: argb(0x28FFFFFF),
        high_contrast: None,
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
        control_alt_fill_secondary: argb(0x06000000),
        control_alt_fill_tertiary: argb(0x0F000000),
        control_alt_fill_quarternary: argb(0x18000000),
        control_alt_fill_disabled: argb(0x00FFFFFF),
        control_strong_stroke: argb(0x72000000),
        control_strong_stroke_disabled: argb(0x37000000),
        high_contrast: None,
    };

    /// WinUI `HighContrast` theme (`Common_themeresources_any.xaml`) for `c`.
    pub fn high_contrast(c: SystemColors) -> Theme {
        Theme {
            dark: !is_light_d2d(c.window),
            accent: c.window,
            text_primary: c.window_text,
            text_secondary: c.window_text,
            text_tertiary: c.window_text,
            control_fill: c.button_face,
            control_fill_hover: c.button_face,
            control_fill_pressed: c.button_face,
            card: c.window,
            stroke: c.button_text,
            subtle_hover: c.button_face,
            subtle_pressed: c.button_face,
            text_disabled: c.gray_text,
            control_fill_disabled: c.button_face,
            stroke_secondary: c.button_text,
            stroke_on_accent: c.button_text,
            stroke_on_accent_secondary: c.button_text,
            accent_disabled: c.window,
            text_on_accent: c.window_text,
            text_on_accent_secondary: c.window_text,
            text_on_accent_disabled: c.gray_text,
            focus_outer: c.window_text,
            focus_inner: c.window,
            control_alt_fill_secondary: c.button_face,
            control_alt_fill_tertiary: c.button_face,
            control_alt_fill_quarternary: c.button_face,
            control_alt_fill_disabled: c.button_face,
            control_strong_stroke: c.button_text,
            control_strong_stroke_disabled: c.button_text,
            high_contrast: Some(c),
        }
    }

    /// Current theme: [`Theme::high_contrast`] with the user's colors when high
    /// contrast is on; otherwise theme and accent from `UISettings`, the documented
    /// source WinUI itself uses (falls back to [`Theme::LIGHT`] if WinRT is unavailable).
    pub fn system() -> Theme {
        if high_contrast_on() {
            return Self::high_contrast(SystemColors::current());
        }
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

/// [`is_light`] for a Direct2D color.
fn is_light_d2d(c: D2D1_COLOR_F) -> bool {
    let byte = |v: f32| (v * 255.0).round() as u8;
    is_light(Color { A: 255, R: byte(c.r), G: byte(c.g), B: byte(c.b) })
}

/// A `GetSysColor` `COLORREF` (`0x00BBGGRR`) as an opaque color.
fn colorref(v: u32) -> D2D1_COLOR_F {
    let [_, b, g, r] = v.to_be_bytes();
    argb(u32::from_be_bytes([0xFF, r, g, b]))
}

impl SystemColors {
    /// The user's current colors, from `GetSysColor`.
    pub fn current() -> SystemColors {
        let get = |i| colorref(unsafe { GetSysColor(i) });
        SystemColors {
            window: get(COLOR_WINDOW),
            window_text: get(COLOR_WINDOWTEXT),
            button_face: get(COLOR_BTNFACE),
            button_text: get(COLOR_BTNTEXT),
            highlight: get(COLOR_HIGHLIGHT),
            highlight_text: get(COLOR_HIGHLIGHTTEXT),
            gray_text: get(COLOR_GRAYTEXT),
        }
    }
}

/// High contrast parameter (Learn): `SPI_GETHIGHCONTRAST` with `HCF_HIGHCONTRASTON`.
fn high_contrast_on() -> bool {
    let mut hc = HIGHCONTRASTW { cbSize: size_of::<HIGHCONTRASTW>() as u32, ..Default::default() };
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, Some((&raw mut hc).cast()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0))
    };
    ok.is_ok() && hc.dwFlags.contains(HCF_HIGHCONTRASTON)
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
pub(crate) mod tests {
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

    /// Distinct colors so each assertion names exactly one system color.
    pub(crate) const HC: SystemColors = SystemColors {
        window: argb(0xFF000001),
        window_text: argb(0xFFFFFF02),
        button_face: argb(0xFF000003),
        button_text: argb(0xFFFFFF04),
        highlight: argb(0xFF1AEBFF),
        highlight_text: argb(0xFF000006),
        gray_text: argb(0xFF3FF207),
    };

    #[test]
    fn colorref_should_read_blue_green_red() {
        assert_eq!(colorref(0x0033_2211), argb(0xFF11_2233));
    }

    #[test]
    fn high_contrast_text_should_use_window_text_and_gray_text() {
        let t = Theme::high_contrast(HC);
        assert_eq!((t.text_primary, t.text_secondary, t.text_disabled), (HC.window_text, HC.window_text, HC.gray_text));
    }

    #[test]
    fn high_contrast_focus_should_use_window_text_outside_and_window_inside() {
        let t = Theme::high_contrast(HC);
        assert_eq!((t.focus_outer, t.focus_inner), (HC.window_text, HC.window));
    }

    #[test]
    fn high_contrast_should_be_dark_when_the_window_is_dark() {
        let light = SystemColors { window: argb(0xFFFFFFFF), ..HC };
        assert_eq!((Theme::high_contrast(HC).dark, Theme::high_contrast(light).dark), (true, false));
    }
}
