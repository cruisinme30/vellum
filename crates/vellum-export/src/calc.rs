// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! How calculator numbers are shown. The values themselves come from the
//! collections engine through [`crate::ports::Collections`].

/// Formats a number for display: integers without decimals, others with
/// up to two, thousands separated by commas.
#[must_use]
pub fn display_number(value: f64) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    let negative = rounded < 0.0;
    let text = format!("{:.2}", rounded.abs());
    let (int, frac) = text.split_once('.').unwrap_or((&text, "00"));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    let frac = frac.trim_end_matches('0');
    let sign = if negative { "-" } else { "" };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}.{frac}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_display_grouped_with_two_decimals_at_most() {
        assert_eq!(display_number(1_234_567.0), "1,234,567");
        assert_eq!(display_number(-0.125), "-0.13");
        assert_eq!(display_number(12.5), "12.5");
    }
}
