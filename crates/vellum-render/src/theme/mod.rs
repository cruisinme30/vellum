// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Material 3 themes as design tokens (R-TH-1 to R-TH-5).
//!
//! A [`Theme`] is a named set of colour tokens, each exposed to CSS as
//! `--vl-<token>`. Rendered HTML never names a colour: it uses classes whose
//! styles read these variables, so one rendered page serves every theme and
//! a theme switch is a CSS change. Charts and diagrams use the same
//! variables, so they re-theme with the page (R-TH-3); [`Theme::resolve_vars`]
//! bakes the colours in for standalone exports.
//!
//! Every built-in theme is derived from a seed colour with Material 3
//! dynamic colour; Vellum's own tokens (callouts, charts, syntax) are solved
//! for WCAG contrast against the surfaces they sit on, and
//! [`Theme::validate`] checks the result (R-TH-2).

mod color;

use std::collections::BTreeMap;
use std::fmt::Write as _;

use material_colors::blend::harmonize;
use material_colors::dynamic_color::{DynamicScheme, Variant};
use material_colors::palette::TonalPalette;
use material_colors::scheme::Scheme;

pub use color::{Color, ParseColorError};
use color::{palette, tone, tone_for_contrast};

/// Vellum's default seed colour, the primary of the UI (brief §13).
pub const DEFAULT_SEED: Color = Color::from_u32(0x4F_5B92);

/// Light or dark: decides `color-scheme` and how tokens are derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Dark text on light surfaces.
    Light,
    /// Light text on dark surfaces.
    Dark,
}

/// The five built-in themes (R-TH-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinTheme {
    /// Material 3 light.
    Light,
    /// Material 3 dark.
    Dark,
    /// Light with every text pair at 7:1 or more.
    HighContrast,
    /// Warm paper surfaces for long reading.
    Sepia,
    /// Dark with lifted surfaces and softer text, for low light.
    Dim,
}

impl BuiltinTheme {
    /// Every built-in theme, in menu order.
    pub const ALL: [Self; 5] = [
        Self::Light,
        Self::Dark,
        Self::HighContrast,
        Self::Sepia,
        Self::Dim,
    ];

    /// The theme's id, used in `data-vl-theme` and file names.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::HighContrast => "high-contrast",
            Self::Sepia => "sepia",
            Self::Dim => "dim",
        }
    }

    /// The built-in theme with this [`id`](Self::id), if any.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// Whether the theme puts dark text on light surfaces, so it can be
    /// printed on white paper (export themes, R-TH-4).
    #[must_use]
    pub const fn is_light(self) -> bool {
        matches!(self.mode(), Mode::Light)
    }

    const fn mode(self) -> Mode {
        match self {
            Self::Light | Self::HighContrast | Self::Sepia => Mode::Light,
            Self::Dark | Self::Dim => Mode::Dark,
        }
    }

    /// The minimum contrast for text pairs in this theme.
    const fn text_ratio(self) -> f64 {
        match self {
            Self::HighContrast => 7.0,
            _ => 4.5,
        }
    }
}

/// The colour tokens every theme defines, in CSS order.
pub const TOKENS: &[&str] = &[
    // Material 3 roles.
    "primary",
    "on-primary",
    "primary-container",
    "on-primary-container",
    "secondary",
    "on-secondary",
    "secondary-container",
    "on-secondary-container",
    "tertiary",
    "on-tertiary",
    "tertiary-container",
    "on-tertiary-container",
    "error",
    "on-error",
    "error-container",
    "on-error-container",
    "surface",
    "on-surface",
    "surface-variant",
    "on-surface-variant",
    "surface-container-lowest",
    "surface-container-low",
    "surface-container",
    "surface-container-high",
    "surface-container-highest",
    "outline",
    "outline-variant",
    "inverse-surface",
    "inverse-on-surface",
    "inverse-primary",
    "scrim",
    "shadow",
    // Links.
    "link",
    // Callouts and chips: container, text on it, and accent (icon, border).
    "info-container",
    "on-info-container",
    "info",
    "warn-container",
    "on-warn-container",
    "warn",
    "good-container",
    "on-good-container",
    "good",
    "crit-container",
    "on-crit-container",
    "crit",
    "tip-container",
    "on-tip-container",
    "tip",
    "note-container",
    "on-note-container",
    "note",
    // Highlight (`==mark==`).
    "highlight",
    "on-highlight",
    // Charts: a categorical palette, at least 3:1 against the surface.
    "chart-1",
    "chart-2",
    "chart-3",
    "chart-4",
    "chart-5",
    "chart-6",
    "chart-7",
    "chart-8",
    // Code blocks and syntax colours.
    "code-bg",
    "code-fg",
    "syntax-keyword",
    "syntax-string",
    "syntax-comment",
    "syntax-number",
    "syntax-function",
    "syntax-type",
    "syntax-variable",
    "syntax-punctuation",
    "syntax-tag",
    "syntax-attribute",
];

