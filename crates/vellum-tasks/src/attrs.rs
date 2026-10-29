// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The attribute span that carries task fields on a checklist item.
//!
//! A task's structured fields sit in one Pandoc-style span at the end of the
//! item, before the block ID (D82):
//!
//! ```text
//! - [ ] Send the Northstar invoice {due=2026-11-03T15:00 rrule="FREQ=MONTHLY" p=1} ^blk-7f3a9k2m
//! ```
//!
//! Values that are not plain tokens are double-quoted, with `\"` and `\\`
//! escapes. Keys this crate does not know are kept, in order, so a newer
//! writer's fields survive an older reader.

use std::fmt;

/// An ordered list of `key=value` pairs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attrs {
    pairs: Vec<(String, String)>,
}

/// Why an attribute span could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttrsError {
    /// The span does not start with `{` and end with `}`.
    #[error("an attribute span must be wrapped in braces")]
    Braces,
    /// A key is empty or has characters outside `[a-z0-9_-]`.
    #[error("invalid attribute key at byte {0}")]
    Key(usize),
    /// A key has no `=value`.
    #[error("attribute {0} has no value")]
    Value(String),
    /// A quoted value is not closed.
    #[error("unterminated quoted value for attribute {0}")]
    Unterminated(String),
}

/// The order keys are written in; unknown keys follow in their own order.
const CANONICAL_ORDER: &[&str] = &[
    "when",
    "start",
    "due",
    "rrule",
    "recur_from",
    "remind",
    "p",
    "labels",
    "list",
    "done",
    "src",
];

const fn is_key_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'
}

const fn is_bare_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | ':' | '+' | '-' | '.' | '/' | '=' | ',' | ';')
}

impl Attrs {
    /// An empty set.
    #[must_use]
    pub const fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    /// Whether no attribute is set.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The value of `key`, if set.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Sets `key`, replacing any earlier value; an empty value removes it.
    pub fn set(&mut self, key: &str, value: impl Into<String>) {
        let value = value.into();
        if value.is_empty() {
            self.remove(key);
            return;
        }
        if let Some(slot) = self.pairs.iter_mut().find(|(k, _)| k == key) {
            slot.1 = value;
        } else {
            self.pairs.push((key.to_owned(), value));
        }
    }

    /// Sets `key` when `value` is `Some`, removes it otherwise.
    pub fn set_opt(&mut self, key: &str, value: Option<impl Into<String>>) {
        match value {
            Some(value) => self.set(key, value),
            None => self.remove(key),
        }
    }

    /// Removes `key`.
    pub fn remove(&mut self, key: &str) {
        self.pairs.retain(|(k, _)| k != key);
    }

    /// Every pair, in stored order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Parses `{key=value key="quoted value"}`.
    ///
    /// # Errors
    ///
    /// Returns [`AttrsError`] for a malformed span.
    pub fn parse(span: &str) -> Result<Self, AttrsError> {
        let inner = span
            .strip_prefix('{')
            .and_then(|rest| rest.strip_suffix('}'))
            .ok_or(AttrsError::Braces)?;
        let mut attrs = Self::new();
        let mut chars = inner.char_indices().peekable();
        loop {
            while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
            let Some(&(start, _)) = chars.peek() else {
                break;
            };
            let mut key = String::new();
            while let Some((_, c)) = chars.next_if(|(_, c)| is_key_char(*c)) {
                key.push(c);
            }
            if key.is_empty() {
                return Err(AttrsError::Key(start + 1));
            }
            if chars.next_if(|(_, c)| *c == '=').is_none() {
                return Err(AttrsError::Value(key));
            }
            let mut value = String::new();
            if chars.next_if(|(_, c)| *c == '"').is_some() {
                let mut closed = false;
                while let Some((_, c)) = chars.next() {
                    match c {
                        '\\' => match chars.next() {
                            Some((_, escaped)) => value.push(escaped),
                            None => break,
                        },
                        '"' => {
                            closed = true;
                            break;
                        }
                        other => value.push(other),
                    }
                }
                if !closed {
                    return Err(AttrsError::Unterminated(key));
                }
            } else {
                while let Some((_, c)) = chars.next_if(|(_, c)| !c.is_whitespace()) {
                    value.push(c);
                }
                if value.is_empty() {
                    return Err(AttrsError::Value(key));
                }
            }
            attrs.set(&key, value);
        }
        Ok(attrs)
    }

    /// Whether a span of text looks like an attribute span this module
    /// can read; used to decide where the item title ends.
    #[must_use]
    pub fn looks_like_span(text: &str) -> bool {
        text.starts_with('{') && text.ends_with('}') && Self::parse(text).is_ok()
    }

    /// Sorts keys into the canonical order; unknown keys keep their order
    /// after the known ones.
    pub fn canonicalise(&mut self) {
        let rank = |key: &str| {
            CANONICAL_ORDER
                .iter()
                .position(|k| *k == key)
                .unwrap_or(CANONICAL_ORDER.len())
        };
        self.pairs.sort_by_key(|(k, _)| rank(k));
    }
}

fn write_value(f: &mut fmt::Formatter<'_>, value: &str) -> fmt::Result {
    if !value.is_empty() && value.chars().all(is_bare_char) {
        return f.write_str(value);
    }
    f.write_str("\"")?;
    for c in value.chars() {
        if matches!(c, '"' | '\\') {
            f.write_str("\\")?;
        }
        write!(f, "{c}")?;
    }
    f.write_str("\"")
}

impl fmt::Display for Attrs {
    /// Writes the span, or nothing when empty.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.pairs.is_empty() {
            return Ok(());
        }
        f.write_str("{")?;
        for (index, (key, value)) in self.pairs.iter().enumerate() {
            if index > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{key}=")?;
            write_value(f, value)?;
        }
        f.write_str("}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bare_and_quoted_values() {
        let attrs = Attrs::parse(
            r#"{due=2026-11-03T15:00 rrule="FREQ=WEEKLY;BYDAY=MO" note="a \"b\" \\"}"#,
        )
        .unwrap();
        assert_eq!(attrs.get("due"), Some("2026-11-03T15:00"));
        assert_eq!(attrs.get("rrule"), Some("FREQ=WEEKLY;BYDAY=MO"));
        assert_eq!(attrs.get("note"), Some(r#"a "b" \"#));
    }

    #[test]
    fn writes_canonical_order_and_round_trips() {
        let mut attrs = Attrs::parse("{zz=1 due=2026-11-03 when=evening}").unwrap();
        attrs.canonicalise();
        assert_eq!(attrs.to_string(), "{when=evening due=2026-11-03 zz=1}");
        attrs.set("rrule", "FREQ=DAILY;INTERVAL=2");
        let text = attrs.to_string();
        assert_eq!(Attrs::parse(&text).unwrap(), attrs);
    }

    #[test]
    fn rejects_malformed_spans() {
        assert_eq!(Attrs::parse("due=1"), Err(AttrsError::Braces));
        assert_eq!(Attrs::parse("{due}"), Err(AttrsError::Value("due".into())));
        assert_eq!(Attrs::parse("{=1}"), Err(AttrsError::Key(1)));
        assert_eq!(
            Attrs::parse("{a=\"x}"),
            Err(AttrsError::Unterminated("a".into()))
        );
        assert!(!Attrs::looks_like_span("{not a span}"));
    }

    #[test]
    fn empty_value_removes() {
        let mut attrs = Attrs::new();
        attrs.set("p", "1");
        attrs.set("p", "");
        assert!(attrs.is_empty());
        assert_eq!(attrs.to_string(), "");
    }
}
