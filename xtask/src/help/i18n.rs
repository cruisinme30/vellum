// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Translated books (R-HG-5, ADR 0024 D102 and D258).
//!
//! A translation is a copy of a book at `docs/i18n/<locale>/<name>.book`,
//! with the same paths, page IDs and heading anchors as the English book,
//! so a help ID resolves in every language and a missing page falls back
//! to English. It may translate only some pages.

use std::fs;
use std::path::Path;

use super::book::Book;
use super::markdown;

/// Where translations live, under the workspace root.
pub const I18N_DIR: &str = "docs/i18n";

/// Checks every translation under `docs/i18n` against `books`.
pub fn check(root: &Path, books: &[Book], errors: &mut Vec<String>) -> Result<(), String> {
    let dir = root.join(I18N_DIR);
    let Ok(locales) = fs::read_dir(&dir) else {
        return Ok(());
    };
    let mut locales: Vec<_> = locales.filter_map(Result::ok).map(|e| e.path()).collect();
    locales.sort();
    for locale_dir in locales.iter().filter(|p| p.is_dir()) {
        let locale = locale_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if !is_locale(locale) {
            errors.push(format!(
                "{I18N_DIR}/{locale}: not a locale such as `de` or `pt-BR`"
            ));
            continue;
        }
        let mut dirs: Vec<_> = fs::read_dir(locale_dir)
            .map_err(|e| format!("cannot list {}: {e}", locale_dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        dirs.sort();
        for book_dir in dirs {
            let shown = format!(
                "{I18N_DIR}/{locale}/{}",
                book_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
            );
            if !book_dir.is_dir() || book_dir.extension().is_none_or(|x| x != "book") {
                errors.push(format!("{shown}: only <name>.book folders belong here"));
                continue;
            }
            let translated = Book::load(&book_dir)?;
            match books.iter().find(|b| b.name == translated.name) {
                Some(english) => compare(&shown, english, &translated, errors),
                None => errors.push(format!("{shown}: there is no English book of that name")),
            }
        }
    }
    Ok(())
}

/// The ways `translated` may not differ from `english`.
pub fn compare(shown: &str, english: &Book, translated: &Book, errors: &mut Vec<String>) {
    if translated.manifest.id != english.manifest.id {
        errors.push(format!(
            "{shown}/book.yaml: `id` must be the English book's"
        ));
    }
    for entry in &translated.entries {
        if let Some(path) = &entry.path
            && english.page(path).is_none()
        {
            errors.push(format!(
                "{shown}/SUMMARY.md: `{path}` is not a page of the English book"
            ));
        }
    }
    for page in &translated.pages {
        let Some(original) = english.page(&page.path) else {
            continue;
        };
        let location = format!("{shown}/{}", page.path);
        let id = |p: &super::book::Page| p.front.as_ref().and_then(|f| f.id.clone());
        if id(page) != id(original) {
            errors.push(format!("{location}: `id` must be the English page's"));
        }
        let anchors = markdown::anchors(&page.text);
        let wanted: std::collections::BTreeSet<String> =
            markdown::anchors(&original.text).into_iter().collect();
        for anchor in wanted {
            if !anchors.contains(&anchor) {
                errors.push(format!(
                    "{location}: keep the heading anchor `{{#{anchor}}}` from the English page"
                ));
            }
        }
    }
}

/// A BCP 47 tag of the shape the books use: `de`, `pt-BR`, `zh-Hant`.
fn is_locale(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let language = parts.next().unwrap_or_default();
    let ok_language =
        (2..=3).contains(&language.len()) && language.chars().all(|c| c.is_ascii_lowercase());
    ok_language
        && parts.all(|p| {
            (p.len() == 2 && p.chars().all(|c| c.is_ascii_uppercase()))
                || (p.len() == 4
                    && p.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    && p.chars().skip(1).all(|c| c.is_ascii_lowercase()))
        })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    fn book(dir: &Path, name: &str, id: &str, pages: &[(&str, &str)]) -> Book {
        let book_dir = dir.join(format!("{name}.book"));
        fs::create_dir_all(&book_dir).unwrap();
        let mut summary = String::from("# Summary\n\n");
        let mut nodes = String::new();
        for (path, text) in pages {
            let _ = writeln!(summary, "- [P]({path})");
            fs::write(book_dir.join(path), text).unwrap();
            let _ = write!(nodes, "  X:\n    path: {path}\n");
        }
        fs::write(book_dir.join("SUMMARY.md"), summary).unwrap();
        fs::write(
            book_dir.join("book.yaml"),
            format!("vellum: 1\nid: {id}\nnodes:\n{nodes}"),
        )
        .unwrap();
        Book::load(&book_dir).unwrap()
    }

    const PAGE: &str =
        "---\nid: 01M63CFE67X46N7KX44C7WV4HP\ntitle: Start\nvellum: 1\n---\n\n## Share {#share}\n";

    #[test]
    fn a_translation_keeps_ids_paths_and_anchors() {
        let dir = std::env::temp_dir().join(format!("vellum-i18n-{}", std::process::id()));
        let english = book(
            &dir.join("en"),
            "user",
            "01M63C8QH9FN872JSH5DQNY6E9",
            &[("a.md", PAGE)],
        );
        let good = book(
            &dir.join("de"),
            "user",
            "01M63C8QH9FN872JSH5DQNY6E9",
            &[("a.md", &PAGE.replace("## Share", "## Teilen"))],
        );
        let mut errors = Vec::new();
        compare("de/user.book", &english, &good, &mut errors);
        assert_eq!(errors, Vec::<String>::new());

        let bad = book(
            &dir.join("fr"),
            "user",
            "01M63C8QH9FN872JSH5DQNY6E0",
            &[
                (
                    "a.md",
                    &PAGE
                        .replace("## Share {#share}", "## Partager {#partager}")
                        .replace("X44C7WV4HP", "X44C7WV4HQ"),
                ),
                ("b.md", PAGE),
            ],
        );
        compare("fr/user.book", &english, &bad, &mut errors);
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(errors.len(), 4, "{errors:#?}");
        assert!(errors.iter().any(|e| e.contains("`{#share}`")));
        assert!(errors.iter().any(|e| e.contains("`b.md` is not a page")));
    }

    #[test]
    fn locale_tags() {
        for good in ["de", "pt-BR", "zh-Hant", "zh-Hant-TW", "fil"] {
            assert!(is_locale(good), "{good}");
        }
        for bad in ["", "DE", "pt_br", "english", "pt-br", "../x"] {
            assert!(!is_locale(bad), "{bad}");
        }
    }
}
