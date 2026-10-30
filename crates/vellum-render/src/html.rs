// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Escaping and small writing helpers shared by every renderer module.
//!
//! Every byte of user content goes through [`text`] or [`attr`]; markup is
//! only ever written from string literals in this crate.

use std::fmt::Write as _;

/// Appends `s` escaped for an HTML text node.
pub fn text(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // Invisible and bidi-control characters are stripped: they are a
            // prompt-injection and spoofing vector and never carry meaning.
            c if is_invisible(c) => {}
            c => out.push(c),
        }
    }
}

/// Appends `s` escaped for a double-quoted attribute value.
pub fn attr(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if is_invisible(c) => {}
            c => out.push(c),
        }
    }
}

/// Returns `s` escaped for a text node.
#[must_use]
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    text(&mut out, s);
    out
}

/// True for zero-width, bidi-override and other invisible format characters.
#[must_use]
pub const fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{00AD}'
    ) || (c.is_control() && c != '\n' && c != '\t')
}

/// Appends ` name="value"` with the value escaped.
pub fn push_attr(out: &mut String, name: &str, value: &str) {
    out.push(' ');
    out.push_str(name);
    out.push_str("=\"");
    attr(out, value);
    out.push('"');
}

/// Formats a number for SVG coordinates: at most two decimals, no trailing
/// zeros, so output is stable across platforms.
#[must_use]
pub fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_owned();
    }
    let mut s = String::new();
    let _ = write!(s, "{v:.2}");
    if s.contains('.') {
        while s.ends_with('0') {
            let _ = s.pop();
        }
        if s.ends_with('.') {
            let _ = s.pop();
        }
    }
    if s == "-0" { "0".to_owned() } else { s }
}

/// A URL-safe, lowercase slug for anchors: letters and digits joined by `-`.
#[must_use]
pub fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out
}

/// True when `class` is safe to copy from a `.class` attribute: an ASCII
/// identifier that does not use the reserved `vl-` prefix.
#[must_use]
pub fn is_user_class(class: &str) -> bool {
    let mut chars = class.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !class.starts_with("vl-")
        && class.len() <= 64
}

/// True when `id` is safe to use as an element id (from `{#id}`).
#[must_use]
pub fn is_user_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
        })
        && id.len() <= 128
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_markup_and_strips_invisible_characters() {
        assert_eq!(escape("<b>&\u{200B}x"), "&lt;b&gt;&amp;x");
        let mut out = String::new();
        attr(&mut out, "a\"b'c");
        assert_eq!(out, "a&quot;b&#39;c");
    }

    #[test]
    fn numbers_are_short_and_stable() {
        assert_eq!(num(1.0), "1");
        assert_eq!(num(1.256), "1.26");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(num(12.5), "12.5");
    }

    #[test]
    fn slugs_and_identifiers() {
        assert_eq!(slug("Request path: v2!"), "request-path-v2");
        assert_eq!(slug("  Ünïcode  ok "), "ünïcode-ok");
        assert!(is_user_class("wide"));
        assert!(!is_user_class("vl-svg"));
        assert!(!is_user_class("a b"));
        assert!(is_user_id("rollback"));
        assert!(!is_user_id("1x"));
    }
}