/// A foreground/background pair and the contrast it must reach.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(clippy::derive_partial_eq_without_eq, reason = "holds an f64 ratio")]
pub struct ContrastRule {
    /// Foreground token.
    pub fg: &'static str,
    /// Background token.
    pub bg: &'static str,
    /// True for text (4.5:1, or 7:1 in high contrast); false for graphics
    /// and borders (3:1).
    pub text: bool,
}

const fn rule(fg: &'static str, bg: &'static str, text: bool) -> ContrastRule {
    ContrastRule { fg, bg, text }
}

/// Every pair [`Theme::validate`] checks.
pub const CONTRAST_RULES: &[ContrastRule] = &[
    rule("on-surface", "surface", true),
    rule("on-surface", "surface-container-lowest", true),
    rule("on-surface", "surface-container-low", true),
    rule("on-surface", "surface-container", true),
    rule("on-surface", "surface-container-high", true),
    rule("on-surface", "surface-container-highest", true),
    rule("on-surface-variant", "surface", true),
    rule("on-surface-variant", "surface-container", true),
    rule("on-primary", "primary", true),
    rule("on-primary-container", "primary-container", true),
    rule("on-secondary", "secondary", true),
    rule("on-secondary-container", "secondary-container", true),
    rule("on-tertiary", "tertiary", true),
    rule("on-tertiary-container", "tertiary-container", true),
    rule("on-error", "error", true),
    rule("on-error-container", "error-container", true),
    rule("inverse-on-surface", "inverse-surface", true),
    rule("link", "surface", true),
    rule("on-info-container", "info-container", true),
    rule("on-warn-container", "warn-container", true),
    rule("on-good-container", "good-container", true),
    rule("on-crit-container", "crit-container", true),
    rule("on-tip-container", "tip-container", true),
    rule("on-note-container", "note-container", true),
    rule("on-highlight", "highlight", true),
    rule("info", "surface", false),
    rule("warn", "surface", false),
    rule("good", "surface", false),
    rule("crit", "surface", false),
    rule("tip", "surface", false),
    rule("note", "surface", false),
    rule("outline", "surface", false),
    rule("chart-1", "surface", false),
    rule("chart-2", "surface", false),
    rule("chart-3", "surface", false),
    rule("chart-4", "surface", false),
    rule("chart-5", "surface", false),
    rule("chart-6", "surface", false),
    rule("chart-7", "surface", false),
    rule("chart-8", "surface", false),
    rule("code-fg", "code-bg", true),
    rule("syntax-keyword", "code-bg", true),
    rule("syntax-string", "code-bg", true),
    rule("syntax-comment", "code-bg", true),
    rule("syntax-number", "code-bg", true),
    rule("syntax-function", "code-bg", true),
    rule("syntax-type", "code-bg", true),
    rule("syntax-variable", "code-bg", true),
    rule("syntax-punctuation", "code-bg", true),
    rule("syntax-tag", "code-bg", true),
    rule("syntax-attribute", "code-bg", true),
];

/// A pair that misses its contrast target.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ContrastIssue {
    /// Foreground token.
    pub fg: String,
    /// Background token.
    pub bg: String,
    /// The measured ratio.
    pub ratio: f64,
    /// The ratio required.
    pub required: f64,
}

