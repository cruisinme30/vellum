// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The small amount of Markdown structure the help checks need: front
//! matter, code fences and heading anchors. This is not a parser for the
//! dialect; `vellum-format` is.

/// A code fence at the top level of a page (not nested in another fence).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fence {
    /// Zero-based index of the opening line.
    pub open_line: usize,
    /// The info string after the backticks, trimmed.
    pub info: String,
    /// The lines between the opening and closing lines, joined with `\n`.
    pub body: String,
}

/// The parsed front matter of a page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatter {
    /// The `id` key, if present.
    pub id: Option<String>,
    /// The `title` key, if present, without surrounding quotes.
    pub title: Option<String>,
}

/// Splits a page into its front matter keys, if it starts with a `---` block.
#[must_use]
pub fn front_matter(text: &str) -> Option<FrontMatter> {
    let rest = text.strip_prefix("---\n")?;
    let end = rest
        .find("\n---\n")
        .or_else(|| rest.strip_suffix("\n---").map(str::len))?;
    let block = rest.get(..end)?;
    let mut matter = FrontMatter::default();
    for line in block.lines() {
        if line.starts_with(' ') {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let value = unquote(value.trim());
            match key {
                "id" => matter.id = Some(value.to_owned()),
                "title" => matter.title = Some(value.to_owned()),
                _ => {}
            }
        }
    }
    Some(matter)
}

/// Removes one pair of matching single or double quotes.
#[must_use]
pub fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return inner;
        }
    }
    value
}

/// The opening of a fence: its marker character, length and info string.
fn fence_marker(line: &str) -> Option<(char, usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    if indent > 3 {
        return None;
    }
    let trimmed = line.trim_start_matches(' ');
    let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let count = trimmed.chars().take_while(|c| *c == marker).count();
    if count < 3 {
        return None;
    }
    let info = trimmed.get(count..).unwrap_or_default().trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some((marker, count, info))
}

/// Finds every top-level fence, skipping fences nested inside another.
#[must_use]
pub fn fences(text: &str) -> Vec<Fence> {
    let mut found = Vec::new();
    let mut open: Option<(char, usize, usize, String)> = None;
    let mut body: Vec<&str> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        match &open {
            None => {
                if let Some((marker, count, info)) = fence_marker(line) {
                    open = Some((marker, count, index, info.to_owned()));
                    body.clear();
                }
            }
            Some((marker, count, start, info)) => {
                let closes = fence_marker(line).is_some_and(|(m, c, rest)| {
                    m == *marker && c >= *count && rest.is_empty()
                });
                if closes {
                    found.push(Fence {
                        open_line: *start,
                        info: info.clone(),
                        body: body.join("\n"),
                    });
                    open = None;
                } else {
                    body.push(line);
                }
            }
        }
    }
    found
}

/// The anchors a page offers: each heading's custom `{#id}` and its slug.
#[must_use]
pub fn anchors(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut in_fence: Option<(char, usize)> = None;
    for line in text.lines() {
        if let Some((marker, count, rest)) = fence_marker(line) {
            match in_fence {
                None => in_fence = Some((marker, count)),
                Some((m, c))
                    if m == marker && count >= c && rest.is_empty() =>
                {
                    in_fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if in_fence.is_some() {
            continue;
        }
        let hashes = line.chars().take_while(|c| *c == '#').count();
        let Some(heading) =
            line.get(hashes..).and_then(|h| h.strip_prefix(' '))
        else {
            continue;
        };
        if hashes == 0 || hashes > 6 {
            continue;
        }
        let (text, custom) = split_attributes(heading.trim());
        if let Some(id) = custom {
            found.push(id.to_owned());
        }
        found.push(slug(text));
    }
    found
}

/// Splits `Title {#id .class}` into the title and the custom id.
fn split_attributes(heading: &str) -> (&str, Option<&str>) {
    let Some(open) = heading.rfind(" {") else {
        return (heading, None);
    };
    let (title, attributes) = heading.split_at(open);
    let attributes = attributes.trim();
    if !attributes.ends_with('}') {
        return (heading, None);
    }
    let id = attributes
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split_whitespace()
        .find_map(|part| part.strip_prefix('#'));
    (title.trim_end(), id)
}

/// The anchor that `mdBook` and `GitHub` derive from a heading.
#[must_use]
pub fn slug(text: &str) -> String {
    text.chars()
        .filter_map(|ch| {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                Some(ch.to_lowercase().next().unwrap_or(ch))
            } else if ch.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_front_matter() {
        let page = "---\nid: 01ABC\ntitle: \"PRD: X\"\ntags: [a]\n---\n\n# X\n";
        let matter = front_matter(page).unwrap_or_default();
        assert_eq!(matter.id.as_deref(), Some("01ABC"));
        assert_eq!(matter.title.as_deref(), Some("PRD: X"));
        assert_eq!(front_matter("# No front matter\n"), None);
    }

    #[test]
    fn finds_top_level_fences_only() {
        let page = "a\n\n````markdown {corpus=\"x/y\"}\n```chart\nbody\n```\n\
            ````\n\n```bash\nls\n```\n";
        let found = fences(page);
        assert_eq!(found.len(), 2);
        assert_eq!(
            found.first().map(|f| f.info.as_str()),
            Some("markdown {corpus=\"x/y\"}")
        );
        assert_eq!(
            found.first().map(|f| f.body.as_str()),
            Some("```chart\nbody\n```")
        );
        assert_eq!(found.get(1).map(|f| f.open_line), Some(8));
    }

    #[test]
    fn headings_give_custom_ids_and_slugs() {
        let page = "# Page\n\n## Share the page {#share}\n\n```text\n\
            ## not a heading\n```\n\n## Math blocks\n";
        let found = anchors(page);
        assert!(found.contains(&"share".to_owned()));
        assert!(found.contains(&"share-the-page".to_owned()));
        assert!(found.contains(&"math-blocks".to_owned()));
        assert!(!found.contains(&"not-a-heading".to_owned()));
    }

    #[test]
    fn slugs_match_mdbook() {
        assert_eq!(
            slug("Exporting & publishing, a book"),
            "exporting--publishing-a-book"
        );
        assert_eq!(
            slug("Live charts from a collection"),
            "live-charts-from-a-collection"
        );
    }
}
