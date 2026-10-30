// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Pandoc-style attribute blocks: `{#id .class key="value"}`.
//!
//! The grammar matches comrak's `attributes` feature, which parses the same
//! syntax on headings and fences, so every block reads attributes alike.

use crate::ast::Attributes;

/// Characters allowed in a bare id, class or value.
const fn is_bare(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.' | '%')
}

/// Characters allowed in a key.
const fn is_key(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')
}

/// Splits a trailing `{…}` attribute block off `text`.
///
/// Returns the text before it (trimmed) and the attributes. When `text` does
/// not end in a valid block, returns it unchanged with empty attributes.
#[must_use]
pub fn split_trailing(text: &str) -> (&str, Attributes) {
    let trimmed = text.trim_end();
    if trimmed.ends_with('}') {
        // Try every `{` from the right; quoted values may contain braces.
        for (start, _) in trimmed.match_indices('{').rev() {
            let before = trimmed.get(..start).unwrap_or_default();
            if !before.is_empty() && !before.ends_with(char::is_whitespace) {
                continue;
            }
            if let Some(attrs) = trimmed.get(start..).and_then(parse) {
                return (before.trim_end(), attrs);
            }
        }
    }
    (trimmed, Attributes::default())
}

/// Parses a complete `{…}` block. Returns `None` if it is not valid.
#[must_use]
pub fn parse(input: &str) -> Option<Attributes> {
    let inner = input.strip_prefix('{')?.strip_suffix('}')?;
    let mut attrs = Attributes::default();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.next_if(char::is_ascii_whitespace).is_some() {}
        let Some(first) = chars.next() else {
            return Some(attrs);
        };
        match first {
            '#' => attrs.id = Some(bare(&mut chars)?),
            '.' => attrs.classes.push(bare(&mut chars)?),
            c if c.is_ascii_alphabetic() => {
                let mut key = String::from(c);
                while let Some(k) = chars.next_if(|c| is_key(*c)) {
                    key.push(k);
                }
                if chars.next() != Some('=') {
                    return None;
                }
                let value = if chars.next_if_eq(&'"').is_some() {
                    quoted(&mut chars)?
                } else {
                    bare(&mut chars)?
                };
                attrs.pairs.push((key, value));
            }
            _ => return None,
        }
        // Each attribute must be followed by whitespace or the end.
        if chars.peek().is_some_and(|c| !c.is_ascii_whitespace()) {
            return None;
        }
    }
}

fn bare(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Option<String> {
    let mut value = String::new();
    while let Some(c) = chars.next_if(|c| is_bare(*c)) {
        value.push(c);
    }
    (!value.is_empty()).then_some(value)
}

fn quoted(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Option<String> {
    let mut value = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(value),
            '\\' => value.push(chars.next()?),
            c => value.push(c),
        }
    }
}

/// Writes attributes in canonical form: `{#id .a .b key="value"}`.
///
/// Returns an empty string for empty attributes. Ids and classes are written
/// bare; values are always quoted.
#[must_use]
pub fn to_string(attrs: &Attributes) -> String {
    if attrs.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    if let Some(id) = &attrs.id {
        parts.push(format!("#{id}"));
    }
    parts.extend(attrs.classes.iter().map(|c| format!(".{c}")));
    for (key, value) in &attrs.pairs {
        let mut part = format!("{key}=\"");
        for c in value.chars() {
            if matches!(c, '"' | '\\') {
                part.push('\\');
            }
            part.push(c);
        }
        part.push('"');
        parts.push(part);
    }
    format!("{{{}}}", parts.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_id_classes_and_pairs() {
        let attrs =
            parse(r#"{#intro .wide .dark owner="Priya S." k=v}"#).unwrap();
        assert_eq!(attrs.id.as_deref(), Some("intro"));
        assert_eq!(attrs.classes, ["wide", "dark"]);
        assert_eq!(
            attrs.pairs,
            [
                ("owner".into(), "Priya S.".into()),
                ("k".into(), "v".into())
            ]
        );
    }

    #[test]
    fn rejects_invalid_blocks() {
        for bad in [
            "{",
            "{#}",
            "{. a}",
            "{k=}",
            "{1=2}",
            r#"{k="v}"#,
            "{#a}x",
            r#"{k="v"x}"#,
        ] {
            assert!(parse(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn splits_trailing_block_only_after_whitespace() {
        let (rest, attrs) =
            split_trailing(r#"Why it fails {#why title="a } b"}  "#);
        assert_eq!(rest, "Why it fails");
        assert_eq!(attrs.id.as_deref(), Some("why"));
        let (rest, attrs) = split_trailing("set{x}");
        assert_eq!(rest, "set{x}");
        assert!(attrs.is_empty());
    }

    #[test]
    fn writes_canonical_form_that_parses_back() {
        let attrs = parse(r#"{k="say \"hi\" \\ ok" .b #x}"#).unwrap();
        let text = to_string(&attrs);
        assert_eq!(text, r#"{#x .b k="say \"hi\" \\ ok"}"#);
        assert_eq!(parse(&text), Some(attrs));
        assert_eq!(to_string(&Attributes::default()), "");
    }
}
