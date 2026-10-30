// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Display math: `TeX` to `MathML` Core.
//!
//! `MathML` renders natively in every current browser, is selectable and
//! accessible, and needs no script; Word and PDF exports read it too.

use pulldown_latex::config::DisplayMode;
use pulldown_latex::{Parser, RenderConfig, Storage, push_mathml};

/// Converts `TeX` to a `<math display="block">` element.
///
/// # Errors
///
/// Returns the parser's message when the `TeX` is invalid; callers show the
/// source instead.
pub fn to_mathml(tex: &str) -> Result<String, String> {
    let tex = tex.trim();
    if tex.len() > 64 * 1024 {
        return Err("formula is longer than 64 KiB".to_owned());
    }
    let storage = Storage::new();
    let events: Vec<_> = Parser::new(tex, &storage).collect();
    if let Some(Err(e)) = events.iter().find(|e| e.is_err()) {
        return Err(e.to_string());
    }
    let mut out = String::new();
    let config = RenderConfig {
        display_mode: DisplayMode::Block,
        xml: true,
        ..RenderConfig::default()
    };
    push_mathml(&mut out, events.into_iter(), config)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_fractions_and_subscripts() {
        let m = to_mathml("F_1 = \\frac{2PR}{P+R}\n").unwrap();
        assert!(m.starts_with("<math"), "{m}");
        assert!(m.contains("<mfrac>"), "{m}");
        assert!(m.contains("<msub>"), "{m}");
    }

    #[test]
    fn reports_errors() {
        assert!(to_mathml("\\frac{1}").is_err());
        assert!(to_mathml("\\notacommand").is_err());
    }
}
