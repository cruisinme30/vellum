// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Block examples in the guide: a fence `markdown {corpus="kind/name"}`
//! holding the source of the block shown just above it.

use super::coverage::KINDS;
use super::markdown;

/// One marked example.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    /// `kind/name`, which is also its corpus path without `.md`.
    pub name: String,
    /// Whether the block itself must appear just above the source.
    pub live: bool,
    /// The source.
    pub body: String,
    /// One-based line of the fence, for messages.
    pub line: usize,
}

/// Reads `key="value"` or `key=value` from a fence's attribute block.
fn attribute<'a>(attributes: &'a str, key: &str) -> Option<&'a str> {
    let start = attributes.find(&format!("{key}="))? + key.len() + 1;
    let rest = attributes.get(start..)?;
    rest.strip_prefix('"').map_or_else(
        || rest.split(|c: char| c.is_whitespace() || c == '}').next(),
        |quoted| quoted.split_once('"').map(|(value, _)| value),
    )
}

/// Finds the examples in a page and checks each one against the text above
/// it. Problems are pushed to `errors` prefixed with `location`.
pub fn collect(
    location: &str,
    text: &str,
    errors: &mut Vec<String>,
) -> Vec<Example> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for fence in markdown::fences(text) {
        let mut words = fence.info.splitn(2, ' ');
        let language = words.next().unwrap_or_default();
        let attributes = words.next().unwrap_or_default();
        let Some(name) = attribute(attributes, "corpus") else {
            continue;
        };
        let line_number = fence.open_line + 1;
        if language != "markdown" {
            errors.push(format!(
                "{location}:{line_number}: corpus examples use a `markdown` \
                    fence"
            ));
        }
        let valid_name = name.split_once('/').is_some_and(|(kind, rest)| {
            KINDS.contains(&kind)
                && !rest.is_empty()
                && rest.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
                })
        });
        if !valid_name {
            errors.push(format!(
                "{location}:{line_number}: corpus name `{name}` must be \
                    kind/name with kind one of {KINDS:?} and a lower-case \
                    kebab name"
            ));
        }
        let live = attribute(attributes, "live") != Some("false");
        if live {
            let above =
                lines.get(..fence.open_line).unwrap_or_default().join("\n");
            let above = above.trim_end();
            let body = fence.body.trim_end();
            let matches = above
                .strip_suffix(body)
                .is_some_and(|rest| rest.is_empty() || rest.ends_with("\n\n"));
            if body.is_empty() || !matches {
                errors.push(format!(
                    "{location}:{line_number}: example `{name}` is not \
                     identical to the block above it; paste the source above \
                     the fence, or mark it live=false"
                ));
            }
        }
        found.push(Example {
            name: name.to_owned(),
            live,
            body: fence.body,
            line: line_number,
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_example_matches_block_above() {
        let page = "Intro.\n\n::: tip\nHi\n:::\n\n\
            ````markdown {corpus=\"container/tip\"}\n::: tip\nHi\n:::\n````\n";
        let mut errors = Vec::new();
        let found = collect("p.md", page, &mut errors);
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(found.len(), 1);
        assert_eq!(
            found.first().map(|e| e.body.as_str()),
            Some("::: tip\nHi\n:::")
        );
    }

    #[test]
    fn drifted_example_is_reported() {
        let page = "::: tip\nHello\n:::\n\n\
            ````markdown {corpus=\"container/tip\"}\n::: tip\nHi\n:::\n````\n";
        let mut errors = Vec::new();
        let _ = collect("p.md", page, &mut errors);
        assert_eq!(errors.len(), 1, "{errors:?}");
    }

    #[test]
    fn source_only_example_and_bad_name() {
        let page = "````markdown {corpus=\"page/front-matter\" live=false}\n\
            ---\nid: x\n---\n````\n\n````markdown {corpus=\"Bad/Name\"}\nx\n\
            ````\n";
        let mut errors = Vec::new();
        let found = collect("p.md", page, &mut errors);
        assert_eq!(found.first().map(|e| e.live), Some(false));
        assert!(errors.iter().any(|e| e.contains("Bad/Name")), "{errors:?}");
    }
}
