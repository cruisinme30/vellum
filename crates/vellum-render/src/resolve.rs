// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Links to the rest of the workspace: wikilinks, mentions, issue keys,
//! subpages and collection views.
//!
//! The renderer never reads the index itself; the caller passes a
//! [`Resolver`] that already applies the reader's permissions. Anything it
//! cannot resolve renders as a readable fallback with a `data-*` hook the
//! client can hydrate later.

/// A resolved page link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// Where the link goes.
    pub href: String,
    /// The page's current title.
    pub title: String,
}

/// A resolved issue key, shown as a live chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueRef {
    /// Where the issue lives.
    pub href: String,
    /// The issue title.
    pub title: String,
    /// The state's display name (`In review`).
    pub state: String,
}

/// A snapshot of a collection view, rendered as a table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CollectionSnapshot {
    /// The view's display name.
    pub title: String,
    /// Column names.
    pub header: Vec<String>,
    /// Rows of cell text.
    pub rows: Vec<Vec<String>>,
}

/// Which `::: issues` block of the page a view is for.
///
/// The renderer never reads the block's YAML: the Markdown parser drops the
/// indentation that nested filters need. The resolver reads the raw body of
/// the `index`th `::: issues` block of the source instead, counting in
/// document order, skipping code fences and including containers nested in
/// other containers. (Containers do not open inside quotes or lists.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IssuesAt<'a> {
    /// The block's position among the page's `::: issues` blocks, from 0.
    pub index: usize,
    /// The block's `^id`, when it has one.
    pub block_id: Option<&'a str>,
}

/// How an issue view is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IssueLayout {
    /// Rows, grouped into sections.
    #[default]
    List,
    /// Columns, one per group.
    Board,
}

/// One issue in an embedded view.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueCard {
    /// The issue key (`ENG-142`).
    pub key: String,
    /// Where the issue lives.
    pub href: String,
    /// The issue title.
    pub title: String,
    /// The state's display name (`In review`).
    pub state: String,
    /// The assignee's display name.
    pub assignee: Option<String>,
    /// The priority's display name (`Urgent`).
    pub priority: Option<String>,
}

/// A group of issues: a list section or a board column.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueGroup {
    /// The group's display name; empty when the view is not grouped.
    pub title: String,
    /// The issues, in order.
    pub issues: Vec<IssueCard>,
}

/// The issues an `::: issues` block shows to this reader, already filtered
/// by their permissions and capped by the block's limit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueView {
    /// A heading, such as the saved view's name.
    pub title: Option<String>,
    /// List or board.
    pub layout: IssueLayout,
    /// The groups, in order.
    pub groups: Vec<IssueGroup>,
    /// How many matching issues the limit left out.
    pub more: usize,
}

/// Resolves references against the workspace for one reader.
///
/// Every method has a default, so a resolver only implements what it knows.
pub trait Resolver: Send + Sync {
    /// A `[[Title]]`, `[[#id]]` or `vellum://doc/<id>` wikilink target.
    fn wikilink(&self, target: &str) -> Option<Resolved> {
        target.starts_with("vellum://").then(|| Resolved {
            href: self.href(target).unwrap_or_default(),
            title: target.to_owned(),
        })
    }

    /// Maps an internal `vellum://` URL to a browser URL. The default maps
    /// `vellum://<kind>/<rest>` to `/<k>/<rest>` with the kind's short
    /// letter (doc → d, book → b, issue → i, collection → c, space → s).
    fn href(&self, url: &str) -> Option<String> {
        let rest = url.strip_prefix("vellum://")?;
        let (kind, path) = rest.split_once('/')?;
        let short = match kind {
            "doc" => "d",
            "book" => "b",
            "issue" => "i",
            "collection" => "c",
            "space" => "s",
            _ => return None,
        };
        let safe = !path.is_empty()
            && path.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '-' | '_' | '.' | '/' | '?' | '=' | '&' | '#' | '%')
            });
        safe.then(|| format!("/{short}/{path}"))
    }

    /// The display name for `@handle`.
    fn mention(&self, _handle: &str) -> Option<String> {
        None
    }

    /// The issue for a candidate key such as `ENG-142`; `None` renders the
    /// key as plain text.
    fn issue(&self, _key: &str) -> Option<IssueRef> {
        None
    }

    /// The current page's subpages, for an empty `::: pages`.
    fn subpages(&self) -> Vec<Resolved> {
        Vec::new()
    }

    /// The current rows of a collection view, for a `collection` fence.
    fn collection(&self, _id: &str, _view: &str) -> Option<CollectionSnapshot> {
        None
    }

    /// The live issue view of an `::: issues` block. `None` renders a
    /// placeholder the client fills in; `Err` explains why the block is
    /// invalid (shown as a warning).
    fn issues(&self, _at: IssuesAt<'_>) -> Option<Result<IssueView, String>> {
        None
    }
}

/// Resolves nothing beyond `vellum://` URLs.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoResolver;

impl Resolver for NoResolver {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_internal_urls() {
        let r = NoResolver;
        assert_eq!(
            r.href("vellum://doc/01JB3K9Q7Z5N8M2X4V6T0R1Y3W").as_deref(),
            Some("/d/01JB3K9Q7Z5N8M2X4V6T0R1Y3W")
        );
        assert_eq!(
            r.href("vellum://issue/ENG-142").as_deref(),
            Some("/i/ENG-142")
        );
        assert_eq!(r.href("vellum://doc/\"><x"), None);
        assert_eq!(r.href("vellum://nope/x"), None);
        assert_eq!(r.wikilink("Gateway architecture"), None);
    }
}
