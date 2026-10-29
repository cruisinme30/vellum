// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Branch names and pull request titles for an issue.
//!
//! The default branch is `user/eng-142-fix-login-redirect`: the person's
//! handle, the lower-case key and a slug of the title, at most 60
//! characters in all. A person may set their own format in settings using
//! the placeholders `{user}`, `{key}`, `{team}`, `{number}` and `{slug}`.

use crate::model::IssueKey;

/// The default format.
#[allow(clippy::literal_string_with_formatting_args)]
pub const DEFAULT_FORMAT: &str = "{user}/{key}-{slug}";

/// The longest branch name generated.
pub const MAX_LEN: usize = 60;

const fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => "a",
        'æ' => "ae",
        'ç' | 'č' | 'ć' => "c",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' | 'ń' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => "o",
        'œ' => "oe",
        'ß' => "ss",
        'ś' | 'š' => "s",
        'ù' | 'ú' | 'û' | 'ü' | 'ū' => "u",
        'ý' | 'ÿ' => "y",
        'ž' | 'ź' | 'ż' => "z",
        'ł' => "l",
        'đ' | 'ð' => "d",
        'þ' => "th",
        _ => return None,
    })
}

/// A lower-case, hyphenated ASCII slug of `text`.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut gap = false;
    for c in text.chars().flat_map(char::to_lowercase) {
        let piece = if c.is_ascii_alphanumeric() {
            Some(c.to_string())
        } else {
            fold(c).map(str::to_owned)
        };
        match piece {
            Some(piece) => {
                if gap && !out.is_empty() {
                    out.push('-');
                }
                gap = false;
                out.push_str(&piece);
            }
            None => gap = true,
        }
    }
    out
}

/// Makes `handle` safe for a Git ref component.
fn handle(user: &str) -> String {
    let handle = slug(user.split('@').next().unwrap_or(user));
    if handle.is_empty() {
        "user".to_owned()
    } else {
        handle
    }
}

/// The branch name for an issue.
///
/// The slug is cut at a hyphen so the whole name fits [`MAX_LEN`]; the
/// key is never cut, because automation finds the issue by it.
#[must_use]
#[allow(clippy::literal_string_with_formatting_args)]
pub fn branch_name(format: &str, user: &str, key: &IssueKey, title: &str) -> String {
    let format = if format.contains("{key}") || format.contains("{number}") {
        format
    } else {
        DEFAULT_FORMAT
    };
    let fill = |slug: &str| {
        format
            .replace("{user}", &handle(user))
            .replace("{key}", &key.to_string().to_ascii_lowercase())
            .replace("{team}", &key.team.to_ascii_lowercase())
            .replace("{number}", &key.number.to_string())
            .replace("{slug}", slug)
    };
    let full_slug = slug(title);
    let base_len = fill("").len();
    let room = MAX_LEN.saturating_sub(base_len);
    let mut cut = full_slug.as_str();
    if cut.len() > room {
        cut = cut.get(..room).unwrap_or_default();
        if let Some((head, _)) = cut.rsplit_once('-') {
            cut = head;
        }
    }
    let name = fill(cut);
    // An empty slug leaves a dangling separator.
    name.trim_end_matches(['-', '/', '_']).to_owned()
}

/// The pull request title: `ENG-142 Fix login redirect`.
#[must_use]
pub fn pr_title(key: &IssueKey, title: &str) -> String {
    format!("{key} {}", crate::model::one_line(title, 200))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> IssueKey {
        IssueKey {
            team: "ENG".to_owned(),
            number: 142,
        }
    }

    #[test]
    fn default_branch_name() {
        assert_eq!(
            branch_name(
                DEFAULT_FORMAT,
                "ada@northstar.dev",
                &key(),
                "Fix login redirect on Safari!"
            ),
            "ada/eng-142-fix-login-redirect-on-safari"
        );
    }

    #[test]
    fn long_titles_are_cut_at_a_word() {
        let name = branch_name(
            DEFAULT_FORMAT,
            "ada",
            &key(),
            "Investigate intermittent failures in the nightly export pipeline for large books",
        );
        assert!(name.len() <= MAX_LEN, "{name}");
        assert!(name.starts_with("ada/eng-142-investigate"));
        assert!(!name.ends_with('-'));
    }

    #[test]
    fn custom_formats_and_unicode() {
        assert_eq!(
            branch_name(
                "{team}/{number}/{slug}",
                "Zoë",
                &key(),
                "Ünïcode café — naïve"
            ),
            "eng/142/unicode-cafe-naive"
        );
        // A format without the key falls back to the default.
        assert_eq!(
            branch_name("{user}/{slug}", "bo", &key(), "x"),
            "bo/eng-142-x"
        );
        assert_eq!(
            branch_name(DEFAULT_FORMAT, "bo", &key(), "日本語"),
            "bo/eng-142"
        );
        assert_eq!(
            branch_name(DEFAULT_FORMAT, "", &key(), "x"),
            "user/eng-142-x"
        );
    }

    #[test]
    fn titles() {
        assert_eq!(pr_title(&key(), "Fix login\nmore"), "ENG-142 Fix login");
    }
}
