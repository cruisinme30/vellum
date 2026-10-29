// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The help books, built into the binary.
//!
//! The user, admin and developer guides live in `docs/*.book` as Vellum
//! books. `cargo xtask help write` generates [`pages`](crate) from them, so
//! every build carries the books that match its version: `vellum docs`
//! reads them offline, and the app's `?` button resolves a help ID such as
//! `editor.slash-menu` to a page and heading with [`resolve`].

use std::fmt;

#[rustfmt::skip]
mod pages;

/// One of the three help books.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Book {
    /// The admin guide: deploying, securing and running Vellum.
    Admin,
    /// The developer guide: architecture, APIs and contributing.
    Developer,
    /// The user guide, with every block and the cookbook.
    User,
}

impl Book {
    /// Every book, in the order they are presented.
    pub const ALL: [Self; 3] = [Self::User, Self::Admin, Self::Developer];

    /// The short name used in paths and on the command line: `user`.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Developer => "developer",
            Self::User => "user",
        }
    }

    /// The book's title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Admin => "Admin guide",
            Self::Developer => "Developer guide",
            Self::User => "User guide",
        }
    }

    /// The book with this short name.
    #[must_use]
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.slug() == slug)
    }

    /// The book's `SUMMARY.md`, which orders its pages.
    #[must_use]
    pub fn summary(self) -> &'static str {
        pages::SUMMARIES
            .iter()
            .find(|(book, _)| *book == self)
            .map_or("", |(_, text)| text)
    }

    /// The book's pages, in reading order.
    pub fn pages(self) -> impl Iterator<Item = &'static Page> {
        pages::PAGES.iter().filter(move |p| p.book == self)
    }
}

impl fmt::Display for Book {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}

/// A page of a help book.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    book: Book,
    path: &'static str,
    id: &'static str,
    title: &'static str,
    markdown: &'static str,
}

impl Page {
    /// The book it belongs to.
    #[must_use]
    pub const fn book(&self) -> Book {
        self.book
    }

    /// Its path inside the book, such as `writing/text.md`.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// Its ULID, from the front matter.
    #[must_use]
    pub const fn id(&self) -> &'static str {
        self.id
    }

    /// Its title, from the front matter.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        self.title
    }

    /// The page as Vellum Markdown, front matter included.
    #[must_use]
    pub const fn markdown(&self) -> &'static str {
        self.markdown
    }
}

/// A file a page refers to, such as an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    book: Book,
    path: &'static str,
    bytes: &'static [u8],
}

impl Asset {
    /// The book it belongs to.
    #[must_use]
    pub const fn book(&self) -> Book {
        self.book
    }

    /// Its path inside the book, such as `writing/assets/request-path.svg`.
    #[must_use]
    pub const fn path(&self) -> &'static str {
        self.path
    }

    /// Its contents.
    #[must_use]
    pub const fn bytes(&self) -> &'static [u8] {
        self.bytes
    }
}

/// Every page of every book.
#[must_use]
pub fn pages() -> &'static [Page] {
    pages::PAGES
}

/// The page at `path` in `book`.
#[must_use]
pub fn page(book: Book, path: &str) -> Option<&'static Page> {
    pages::PAGES
        .iter()
        .find(|p| p.book == book && p.path == path)
}

/// The asset at `path` in `book`.
#[must_use]
pub fn asset(book: Book, path: &str) -> Option<&'static Asset> {
    pages::ASSETS
        .iter()
        .find(|a| a.book == book && a.path == path)
}

/// Where a help ID points: a page and, usually, a heading anchor on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HelpTarget {
    /// The page.
    pub page: &'static Page,
    /// The heading anchor, without `#`.
    pub anchor: Option<&'static str>,
}

/// Every contextual help ID the app may ask for.
pub fn help_ids() -> impl Iterator<Item = &'static str> {
    pages::ANCHORS.iter().map(|(id, _)| *id)
}

/// The page and heading for a contextual help ID such as
/// `editor.slash-menu`, as listed in `docs/help-anchors.yaml`.
#[must_use]
pub fn resolve(help_id: &str) -> Option<HelpTarget> {
    let (_, target) = pages::ANCHORS.iter().find(|(id, _)| *id == help_id)?;
    let (path, anchor) = target
        .split_once('#')
        .map_or((*target, None), |(p, a)| (p, Some(a)));
    let (book, path) = path.split_once('/')?;
    let page = page(Book::from_slug(book)?, path)?;
    Some(HelpTarget { page, anchor })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_book_has_a_summary_and_an_introduction() {
        for book in Book::ALL {
            assert!(book.summary().starts_with("# Summary"), "{book}");
            assert!(page(book, "README.md").is_some(), "{book}");
            assert_eq!(Book::from_slug(book.slug()), Some(book));
        }
    }

    #[test]
    fn pages_carry_their_front_matter() {
        for page in pages() {
            assert!(page.markdown().starts_with("---\n"), "{}", page.path());
            assert!(page.markdown().contains(page.id()), "{}", page.path());
            assert_eq!(page.id().len(), 26, "{}", page.path());
            assert!(!page.title().is_empty(), "{}", page.path());
        }
        assert!(Book::User.pages().count() > 30);
    }

    #[test]
    fn every_help_id_resolves_to_a_heading() {
        let mut count = 0;
        for id in help_ids() {
            let target = resolve(id);
            assert!(target.is_some(), "{id}");
            // `cargo xtask help` checks that each anchor is a heading.
            if let Some(HelpTarget {
                anchor: Some(anchor),
                ..
            }) = target
            {
                assert!(!anchor.is_empty() && !anchor.contains('#'), "{id}");
            }
            count += 1;
        }
        assert!(count > 50);
        assert_eq!(resolve("no.such.id"), None);
    }

    #[test]
    fn assets_are_embedded() {
        let svg = asset(Book::User, "writing/assets/request-path.svg");
        assert!(svg.is_some_and(|a| a.bytes().starts_with(b"<?xml")));
        assert_eq!(svg.map(Asset::book), Some(Book::User));
        assert!(svg.is_some_and(|a| {
            std::path::Path::new(a.path())
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("svg"))
        }));
    }
}