/// Why a theme file was rejected.
#[derive(Debug, thiserror::Error)]
pub enum ThemeError {
    /// The JSON is malformed or a colour is not `#rrggbb`.
    #[error("invalid theme file: {0}")]
    Json(#[from] serde_json::Error),
    /// A required token is missing.
    #[error("the theme has no `{0}` token")]
    MissingToken(String),
    /// A token is not one Vellum knows.
    #[error("unknown token `{0}`")]
    UnknownToken(String),
    /// The name is empty or too long.
    #[error("a theme name must be 1 to 64 characters")]
    BadName,
    /// Some pairs miss their contrast target.
    #[error("{} colour pairs miss their contrast target", .0.len())]
    Contrast(Vec<ContrastIssue>),
}

/// A complete, validated set of colour tokens.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// Display name, such as `Light` or `Northstar brand`.
    pub name: String,
    /// Light or dark.
    pub mode: Mode,
    /// The seed colour the theme was derived from.
    pub seed: Color,
    /// True when text pairs must reach 7:1 rather than 4.5:1.
    #[serde(default)]
    pub high_contrast: bool,
    /// Every token in [`TOKENS`].
    pub tokens: BTreeMap<String, Color>,
}

impl Theme {
    /// A built-in theme from Vellum's default seed.
    #[must_use]
    pub fn builtin(which: BuiltinTheme) -> Self {
        Self::from_seed(which, DEFAULT_SEED)
    }

    /// A built-in theme recoloured from an organisation's seed (R-TH-1).
    #[must_use]
    pub fn from_seed(which: BuiltinTheme, seed: Color) -> Self {
        let seed = if which == BuiltinTheme::Sepia {
            // Sepia keeps the seed's hue for accents but sits on paper.
            Color::from(harmonize(Color::from_u32(0x8B_5E34).into(), seed.into()))
        } else {
            seed
        };
        let dark = which.mode() == Mode::Dark;
        let contrast = if which == BuiltinTheme::HighContrast {
            1.0
        } else {
            0.0
        };
        let dynamic =
            DynamicScheme::by_variant(seed.into(), &Variant::TonalSpot, dark, Some(contrast));
        let neutral = dynamic.neutral_palette;
        let neutral_variant = dynamic.neutral_variant_palette;
        let primary_palette = dynamic.primary_palette;
        let scheme = Scheme::from(dynamic);
        let mut tokens = m3_tokens(&scheme);
        match which {
            BuiltinTheme::Sepia => sepia_surfaces(&mut tokens, seed),
            BuiltinTheme::Dim => dim_surfaces(&mut tokens, &neutral, &neutral_variant),
            _ => {}
        }
        let mut builder = Derive {
            tokens,
            seed,
            dark,
            text_ratio: which.text_ratio(),
        };
        builder.derive(&primary_palette);
        Self {
            name: builtin_name(which).to_owned(),
            mode: which.mode(),
            seed,
            high_contrast: which == BuiltinTheme::HighContrast,
            tokens: builder.tokens,
        }
    }

    /// Reads a theme file (R-TH-2) and checks it completely: every token
    /// present and known, and every contrast rule met.
    ///
    /// # Errors
    ///
    /// Returns [`ThemeError`] naming the first structural problem, or every
    /// contrast issue at once.
    pub fn from_json(json: &str) -> Result<Self, ThemeError> {
        let theme: Self = serde_json::from_str(json)?;
        if theme.name.trim().is_empty() || theme.name.chars().count() > 64 {
            return Err(ThemeError::BadName);
        }
        if let Some(unknown) = theme.tokens.keys().find(|k| !TOKENS.contains(&k.as_str())) {
            return Err(ThemeError::UnknownToken(unknown.clone()));
        }
        if let Some(missing) = TOKENS.iter().find(|t| !theme.tokens.contains_key(**t)) {
            return Err(ThemeError::MissingToken((*missing).to_owned()));
        }
        let issues = theme.validate();
        if issues.is_empty() {
            Ok(theme)
        } else {
            Err(ThemeError::Contrast(issues))
        }
    }

    /// Writes the theme as pretty JSON, the format [`Self::from_json`] reads.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// A token's colour; black when the token is unknown.
    #[must_use]
    pub fn get(&self, token: &str) -> Color {
        self.tokens.get(token).copied().unwrap_or_default()
    }

