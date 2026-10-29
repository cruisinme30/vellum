// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Walking a document's links and images, and resolving relative paths.

use vellum_format::{Block, Inline};

/// Calls `f` on every inline run of `blocks`, including table cells,
/// list items and containers.
pub fn inlines_mut(blocks: &mut [Block], f: &mut impl FnMut(&mut Vec<Inline>)) {
    for block in blocks {
        match block {
            Block::Paragraph { content, .. } | Block::Heading { content, .. } => {
                walk_inlines(content, f);
            }
            Block::BlockQuote { blocks } => inlines_mut(blocks, f),
            Block::List(list) => {
                for item in &mut list.items {
                    inlines_mut(&mut item.blocks, f);
                }
            }
            Block::Table(table) => {
                for cell in table
                    .header
                    .iter_mut()
                    .chain(table.rows.iter_mut().flatten())
                {
                    walk_inlines(cell, f);
                }
            }
            Block::Container(container) => {
                for section in &mut container.sections {
                    inlines_mut(section, f);
                }
            }
            Block::ThematicBreak | Block::Fence(_) | Block::Math { .. } => {}
        }
    }
}

fn walk_inlines(inlines: &mut Vec<Inline>, f: &mut impl FnMut(&mut Vec<Inline>)) {
    for inline in inlines.iter_mut() {
        match inline {
            Inline::Emphasis { content }
            | Inline::Strong { content }
            | Inline::Strikethrough { content }
            | Inline::Highlight { content }
            | Inline::Subscript { content }
            | Inline::Superscript { content }
            | Inline::Link { content, .. } => walk_inlines(content, f),
            _ => {}
        }
    }
    f(inlines);
}

/// Whether `url` names something outside the import (`https:`, `mailto:`,
/// `vellum:`) or a place in the same page (`#…`).
pub fn is_external(url: &str) -> bool {
    let url = url.trim();
    url.starts_with('#')
        || url.starts_with("//")
        || url.split_once(':').is_some_and(|(scheme, _)| {
            !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && scheme.len() > 1
        })
}

/// Splits `url` into its path and `#fragment`, dropping any `?query`, and
/// percent-decodes the path.
pub fn split(url: &str) -> (String, Option<String>) {
    let url = url.trim().trim_start_matches('<').trim_end_matches('>');
    let (rest, fragment) = match url.split_once('#') {
        Some((rest, fragment)) => (rest, Some(fragment.to_owned()).filter(|f| !f.is_empty())),
        None => (url, None),
    };
    let rest = rest.split_once('?').map_or(rest, |(path, _)| path);
    let decoded = percent_encoding::percent_decode_str(rest)
        .decode_utf8_lossy()
        .into_owned();
    (decoded, fragment)
}

/// Joins `relative` onto the folder `base` (both `/`-separated, relative
/// to the import root) and normalises `.` and `..`. A leading `/` starts
/// from the root. Returns `None` when `..` climbs out of the root.
pub fn join(base: &str, relative: &str) -> Option<String> {
    let mut parts: Vec<&str> = if relative.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|p| !p.is_empty()).collect()
    };
    for part in relative.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                let _ = parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// The folder part of a path (`a/b` for `a/b/c.md`, empty at the root).
pub fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// The path from the folder `from` to the file `to`, both relative to the
/// import root, as a Markdown link destination.
pub fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').filter(|p| !p.is_empty()).collect();
    let to_parts: Vec<&str> = to.split('/').filter(|p| !p.is_empty()).collect();
    let common = from
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count();
    let mut out: Vec<String> = from
        .get(common..)
        .unwrap_or_default()
        .iter()
        .map(|_| "..".to_owned())
        .collect();
    out.extend(
        to_parts
            .get(common..)
            .unwrap_or_default()
            .iter()
            .map(|p| (*p).to_owned()),
    );
    let path = out.join("/");
    if path.contains([' ', '(', ')', '<', '>']) {
        percent_encoding::utf8_percent_encode(&path, PATH).to_string()
    } else {
        path
    }
}

const PATH: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'(')
    .add(b')')
    .add(b'<')
    .add(b'>')
    .add(b'"');

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_and_climbs() {
        assert_eq!(join("a/b", "../c/d.md").as_deref(), Some("a/c/d.md"));
        assert_eq!(join("a", "./x.png").as_deref(), Some("a/x.png"));
        assert_eq!(join("a/b", "/root.md").as_deref(), Some("root.md"));
        assert_eq!(join("a", "../../x"), None);
    }

    #[test]
    fn relative_paths_encode_spaces() {
        assert_eq!(relative("a/b", "a/assets/x.png"), "../assets/x.png");
        assert_eq!(relative("", "My Page/img 1.png"), "My%20Page/img%201.png");
    }

    #[test]
    fn splits_and_decodes() {
        assert_eq!(
            split("Page%20One%20abc.md?x=1#part"),
            ("Page One abc.md".into(), Some("part".into()))
        );
    }

    #[test]
    fn external_urls_are_left_alone() {
        assert!(is_external("https://northstar.dev"));
        assert!(is_external("mailto:ops@northstar.dev"));
        assert!(is_external("vellum://doc/01ABC"));
        assert!(is_external("#section"));
        assert!(!is_external("Child%20Page.md"));
        assert!(!is_external("C:/not/a/scheme"));
    }
}
