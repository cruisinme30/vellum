// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Line-level access to top-level YAML keys.
//!
//! The drive reads and changes only a handful of keys (`id`, `title`) in
//! files people also edit by hand, so it edits the matching line and keeps
//! every other byte, comment and key order as it was. Full parsing belongs
//! to `vellum-format`.

/// Splits a Markdown file into its front matter (without the `---` fences)
/// and the rest, returning `None` when the file has no front matter.
#[must_use]
pub fn split(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed == "---" || trimmed == "..." {
            let yaml = rest.get(..offset)?;
            let body = rest.get(offset + line.len()..)?;
            return Some((yaml, body));
        }
        offset += line.len();
    }
    None
}

/// Reads the scalar value of top-level `key` from YAML text.
#[must_use]
pub fn get(yaml: &str, key: &str) -> Option<String> {
    yaml.lines()
        .find_map(|line| value_of(line, key).map(unquote))
}

/// Reads top-level `key` from a Markdown file's front matter.
#[must_use]
pub fn get_md(text: &str, key: &str) -> Option<String> {
    split(text).and_then(|(yaml, _)| get(yaml, key))
}

/// Sets top-level `key` in YAML text, replacing its line or appending one.
#[must_use]
pub fn set(yaml: &str, key: &str, value: &str) -> String {
    let line = format!("{key}: {}", quote(value));
    let mut out = String::with_capacity(yaml.len() + line.len() + 1);
    let mut found = false;
    for raw in yaml.split_inclusive('\n') {
        if !found && value_of(raw.trim_end_matches(['\r', '\n']), key).is_some() {
            found = true;
            out.push_str(&line);
            out.push('\n');
        } else {
            out.push_str(raw);
        }
    }
    if !found {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Sets top-level `key` in a Markdown file's front matter, adding front
/// matter when the file has none.
#[must_use]
pub fn set_md(text: &str, key: &str, value: &str) -> String {
    match split(text) {
        Some((yaml, body)) => format!("---\n{}---\n{body}", set(yaml, key, value)),
        None => format!("---\n{}---\n{text}", set("", key, value)),
    }
}

/// The raw value text if `line` is the top-level `key:` line.
fn value_of<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(key)?.strip_prefix(':')?;
    if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
        return None;
    }
    Some(strip_comment(rest.trim()))
}

fn strip_comment(value: &str) -> &str {
    if value.starts_with(['"', '\'']) {
        return value;
    }
    value
        .find(" #")
        .map_or(value, |at| value.get(..at).unwrap_or(value))
        .trim_end()
}

fn unquote(value: &str) -> String {
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        return out;
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    value.to_owned()
}

/// Quotes `value` when YAML would otherwise read it as something else.
fn quote(value: &str) -> String {
    let plain = !value.is_empty()
        && value == value.trim()
        && !value.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%',
            '@', '`',
        ])
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.ends_with(':')
        && !value.chars().any(char::is_control)
        && !matches!(
            value.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
        )
        && value.parse::<f64>().is_err();
    if plain {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "---\nid: 01JB3K9Q7Z5N8M2X4V6T0R1Y3W\ntitle: Roadmap 2027 # shown in the tree\ntags: [a]\n---\n# Body\n";

    #[test]
    fn reads_keys() {
        assert_eq!(
            get_md(DOC, "id").as_deref(),
            Some("01JB3K9Q7Z5N8M2X4V6T0R1Y3W")
        );
        assert_eq!(get_md(DOC, "title").as_deref(), Some("Roadmap 2027"));
        assert_eq!(get_md(DOC, "owner"), None);
        assert_eq!(get_md("# no front matter", "id"), None);
    }

    #[test]
    fn sets_keys_in_place() {
        let out = set_md(DOC, "title", "Plan: 2028");
        assert_eq!(
            out,
            "---\nid: 01JB3K9Q7Z5N8M2X4V6T0R1Y3W\ntitle: \"Plan: 2028\"\ntags: [a]\n---\n# Body\n"
        );
        assert_eq!(get_md(&out, "title").as_deref(), Some("Plan: 2028"));
    }

    #[test]
    fn adds_front_matter_when_missing() {
        let out = set_md("# Hi\n", "id", "X");
        assert_eq!(out, "---\nid: X\n---\n# Hi\n");
    }

    #[test]
    fn nested_keys_are_ignored() {
        let yaml = "acl:\n  id: nested\nid: top\n";
        assert_eq!(get(yaml, "id").as_deref(), Some("top"));
        assert_eq!(get("identity: x\n", "id"), None);
    }

    #[test]
    fn quoting_round_trips() {
        for value in [
            "true",
            "12",
            "a \"quoted\" \\ word",
            "- dash",
            "",
            "it's",
            "#tag",
        ] {
            let yaml = set("", "title", value);
            assert_eq!(get(&yaml, "title").as_deref(), Some(value), "{yaml}");
        }
    }
}
