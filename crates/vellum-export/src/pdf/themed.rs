// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Print themes made from Vellum themes (R-TH-4, ADR 0007 D245).
//!
//! The organisation picks an export theme apart from anyone's app theme.
//! Only light themes print: paper is white, so a dark theme's light text
//! would vanish.

use vellum_render::theme::{BuiltinTheme, Color, Theme};

use super::{Colors, PrintTheme, ToneColors};
use crate::flow::Tone;

const TONES: [(Tone, &str); 6] = [
    (Tone::Info, "info"),
    (Tone::Warn, "warn"),
    (Tone::Good, "good"),
    (Tone::Crit, "crit"),
    (Tone::Tip, "tip"),
    (Tone::Note, "note"),
];

fn hex(c: Color) -> String {
    c.to_string().to_uppercase()
}

impl PrintTheme {
    /// The print theme for an export theme: the default page and fonts with
    /// the theme's colours. A dark theme falls back to its light sibling's
    /// colours (Light for Dark and Dim), because the page is white.
    #[must_use]
    pub fn from_builtin(which: BuiltinTheme, seed: Color) -> Self {
        let printable = if which.is_light() {
            which
        } else {
            BuiltinTheme::Light
        };
        Self::from_theme(&Theme::from_seed(printable, seed))
    }

    /// The print theme with `theme`'s colours. Use a light theme; see
    /// [`Self::from_builtin`].
    #[must_use]
    pub fn from_theme(theme: &Theme) -> Self {
        let get = |token: &str| hex(theme.get(token));
        let base = Self::default();
        Self {
            colors: Colors {
                text: get("on-surface"),
                heading: get("on-surface"),
                muted: get("on-surface-variant"),
                primary: get("primary"),
                outline: get("outline-variant"),
                surface: get("surface-container"),
            },
            tones: TONES
                .into_iter()
                .map(|(tone, name)| {
                    (
                        tone,
                        ToneColors {
                            fg: get(name),
                            bg: get(&format!("{name}-container")),
                            label: tone.label().to_owned(),
                        },
                    )
                })
                .collect(),
            ..base
        }
    }
}

#[cfg(test)]
mod tests {
    use vellum_render::theme::DEFAULT_SEED;

    use super::*;

    fn color(s: &str) -> Color {
        s.parse().unwrap_or_default()
    }

    #[test]
    fn every_export_theme_prints_readable_text_on_white() {
        let white = Color::from_u32(0xFF_FFFF);
        for which in BuiltinTheme::ALL {
            let print = PrintTheme::from_builtin(which, DEFAULT_SEED);
            for c in [
                &print.colors.text,
                &print.colors.muted,
                &print.colors.primary,
            ] {
                assert!(
                    color(c).contrast(white) >= 4.5,
                    "{} {c} on white",
                    which.id()
                );
            }
            assert_eq!(print.tones.len(), 6, "{}", which.id());
            for (tone, colors) in &print.tones {
                assert!(
                    color(&colors.fg).contrast(color(&colors.bg)) >= 3.0,
                    "{} {tone:?}",
                    which.id()
                );
            }
        }
    }

    #[test]
    fn dark_themes_print_with_light_colours() {
        let dark = PrintTheme::from_builtin(BuiltinTheme::Dark, DEFAULT_SEED);
        let light = PrintTheme::from_builtin(BuiltinTheme::Light, DEFAULT_SEED);
        assert_eq!(dark, light);
    }

    #[test]
    fn keeps_the_default_page_and_follows_the_seed() {
        let red = PrintTheme::from_builtin(BuiltinTheme::Light, Color::from_u32(0xB0_2020));
        let base = PrintTheme::default();
        assert_eq!(red.paper, base.paper);
        assert_eq!(red.fonts, base.fonts);
        assert_ne!(red.colors.primary, base.colors.primary);
        let high = PrintTheme::from_builtin(BuiltinTheme::HighContrast, DEFAULT_SEED);
        assert!(color(&high.colors.text).contrast(Color::from_u32(0xFF_FFFF)) >= 7.0);
    }
}
