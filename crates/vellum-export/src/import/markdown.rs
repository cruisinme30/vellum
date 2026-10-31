// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Markdown folders: plain Markdown, Obsidian vaults, Outline and
//! Docmost exports.
//!
//! Every `.md` file becomes a page at the same place in the folder tree,
//! and every other file an attachment. Obsidian `![[embeds]]` become
//! images or wiki links; Outline and Docmost `:::warning`-style notices
//! become Vellum callouts.

use super::{Builder, Source, archive, markdown_title};
use crate::error::ImportError;

pub(super) fn import(
    source: Source,
    bytes: &[u8],
) -> Result<Builder, ImportError> {
    let mut entries = archive::read(source.name(), bytes, false)?;
    archive::strip_common_root(&mut entries);
    let mut builder = Builder::new(source.name());
    let mut pages = 0;
    for entry in entries {
        if entry
            .path
            .split('/')
            .any(|segment| segment.starts_with('.'))
        {
            // `.obsidian/`, `.git/`, `.trash/` and other tool folders.
            continue;
        }
        let lower = entry.path.to_lowercase();
        if super::has_ext(&lower, &["md", "markdown"]) {
            let text = String::from_utf8_lossy(&entry.bytes).into_owned();
            let text = match source {
                Source::Obsidian => obsidian(&text),
                Source::Outline | Source::Docmost => notices(&text),
                _ => text,
            };
            let stem = super::stem(super::file_name(&entry.path));
            let title = markdown_title(&text, stem);
            let out = format!("{}.md", super::stem(&entry.path));
            let _ = builder.page(&entry.path, &out, &title, text);
            pages += 1;
        } else {
            builder.asset(&entry.path, &entry.path, entry.bytes);
        }
    }
    if pages == 0 {
        return Err(ImportError::format(
            source.name(),
            "the archive has no Markdown files",
        ));
    }
    Ok(builder)
}

/// Obsidian embeds: `![[diagram.png|300]]` becomes an image and
/// `![[Other note]]` a wiki link (Vellum has no transclusion).
fn obsidian(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("![[") {
        let Some(len) = rest.get(start + 3..).and_then(|r| r.find("]]")) else {
            break;
        };
        out.push_str(rest.get(..start).unwrap_or_default());
        let inner = rest.get(start + 3..start + 3 + len).unwrap_or_default();
        let (target, _) = inner.split_once('|').unwrap_or((inner, ""));
        let target = target.trim();
        let is_file = target.rsplit_once('.').is_some_and(|(_, ext)| {
            !ext.eq_ignore_ascii_case("md") && ext.len() <= 5
        });
        if is_file {
            out.push_str("![](<");
            out.push_str(target);
            out.push_str(">)");
        } else {
            out.push_str("[[");
            out.push_str(target);
            out.push_str("]]");
        }
        rest = rest.get(start + 3 + len + 2..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

/// Outline and Docmost notices use `:::info`, `:::warning`, `:::tip`,
/// `:::success` and `:::danger`; Vellum's callouts are `info`, `warn`,
/// `tip`, `good`, `crit` and `note`.
fn notices(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end();
        let name = trimmed.strip_prefix(":::").map(str::trim);
        let mapped = match name {
            Some("warning") => Some("warn"),
            Some("success") => Some("good"),
            Some("danger" | "error") => Some("crit"),
            Some("notice") => Some("note"),
            _ => None,
        };
        match mapped {
            Some(name) => {
                out.push_str(":::");
                out.push_str(name);
                out.push_str(line.get(trimmed.len()..).unwrap_or_default());
            }
            None => out.push_str(line),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obsidian_embeds_become_images_or_links() {
        assert_eq!(
            obsidian("See ![[flow chart.png|300]] and ![[Runbook]]."),
            "See ![](<flow chart.png>) and [[Runbook]]."
        );
        assert_eq!(obsidian("no embeds ![[unclosed"), "no embeds ![[unclosed");
    }

    #[test]
    fn notices_map_to_callouts() {
        assert_eq!(
            notices(":::warning\nCareful\n:::\n:::success\nDone\n:::\n"),
            ":::warn\nCareful\n:::\n:::good\nDone\n:::\n"
        );
    }
}