    /// Every contrast rule this theme misses.
    #[must_use]
    pub fn validate(&self) -> Vec<ContrastIssue> {
        let text_ratio = if self.high_contrast { 7.0 } else { 4.5 };
        CONTRAST_RULES
            .iter()
            .filter_map(|r| {
                let required = if r.text { text_ratio } else { 3.0 };
                let ratio = self.get(r.fg).contrast(self.get(r.bg));
                // Compare at two decimals, as contrast checkers report.
                (ratio + 0.005 < required).then(|| ContrastIssue {
                    fg: r.fg.to_owned(),
                    bg: r.bg.to_owned(),
                    ratio: (ratio * 100.0).round() / 100.0,
                    required,
                })
            })
            .collect()
    }

    /// The CSS custom property declarations, one per line.
    #[must_use]
    pub fn css_declarations(&self) -> String {
        let mut out = String::new();
        let scheme = match self.mode {
            Mode::Light => "light",
            Mode::Dark => "dark",
        };
        let _ = writeln!(out, "  color-scheme: {scheme};");
        for token in TOKENS {
            let _ = writeln!(out, "  --vl-{token}: {};", self.get(token));
        }
        out
    }

    /// A CSS rule that applies the theme to `selector`.
    #[must_use]
    pub fn css(&self, selector: &str) -> String {
        format!("{selector} {{\n{}}}\n", self.css_declarations())
    }

