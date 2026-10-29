// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Section numbers, computed from the tree and never stored (R-BK-3).
//!
//! Numbering follows mdBook: only numbered chapters get numbers, drafts
//! count, part titles and separators do not, and numbering continues
//! across parts.

use std::fmt::Write as _;

use crate::manifest::NumberingStyle;

/// A section number such as `2.3`, held as its components (`[2, 3]`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct SectionNumber(pub Vec<u32>);

impl SectionNumber {
    /// The nesting depth, 1 for a top-level chapter.
    #[must_use]
    pub const fn depth(&self) -> usize {
        self.0.len()
    }

    /// The number written in `style`, or `None` for [`NumberingStyle::None`].
    #[must_use]
    pub fn format(&self, style: NumberingStyle) -> Option<String> {
        match style {
            NumberingStyle::None => None,
            NumberingStyle::Decimal => Some(
                self.0
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join("."),
            ),
            NumberingStyle::Outline => Some(
                self.0
                    .iter()
                    .enumerate()
                    .map(|(level, n)| outline_component(level, *n))
                    .collect::<Vec<_>>()
                    .join("."),
            ),
        }
    }
}

fn outline_component(level: usize, n: u32) -> String {
    match level % 6 {
        0 => roman(n).to_uppercase(),
        1 => letters(n).to_uppercase(),
        3 => letters(n),
        4 => roman(n),
        _ => n.to_string(),
    }
}

/// `1 → a`, `26 → z`, `27 → aa`.
fn letters(mut n: u32) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        out.push(char::from(b'a' + u8::try_from(n % 26).unwrap_or_default()));
        n /= 26;
    }
    out.iter().rev().collect()
}

/// Lower-case Roman numerals; numbers past 3999 fall back to digits.
fn roman(mut n: u32) -> String {
    const TABLE: [(u32, &str); 13] = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    if n == 0 || n > 3999 {
        return n.to_string();
    }
    let mut out = String::new();
    for (value, digits) in TABLE {
        while n >= value {
            let _ = out.write_str(digits);
            n -= value;
        }
    }
    out
}

/// The cross-reference label for a section: `§2.3 Services`, or just the
/// title when the section is unnumbered or the book uses no numbers.
#[must_use]
pub fn label(number: Option<&SectionNumber>, style: NumberingStyle, title: &str) -> String {
    number
        .and_then(|n| n.format(style))
        .map_or_else(|| title.to_owned(), |n| format!("§{n} {title}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal() {
        let n = SectionNumber(vec![2, 3]);
        assert_eq!(n.format(NumberingStyle::Decimal).as_deref(), Some("2.3"));
        assert_eq!(n.format(NumberingStyle::None), None);
        assert_eq!(
            label(Some(&n), NumberingStyle::Decimal, "Services"),
            "§2.3 Services"
        );
        assert_eq!(label(None, NumberingStyle::Decimal, "Glossary"), "Glossary");
    }

    #[test]
    fn outline() {
        let n = SectionNumber(vec![2, 3, 4, 1, 4, 6]);
        assert_eq!(
            n.format(NumberingStyle::Outline).as_deref(),
            Some("II.C.4.a.iv.6")
        );
        assert_eq!(letters(27), "aa");
        assert_eq!(roman(1994), "mcmxciv");
        assert_eq!(roman(4000), "4000");
    }
}
