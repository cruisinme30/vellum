// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Confluence space exports (HTML).
//!
//! The export has one HTML file per page, `index.html` with the page tree
//! as nested lists, and attachments under `attachments/<page id>/`. Pages
//! are written as a folder tree that follows the page tree
//! (`Home/Runbooks/Restart.md`), with the space's breadcrumbs, footer
//! and attachment listing left out.

use std::collections::HashMap;

use scraper::{ElementRef, Html, Selector};
use vellum_format::{Attributes, Block, Document, Inline};

use super::{Builder, archive, html};
use crate::error::ImportError;

const FORMAT: &str = "confluence";

pub(super) fn import(bytes: &[u8]) -> Result<Builder, ImportError> {
    let mut entries = archive::read(FORMAT, bytes, false)?;
    archive::strip_common_root(&mut entries);
    let parents = entries
        .iter()
        .find(|e| e.path == "index.html")
        .map(|index| tree(&String::from_utf8_lossy(&index.bytes)))
        .unwrap_or_default();

    let mut builder = Builder::new(FORMAT);
    let mut titles: HashMap<String, String> = HashMap::new();
    let mut pages = Vec::new();
    for entry in &entries {
        let lower = entry.path.to_lowercase();
        if lower == "index.html" || !super::has_ext(&lower, &["html", "htm"]) {
            continue;
        }
        let text = String::from_utf8_lossy(&entry.bytes);
        let page = Html::parse_document(&text);
        let title = title(&page)
            .unwrap_or_else(|| super::stem(&entry.path).replace('_', " "));
        let _ = titles.insert(entry.path.clone(), title.clone());
        pages.push((entry.path.clone(), title, page));
    }
    if pages.is_empty() {
        return Err(ImportError::format(
            FORMAT,
            "the archive has no HTML pages",
        ));
    }
    for (path, title, page) in pages {
        let mut folders = Vec::new();
        let mut current = parents.get(&path);
        while let Some(parent) = current {
            if folders.len() > 32 || folders.contains(parent) {
                break;
            }
            folders.push(parent.clone());
            current = parents.get(parent);
        }
        let mut out: Vec<String> = folders
            .iter()
            .rev()
            .map(|p| {
                titles
                    .get(p)
                    .cloned()
                    .unwrap_or_else(|| super::stem(p).to_owned())
            })
            .map(|t| t.replace('/', "-"))
            .collect();
        out.push(format!("{}.md", title.replace('/', "-")));
        let mut blocks = vec![Block::Heading {
            level: 1,
            content: vec![Inline::Text {
                text: title.clone(),
            }],
            attrs: Attributes::default(),
            id: None,
        }];
        if let Some(main) = main(&page) {
            blocks.extend(html::blocks(main));
        }
        let text = vellum_format::serialize(&Document {
            front_matter: None,
            blocks,
        });
        let _ = builder.page(&path, &out.join("/"), &title, text);
    }
    for entry in entries {
        let lower = entry.path.to_lowercase();
        if super::has_ext(&lower, &["html", "htm"])
            || lower.starts_with("styles/")
            || lower.starts_with("images/icons/")
        {
            continue;
        }
        builder.asset(&entry.path, &entry.path, entry.bytes);
    }
    Ok(builder)
}

/// Child page → parent page, from `index.html`'s nested lists.
fn tree(index: &str) -> HashMap<String, String> {
    let doc = Html::parse_document(index);
    let mut parents = HashMap::new();
    for li in doc.select(&li_selector()) {
        let Some(own) = link_of(li) else {
            continue;
        };
        let parent = li
            .ancestors()
            .filter_map(ElementRef::wrap)
            .filter(|a| a.value().name() == "li")
            .find_map(link_of);
        if let Some(parent) = parent {
            let _ = parents.insert(own, parent);
        }
    }
    parents
}

/// The page a list item links to (its own link, not a child's).
fn link_of(li: ElementRef<'_>) -> Option<String> {
    li.children()
        .filter_map(ElementRef::wrap)
        .find(|c| c.value().name() == "a")
        .and_then(|a| a.value().attr("href"))
        .map(|href| super::links::split(href).0)
        .filter(|href| !href.is_empty())
}

fn title(page: &Html) -> Option<String> {
    let text = page
        .select(&title_text_selector())
        .next()
        .or_else(|| page.select(&title_selector()).next())?
        .text()
        .collect::<String>();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    // "Space name : Page title"
    let title = text
        .split_once(" : ")
        .map_or(text.as_str(), |(_, t)| t)
        .trim();
    (!title.is_empty()).then(|| title.to_owned())
}

fn main(page: &Html) -> Option<ElementRef<'_>> {
    page.select(&main_selector())
        .next()
        .or_else(|| page.select(&body_selector()).next())
}

macro_rules! selector {
    ($name:ident, $css:literal) => {
        fn $name() -> Selector {
            static SELECTOR: std::sync::OnceLock<Selector> =
                std::sync::OnceLock::new();
            SELECTOR
                .get_or_init(|| {
                    Selector::parse($css)
                        .unwrap_or_else(|_| unreachable!("valid selector"))
                })
                .clone()
        }
    };
}

selector!(li_selector, "li");
selector!(title_text_selector, "#title-text");
selector!(title_selector, "title");
selector!(main_selector, "#main-content");
selector!(body_selector, "body");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_tree_comes_from_the_index() {
        let parents = tree(
            r#"<div class="pageSection"><ul><li><a href="Home_1.html">Home</a>
            <ul><li><a href="Runbooks_2.html">Runbooks</a><ul><li><a href="Restart_3.html">Restart</a></li></ul></li></ul>
            </li></ul></div>"#,
        );
        assert_eq!(
            parents.get("Runbooks_2.html").map(String::as_str),
            Some("Home_1.html")
        );
        assert_eq!(
            parents.get("Restart_3.html").map(String::as_str),
            Some("Runbooks_2.html")
        );
        assert!(!parents.contains_key("Home_1.html"));
    }

    #[test]
    fn titles_drop_the_space_name() {
        let page = Html::parse_document("<html><head><title>Ops : \
            Restart</title></head><body><h1 id=\"title-heading\"><span \
            id=\"title-text\"> Ops : Restart the gateway \
            </span></h1></body></html>");
        assert_eq!(title(&page).as_deref(), Some("Restart the gateway"));
    }
}
