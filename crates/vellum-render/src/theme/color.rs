// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An sRGB colour and WCAG contrast arithmetic.

use std::fmt;
use std::str::FromStr;

use material_colors::color::Rgb;
use material_colors::hct::Hct;
use material_colors::palette::TonalPalette;

/// An opaque sRGB colour, written `#rrggbb`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Color {
    /// Red channel.
    pub r: u8,
    /// Green channel.
    pub g: u8,
    /// Blue channel.
    pub b: u8,
}

impl Color {
    /// Builds a colour from its channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Builds a colour from `0xRRGGBB`.
    #[must_use]
    pub const fn from_u32(v: u32) -> Self {
        Self {
            r: ((v >> 16) & 0xff) as u8,
            g: ((v >> 8) & 0xff) as u8,
            b: (v & 0xff) as u8,
        }
    }

    /// WCAG 2.2 relative luminance, 0 (black) to 1 (white).
    #[must_use]
    pub fn luminance(self) -> f64 {
        fn channel(c: u8) -> f64 {
            let c = f64::from(c) / 255.0;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        }
        0.0722_f64.mul_add(
            channel(self.b),
            0.2126_f64.mul_add(channel(self.r), 0.7152 * channel(self.g)),
        )
    }

    /// WCAG 2.2 contrast ratio between two colours, 1 to 21.
    #[must_use]
    pub fn contrast(self, other: Self) -> f64 {
        let (a, b) = (self.luminance(), other.luminance());
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// The HCT hue of this colour, in degrees.
    #[must_use]
    pub fn hue(self) -> f64 {
        Hct::new(self.into()).get_hue()
    }
}

impl From<Rgb> for Color {
    fn from(c: Rgb) -> Self {
        Self::new(c.red, c.green, c.blue)
    }
}

impl From<Color> for Rgb {
    fn from(c: Color) -> Self {
        Self::new(c.r, c.g, c.b)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

/// Why a colour string could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a #rrggbb colour: {0:?}")]
pub struct ParseColorError(pub String);

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseColorError(s.to_owned());
        let hex = s.strip_prefix('#').ok_or_else(err)?;
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(err());
        }
        u32::from_str_radix(hex, 16)
            .map(Self::from_u32)
            .map_err(|_| err())
    }
}

impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(
        d: D,
    ) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// A tone of `palette` that reaches `ratio` against `bg`, searching from
/// `start` towards black on light backgrounds and towards white on dark ones.
/// Falls back to pure black or white, which always reach 4.5:1 on one side.
#[must_use]
pub fn tone_for_contrast(
    palette: &TonalPalette,
    start: i32,
    bg: Color,
    ratio: f64,
) -> Color {
    let towards_dark = bg.luminance() > 0.18;
    let mut tone = start.clamp(0, 100);
    loop {
        let c: Color = palette.tone(tone).into();
        if c.contrast(bg) >= ratio {
            return c;
        }
        if towards_dark {
            if tone == 0 {
                return Color::new(0, 0, 0);
            }
            tone -= 1;
        } else {
            if tone == 100 {
                return Color::new(255, 255, 255);
            }
            tone += 1;
        }
    }
}

/// A palette for `hue` and `chroma`.
#[must_use]
pub fn palette(hue: f64, chroma: f64) -> TonalPalette {
    TonalPalette::of(hue, chroma)
}

/// One tone of a palette.
#[must_use]
pub fn tone(palette: &TonalPalette, tone: i32) -> Color {
    palette.tone(tone).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints() {
        let c: Color = "#4F5B92".parse().unwrap_or(Color::new(0, 0, 0));
        assert_eq!(c, Color::new(0x4f, 0x5b, 0x92));
        assert_eq!(c.to_string(), "#4f5b92");
        assert!("4f5b92".parse::<Color>().is_err());
        assert!("#4f5b9".parse::<Color>().is_err());
        assert!("#4f5b9g".parse::<Color>().is_err());
    }

    #[test]
    fn contrast_matches_wcag_reference_values() {
        let black = Color::new(0, 0, 0);
        let white = Color::new(255, 255, 255);
        assert!((black.contrast(white) - 21.0).abs() < 1e-9);
        assert!((white.contrast(white) - 1.0).abs() < 1e-9);
        // #767676 on white is the classic 4.54:1 grey.
        let grey = Color::new(0x76, 0x76, 0x76);
        assert!((grey.contrast(white) - 4.54).abs() < 0.01);
    }

    #[test]
    fn tone_search_reaches_the_ratio() {
        let p = palette(260.0, 40.0);
        for bg in [Color::new(255, 255, 255), Color::new(0x12, 0x12, 0x14)] {
            for ratio in [3.0, 4.5, 7.0] {
                assert!(
                    tone_for_contrast(&p, 50, bg, ratio).contrast(bg) >= ratio
                );
            }
        }
    }
}
