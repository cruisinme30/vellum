// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A Vellum book builds with unmodified mdBook (D14, R-BK-10): both the
//! raw book folder and the published mdBook tree.
//!
//! CI installs `mdbook`; locally the test is skipped with a message when it
//! is not on the `PATH`.

#![allow(clippy::unwrap_used, reason = "test helpers fail the test on error")]

use std::path::{Path, PathBuf};
use std::process::Command;

use vellum_books::fs::{save_dir, write_site};
use vellum_books::ops::Place;
use vellum_books::publish::Audience;
use vellum_books::summary::Section;
use vellum_books::{Book, BookId, IdSource, SequentialIds};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir()
            .join(format!("vellum-mdbook-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mdbook_available() -> bool {
    let found = Command::new("mdbook").arg("--version").output().is_ok();
    assert!(
        found || std::env::var_os("CI").is_none(),
        "mdbook must be installed in CI"
    );
    if !found {
        eprintln!("mdbook is not installed; skipping the mdBook build check");
    }
    found
}

fn build(dir: &Path) {
    let output = Command::new("mdbook")
        .arg("build")
        .arg(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mdbook build failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn northstar() -> Book {
    let mut ids = SequentialIds::new("01JBK0000T");
    let mut book = Book::new(
        BookId::parse("01JBK00000000000000000B00K").unwrap(),
        "Northstar handbook",
    );
    let intro = book
        .add_section(
            &Place::End(Section::Prefix),
            "Introduction",
            "Welcome to Northstar.",
            &mut ids,
        )
        .unwrap();
    let onboarding = book
        .add_section(
            &Place::End(Section::Numbered),
            "Onboarding",
            "How we join.",
            &mut ids,
        )
        .unwrap();
    let architecture = book
        .add_section(
            &Place::End(Section::Numbered),
            "Architecture",
            "The big picture.",
            &mut ids,
        )
        .unwrap();
    let services_id = ids.next_id();
    let week = book
        .add_section(
            &Place::LastChildOf(onboarding.clone().into()),
            "Your first week",
            &format!("Read [§]({}) first.", services_id.uri()),
            &mut ids,
        )
        .unwrap();
    let _ = book
        .add_draft(&Place::LastChildOf(onboarding.into()), "Security training")
        .unwrap();
    let mut fixed = move || services_id.clone();
    let _ = book
        .add_section(
            &Place::LastChildOf(architecture.into()),
            "Services",
            "Gateway, pages and search.",
            &mut fixed,
        )
        .unwrap();
    let _ = book
        .add_section(
            &Place::End(Section::Suffix),
            "Glossary",
            "Terms.",
            &mut ids,
        )
        .unwrap();
    book.add_part(0, "Part I · Joining").unwrap();
    let _ = (intro, week);
    book
}

#[test]
fn raw_book_folder_builds_with_mdbook() {
    if !mdbook_available() {
        return;
    }
    let tmp = TempDir::new("raw");
    let dir = tmp.0.join("handbook.book");
    let mut book = northstar();
    let _ = save_dir(&mut book, &dir).unwrap();
    build(&dir);
    let page = std::fs::read_to_string(
        dir.join(".mdbook/onboarding/your-first-week.html"),
    )
    .unwrap();
    assert!(page.contains("Your first week"));
}

#[test]
fn published_tree_builds_with_mdbook_and_resolves_cross_refs() {
    if !mdbook_available() {
        return;
    }
    let tmp = TempDir::new("published");
    let site = northstar().to_mdbook(&Audience::Public).unwrap();
    write_site(&tmp.0, &site).unwrap();
    build(&tmp.0);
    let page = std::fs::read_to_string(
        tmp.0.join("book/onboarding/your-first-week.html"),
    )
    .unwrap();
    assert!(
        page.contains("§2.1 Services"),
        "cross-reference label missing"
    );
    assert!(
        page.contains("../architecture/services.html"),
        "cross-reference link missing"
    );
    assert!(
        !page.contains("vellum: 1"),
        "front matter leaked into the page"
    );
    // mdBook's own numbering agrees with Vellum's.
    let toc = std::fs::read_to_string(tmp.0.join("book/toc.html"))
        .unwrap_or_default();
    let index = std::fs::read_to_string(tmp.0.join("book/index.html")).unwrap();
    assert!(
        toc.contains("2.1.") || index.contains("2.1."),
        "mdBook numbering differs"
    );
}
