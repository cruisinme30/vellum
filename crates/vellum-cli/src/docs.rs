// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum docs`: read the help books built into this binary.

use std::io::{self, Write};

use clap::Args;
use vellum_help::Book;

/// Arguments of `vellum docs`.
#[derive(Debug, Args)]
pub struct DocsArgs {
    /// The book: user, admin or developer. Without it, lists every book.
    book: Option<String>,
    /// A page path inside the book, such as `writing/text.md`. Without it,
    /// prints the book's table of contents.
    page: Option<String>,
    /// Print the page for an in-app help ID, such as `editor.slash-menu`.
    #[arg(long, conflicts_with_all = ["book", "page"])]
    help_id: Option<String>,
}

/// Why `vellum docs` could not print what was asked.
#[derive(Debug, thiserror::Error)]
pub enum DocsError {
    /// No book has that name.
    #[error("no book named `{0}`; the books are user, admin and developer")]
    UnknownBook(String),
    /// The book has no page at that path.
    #[error("the {book} guide has no page `{page}`; run `vellum docs {book}` to list its pages")]
    UnknownPage {
        /// The book.
        book: Book,
        /// The path asked for.
        page: String,
    },
    /// No help ID with that name.
    #[error("no help ID `{0}`")]
    UnknownHelpId(String),
    /// Writing the output failed.
    #[error("cannot write output: {0}")]
    Io(#[from] io::Error),
}

/// Runs `vellum docs`, writing to `out`.
pub fn run(args: &DocsArgs, out: &mut impl Write) -> Result<(), DocsError> {
    if let Some(id) = &args.help_id {
        let target =
            vellum_help::resolve(id).ok_or_else(|| DocsError::UnknownHelpId(id.clone()))?;
        let page = target.page;
        writeln!(
            out,
            "# {} guide: {}{}\n",
            page.book(),
            page.path(),
            target.anchor.map(|a| format!("#{a}")).unwrap_or_default()
        )?;
        out.write_all(page.markdown().as_bytes())?;
        return Ok(());
    }
    let Some(name) = &args.book else {
        for book in Book::ALL {
            writeln!(
                out,
                "{:<10} {} ({} pages)",
                book.slug(),
                book.title(),
                book.pages().count()
            )?;
        }
        writeln!(out, "\nRun `vellum docs <book>` for its contents.")?;
        return Ok(());
    };
    let book = Book::from_slug(name).ok_or_else(|| DocsError::UnknownBook(name.clone()))?;
    let Some(path) = &args.page else {
        writeln!(out, "{}\n", book.title())?;
        for page in book.pages() {
            writeln!(out, "{:<40} {}", page.path(), page.title())?;
        }
        return Ok(());
    };
    let page = vellum_help::page(book, path).ok_or_else(|| DocsError::UnknownPage {
        book,
        page: path.clone(),
    })?;
    out.write_all(page.markdown().as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(
        book: Option<&str>,
        page: Option<&str>,
        help_id: Option<&str>,
    ) -> Result<String, DocsError> {
        let args = DocsArgs {
            book: book.map(str::to_owned),
            page: page.map(str::to_owned),
            help_id: help_id.map(str::to_owned),
        };
        let mut out = Vec::new();
        run(&args, &mut out)?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    #[test]
    fn lists_books_and_pages() -> Result<(), DocsError> {
        let books = output(None, None, None)?;
        assert!(books.contains("user") && books.contains("Admin guide"));
        let contents = output(Some("user"), None, None)?;
        assert!(contents.contains("writing/text.md"));
        Ok(())
    }

    #[test]
    fn prints_a_page_and_a_help_target() -> Result<(), DocsError> {
        let page = output(Some("admin"), Some("reference/cli.md"), None)?;
        assert!(page.contains("# Command reference"));
        let help = output(None, None, Some("editor.slash-menu"))?;
        assert!(help.starts_with("# user guide: start/README.md#slash-menu"));
        Ok(())
    }

    #[test]
    fn explains_unknown_names() {
        assert!(matches!(
            output(Some("nope"), None, None),
            Err(DocsError::UnknownBook(_))
        ));
        assert!(matches!(
            output(Some("user"), Some("nope.md"), None),
            Err(DocsError::UnknownPage { .. })
        ));
        assert!(matches!(
            output(None, None, Some("nope")),
            Err(DocsError::UnknownHelpId(_))
        ));
    }
}