    /// Replaces every `var(--vl-<token>)` in `s` with the colour, for
    /// standalone SVG and print exports that cannot load the page CSS.
    #[must_use]
    pub fn resolve_vars(&self, s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut rest = s;
        while let Some(start) = rest.find("var(--vl-") {
            let (head, tail) = rest.split_at(start);
            out.push_str(head);
            let after = tail.get("var(--vl-".len()..).unwrap_or_default();
            match after.find(')') {
                Some(end) => {
                    let name = after.get(..end).unwrap_or_default();
                    match self.tokens.get(name) {
                        Some(c) => out.push_str(&c.to_string()),
                        None => out.push_str(
                            tail.get(..("var(--vl-".len() + end + 1))
                                .unwrap_or_default(),
                        ),
                    }
                    rest = after.get(end + 1..).unwrap_or_default();
                }
                None => {
                    out.push_str(tail);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
        out
    }
}

const fn builtin_name(which: BuiltinTheme) -> &'static str {
    match which {
        BuiltinTheme::Light => "Light",
        BuiltinTheme::Dark => "Dark",
        BuiltinTheme::HighContrast => "High contrast",
        BuiltinTheme::Sepia => "Sepia",
        BuiltinTheme::Dim => "Dim",
    }
}

fn m3_tokens(s: &Scheme) -> BTreeMap<String, Color> {
    let pairs: [(&str, material_colors::color::Rgb); 32] = [
        ("primary", s.primary),
        ("on-primary", s.on_primary),
        ("primary-container", s.primary_container),
        ("on-primary-container", s.on_primary_container),
        ("secondary", s.secondary),
        ("on-secondary", s.on_secondary),
        ("secondary-container", s.secondary_container),
        ("on-secondary-container", s.on_secondary_container),
        ("tertiary", s.tertiary),
        ("on-tertiary", s.on_tertiary),
        ("tertiary-container", s.tertiary_container),
        ("on-tertiary-container", s.on_tertiary_container),
        ("error", s.error),
        ("on-error", s.on_error),
        ("error-container", s.error_container),
        ("on-error-container", s.on_error_container),
        ("surface", s.surface),
        ("on-surface", s.on_surface),
        ("surface-variant", s.surface_variant),
        ("on-surface-variant", s.on_surface_variant),
        ("surface-container-lowest", s.surface_container_lowest),
        ("surface-container-low", s.surface_container_low),
        ("surface-container", s.surface_container),
        ("surface-container-high", s.surface_container_high),
        ("surface-container-highest", s.surface_container_highest),
        ("outline", s.outline),
        ("outline-variant", s.outline_variant),
        ("inverse-surface", s.inverse_surface),
        ("inverse-on-surface", s.inverse_on_surface),
        ("inverse-primary", s.inverse_primary),
        ("scrim", s.scrim),
        ("shadow", s.shadow),
    ];
    pairs
        .into_iter()
        .map(|(k, v)| (k.to_owned(), Color::from(v)))
        .collect()
}

fn set(tokens: &mut BTreeMap<String, Color>, key: &str, c: Color) {
    let _ = tokens.insert(key.to_owned(), c);
}

/// Warm paper surfaces around hue 75.
fn sepia_surfaces(tokens: &mut BTreeMap<String, Color>, seed: Color) {
    let paper = palette(75.0, 14.0);
    let ink = palette(seed.hue(), 10.0);
    for (key, t) in [
        ("surface", 94),
        ("surface-container-lowest", 97),
        ("surface-container-low", 95),
        ("surface-container", 92),
        ("surface-container-high", 90),
        ("surface-container-highest", 88),
        ("surface-variant", 88),
        ("outline-variant", 78),
    ] {
        set(tokens, key, tone(&paper, t));
    }
    set(tokens, "on-surface", tone(&ink, 12));
    set(tokens, "on-surface-variant", tone(&ink, 30));
    set(tokens, "outline", tone(&paper, 48));
}

/// Lifted dark surfaces and slightly softer text.
fn dim_surfaces(
    tokens: &mut BTreeMap<String, Color>,
    neutral: &TonalPalette,
    variant: &TonalPalette,
) {
    for (key, t) in [
        ("surface", 14),
        ("surface-container-lowest", 10),
        ("surface-container-low", 16),
        ("surface-container", 18),
        ("surface-container-high", 22),
        ("surface-container-highest", 26),
    ] {
        set(tokens, key, tone(neutral, t));
    }
    set(tokens, "surface-variant", tone(variant, 28));
    set(tokens, "on-surface", tone(neutral, 88));
    set(tokens, "on-surface-variant", tone(variant, 78));
}

/// Derives Vellum's own tokens from the Material 3 roles.
struct Derive {
    tokens: BTreeMap<String, Color>,
    seed: Color,
    dark: bool,
    text_ratio: f64,
}

impl Derive {
    fn get(&self, key: &str) -> Color {
        self.tokens.get(key).copied().unwrap_or_default()
    }

    /// A hue moved part of the way towards the seed, as M3 harmonises
    /// custom colours.
    fn hue(&self, hue: f64) -> f64 {
        let fixed = TonalPalette::of(hue, 48.0).tone(50);
        Color::from(harmonize(fixed, self.seed.into())).hue()
    }

    fn derive(&mut self, primary: &TonalPalette) {
        let surface = self.get("surface");
        let text = self.text_ratio;
        let (accent_start, text_start) = if self.dark { (75, 85) } else { (45, 30) };

        // Links are primary-coloured but must read as text on the surface.
        let link = tone_for_contrast(primary, if self.dark { 80 } else { 40 }, surface, text);
        set(&mut self.tokens, "link", link);

        // Callouts and chips.
        let note_hue = self.seed.hue();
        for (name, hue, chroma) in [
            ("info", self.hue(260.0), 36.0),
            ("warn", self.hue(75.0), 40.0),
            ("good", self.hue(145.0), 36.0),
            ("crit", self.hue(25.0), 40.0),
            ("tip", self.hue(300.0), 32.0),
            ("note", note_hue, 6.0),
        ] {
            let p = palette(hue, chroma);
            let bg = tone(&p, if self.dark { 22 } else { 93 });
            set(&mut self.tokens, &format!("{name}-container"), bg);
            let fg = tone_for_contrast(&p, text_start, bg, text);
            set(&mut self.tokens, &format!("on-{name}-container"), fg);
            let accent = tone_for_contrast(&p, accent_start, surface, 3.0);
            set(&mut self.tokens, name, accent);
        }

        let mark = palette(self.hue(90.0), 40.0);
        let mark_bg = tone(&mark, if self.dark { 35 } else { 90 });
        set(&mut self.tokens, "highlight", mark_bg);
        let mark_fg = tone_for_contrast(
            &palette(note_hue, 4.0),
            if self.dark { 95 } else { 10 },
            mark_bg,
            text,
        );
        set(&mut self.tokens, "on-highlight", mark_fg);

        // A categorical chart palette: distinct hues, at least 3:1.
        let chart_start = if self.dark { 70 } else { 50 };
        for (i, hue) in [260.0, 35.0, 150.0, 330.0, 85.0, 200.0, 10.0, 290.0]
            .into_iter()
            .enumerate()
        {
            let p = palette(self.hue(hue), 48.0);
            let c = tone_for_contrast(&p, chart_start, surface, 3.0);
            set(&mut self.tokens, &format!("chart-{}", i + 1), c);
        }

        // Code: syntax colours solved against the code background.
        let code_bg = self.get("surface-container");
        set(&mut self.tokens, "code-bg", code_bg);
        let code_fg = self.get("on-surface");
        set(&mut self.tokens, "code-fg", code_fg);
        let syntax_start = if self.dark { 80 } else { 40 };
        let neutral = palette(note_hue, 8.0);
        for (name, hue, chroma) in [
            ("keyword", self.hue(290.0), 48.0),
            ("string", self.hue(145.0), 40.0),
            ("comment", note_hue, 8.0),
            ("number", self.hue(40.0), 48.0),
            ("function", self.hue(250.0), 48.0),
            ("type", self.hue(190.0), 40.0),
            ("tag", self.hue(20.0), 48.0),
            ("attribute", self.hue(70.0), 44.0),
        ] {
            let p = palette(hue, chroma);
            let start = if name == "comment" {
                syntax_start + if self.dark { -10 } else { 10 }
            } else {
                syntax_start
            };
            let c = tone_for_contrast(&p, start, code_bg, text);
            set(&mut self.tokens, &format!("syntax-{name}"), c);
        }
        let variable = tone_for_contrast(&neutral, if self.dark { 90 } else { 15 }, code_bg, text);
        set(&mut self.tokens, "syntax-variable", variable);
        let punct = tone_for_contrast(&neutral, if self.dark { 75 } else { 35 }, code_bg, text);
        set(&mut self.tokens, "syntax-punctuation", punct);

        // Make the M3 text roles meet the theme's ratio on every surface
        // (Sepia and Dim move surfaces after M3 solved them).
        for (fg, bgs) in [
            (
                "on-surface",
                &[
                    "surface",
                    "surface-container-lowest",
                    "surface-container-low",
                    "surface-container",
                    "surface-container-high",
                    "surface-container-highest",
                ][..],
            ),
            ("on-surface-variant", &["surface", "surface-container"][..]),
        ] {
            let worst = bgs
                .iter()
                .map(|bg| self.get(bg))
                .min_by(|a, b| {
                    self.get(fg)
                        .contrast(*a)
                        .total_cmp(&self.get(fg).contrast(*b))
                })
                .unwrap_or(surface);
            if self.get(fg).contrast(worst) < text {
                let start = if self.dark { 90 } else { 20 };
                let fixed = tone_for_contrast(&neutral, start, worst, text);
                set(&mut self.tokens, fg, fixed);
            }
        }
        if self.get("outline").contrast(surface) < 3.0 {
            let fixed = tone_for_contrast(&neutral, if self.dark { 60 } else { 50 }, surface, 3.0);
            set(&mut self.tokens, "outline", fixed);
        }
    }
}

/// The page stylesheet: every built-in theme plus the component styles.
///
/// `:root` follows the operating system's light or dark preference until a
/// `data-vl-theme` attribute picks a theme (R-TH-1).
#[must_use]
pub fn stylesheet() -> String {
    let mut out = String::from("/* Vellum themes: generated by vellum-render. */\n");
    out.push_str(&Theme::builtin(BuiltinTheme::Light).css(":root"));
    out.push_str("@media (prefers-color-scheme: dark) {\n");
    out.push_str(&Theme::builtin(BuiltinTheme::Dark).css(":root:not([data-vl-theme])"));
    out.push_str("}\n");
    for which in BuiltinTheme::ALL {
        let selector = format!("[data-vl-theme=\"{}\"]", which.id());
        out.push_str(&Theme::builtin(which).css(&selector));
    }
    out.push_str(COMPONENT_CSS);
    out
}

/// Styles for every class the renderer emits. They use only `--vl-*`
/// variables, so they work under any theme.
pub const COMPONENT_CSS: &str = include_str!("vellum.css");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_theme_is_complete_and_passes_contrast() {
        for which in BuiltinTheme::ALL {
            let theme = Theme::builtin(which);
            for token in TOKENS {
                assert!(
                    theme.tokens.contains_key(*token),
                    "{} lacks {token}",
                    which.id()
                );
            }
            assert_eq!(theme.tokens.len(), TOKENS.len());
            assert_eq!(theme.validate(), vec![], "{}", which.id());
        }
    }

    #[test]
    fn ids_round_trip() {
        for which in BuiltinTheme::ALL {
            assert_eq!(BuiltinTheme::from_id(which.id()), Some(which));
        }
        assert_eq!(BuiltinTheme::from_id("Light"), None);
        assert_eq!(BuiltinTheme::from_id(""), None);
        assert!(BuiltinTheme::Sepia.is_light());
        assert!(!BuiltinTheme::Dim.is_light());
    }

    #[test]
    fn any_seed_produces_valid_themes() {
        for seed in [
            0xFF_0000, 0x00_8000, 0xFF_D700, 0x00_00FF, 0x80_8080, 0x00_0000, 0xFF_FFFF,
        ] {
            for which in BuiltinTheme::ALL {
                let theme = Theme::from_seed(which, Color::from_u32(seed));
                assert_eq!(theme.validate(), vec![], "{} from {seed:06x}", which.id());
            }
        }
    }

    #[test]
    fn json_round_trips_and_rejects_bad_files() {
        let theme = Theme::builtin(BuiltinTheme::Sepia);
        let json = theme.to_json();
        assert_eq!(Theme::from_json(&json).unwrap(), theme);

        let mut missing = theme.clone();
        let _ = missing.tokens.remove("chart-3");
        assert!(matches!(
            Theme::from_json(&missing.to_json()),
            Err(ThemeError::MissingToken(t)) if t == "chart-3"
        ));

        let mut unknown = theme.clone();
        let _ = unknown.tokens.insert("sparkle".into(), Color::new(1, 2, 3));
        assert!(matches!(
            Theme::from_json(&unknown.to_json()),
            Err(ThemeError::UnknownToken(_))
        ));

        let mut low = theme;
        let surface = low.get("surface");
        let _ = low.tokens.insert("on-surface".into(), surface);
        match Theme::from_json(&low.to_json()) {
            Err(ThemeError::Contrast(issues)) => {
                assert!(
                    issues
                        .iter()
                        .any(|i| i.fg == "on-surface" && i.bg == "surface")
                );
            }
            other => panic!("expected a contrast error, got {other:?}"),
        }
        assert!(matches!(Theme::from_json("{"), Err(ThemeError::Json(_))));
    }

    #[test]
    fn high_contrast_reaches_seven_to_one() {
        let theme = Theme::builtin(BuiltinTheme::HighContrast);
        assert!(theme.get("on-surface").contrast(theme.get("surface")) >= 7.0);
        assert!(theme.get("syntax-comment").contrast(theme.get("code-bg")) >= 7.0);
    }

    #[test]
    fn default_light_primary_is_the_brand_colour_family() {
        let theme = Theme::builtin(BuiltinTheme::Light);
        // M3 tone 40 of the brief's #4F5B92 seed stays in its hue.
        let hue = theme.get("primary").hue();
        assert!((hue - DEFAULT_SEED.hue()).abs() < 15.0, "hue {hue}");
    }

    #[test]
    fn resolve_vars_bakes_colours_in() {
        let theme = Theme::builtin(BuiltinTheme::Light);
        let out = theme.resolve_vars("fill:var(--vl-chart-1);stroke:var(--vl-nope)");
        assert_eq!(
            out,
            format!("fill:{};stroke:var(--vl-nope)", theme.get("chart-1"))
        );
    }

    #[test]
    fn stylesheet_has_every_theme_and_components() {
        let css = stylesheet();
        for which in BuiltinTheme::ALL {
            assert!(css.contains(&format!("[data-vl-theme=\"{}\"]", which.id())));
        }
        assert!(css.contains("prefers-color-scheme: dark"));
        assert!(css.contains(".vl-callout"));
    }
}
