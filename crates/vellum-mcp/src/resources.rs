// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Resources: `vellum://` URIs read as Markdown.

use serde_json::{Map, Value, json};

use crate::ports::{Actor, Document, Workspace, WorkspaceError};
use crate::protocol::RpcError;
use crate::safety;

/// Freshness hint for resource reads, in milliseconds. Reads are
/// per-user, so they are also `private`.
pub const READ_TTL_MS: u64 = 30_000;

/// Freshness hint for the per-user resource list.
pub const LIST_TTL_MS: u64 = 60_000;

/// Freshness hint for static lists (templates, tools, prompts, discover).
pub const STATIC_TTL_MS: u64 = 3_600_000;

/// The resource templates, in a fixed order.
#[must_use]
pub fn templates() -> Value {
    json!([
        {
            "uriTemplate": "vellum://doc/{id}",
            "name": "doc",
            "title": "Page",
            "description": "A page as Vellum Markdown with front matter.",
            "mimeType": "text/markdown"
        },
        {
            "uriTemplate": "vellum://book/{id}",
            "name": "book",
            "title": "Book",
            "description": "A book: its manifest, numbered table of contents \
                and section ids.",
            "mimeType": "text/markdown"
        },
        {
            "uriTemplate": "vellum://issue/{key}",
            "name": "issue",
            "title": "Issue",
            "description": "An issue such as ENG-142, with its front matter \
                and description.",
            "mimeType": "text/markdown"
        },
        {
            "uriTemplate": "vellum://collection/{id}{?view}",
            "name": "collection",
            "title": "Collection view",
            "description": "A collection's rows as a Markdown table, \
                optionally through a saved view.",
            "mimeType": "text/markdown"
        },
        {
            "uriTemplate": "vellum://space/{slug}/tree",
            "name": "space-tree",
            "title": "Space tree",
            "description": "Every page, book and collection in a space as a \
                nested list of links.",
            "mimeType": "text/markdown"
        }
    ])
}

/// A parsed `vellum://` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VellumUri {
    /// `vellum://doc/{id}`
    Doc(String),
    /// `vellum://book/{id}`
    Book(String),
    /// `vellum://issue/{KEY-n}`
    Issue(String),
    /// `vellum://collection/{id}?view=…`
    Collection {
        /// Collection id.
        id: String,
        /// Saved view.
        view: Option<String>,
    },
    /// `vellum://space/{slug}/tree`
    SpaceTree(String),
}

fn segment_ok(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn is_issue_key(s: &str) -> bool {
    s.split_once('-').is_some_and(|(team, n)| {
        !team.is_empty()
            && team
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
    })
}

fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'%' => {
                let hex = s.get(i + 1..i + 3)?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

impl VellumUri {
    /// Parses a `vellum://` URI.
    #[must_use]
    pub fn parse(uri: &str) -> Option<Self> {
        let rest = uri.strip_prefix("vellum://")?;
        let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["doc", id] if segment_ok(id) && query.is_empty() => {
                Some(Self::Doc((*id).to_owned()))
            }
            ["book", id] if segment_ok(id) && query.is_empty() => {
                Some(Self::Book((*id).to_owned()))
            }
            ["issue", key] if is_issue_key(key) && query.is_empty() => {
                Some(Self::Issue((*key).to_owned()))
            }
            ["space", slug, "tree"] if segment_ok(slug) && query.is_empty() => {
                Some(Self::SpaceTree((*slug).to_owned()))
            }
            ["collection", id] if segment_ok(id) => {
                let mut view = None;
                for pair in query.split('&').filter(|p| !p.is_empty()) {
                    match pair.split_once('=') {
                        Some(("view", v)) => view = Some(decode(v)?),
                        _ => return None,
                    }
                }
                Some(Self::Collection {
                    id: (*id).to_owned(),
                    view,
                })
            }
            _ => None,
        }
    }

    /// The id to record in the audit log.
    #[must_use]
    pub fn subject(&self) -> &str {
        match self {
            Self::Doc(id)
            | Self::Book(id)
            | Self::Issue(id)
            | Self::SpaceTree(id)
            | Self::Collection { id, .. } => id,
        }
    }
}

/// Reads a resource for an actor.
///
/// # Errors
///
/// `-32602` for an unknown or unreadable URI (indistinguishable on
/// purpose), and the matching JSON-RPC error for an unavailable backend.
pub async fn read(
    ws: &dyn Workspace,
    actor: &Actor,
    uri: &str,
) -> Result<Map<String, Value>, RpcError> {
    let parsed = VellumUri::parse(uri).ok_or_else(|| not_found(uri))?;
    let doc: Result<Document, WorkspaceError> = match &parsed {
        VellumUri::Doc(id) => ws.get_doc(actor, id).await,
        VellumUri::Book(id) => ws.get_book(actor, id).await,
        VellumUri::Issue(key) => {
            ws.get_issue(actor, key).await.map(|i| Document {
                id: i.key.clone(),
                title: i.title,
                markdown: i.markdown,
                url: i.url,
                space: String::new(),
                version: String::new(),
                updated_at: String::new(),
            })
        }
        VellumUri::Collection { id, view } => {
            ws.get_collection(actor, id, view.as_deref()).await
        }
        VellumUri::SpaceTree(slug) => ws.space_tree(actor, slug).await,
    };
    let doc = doc.map_err(|e| match e {
        WorkspaceError::NotFound(_) | WorkspaceError::Forbidden(_) => {
            not_found(uri)
        }
        WorkspaceError::Unavailable(m) => RpcError::new(
            crate::protocol::code::INTERNAL_ERROR,
            format!("temporarily unavailable: {m}"),
        ),
        other => RpcError::invalid_params(other.to_string()),
    })?;
    let guarded = safety::guard(&doc.markdown);
    let mut content = json!({
        "uri": uri,
        "mimeType": "text/markdown",
        "text": guarded.text
    });
    let mut meta = Map::new();
    if !doc.url.is_empty() {
        let _ = meta.insert("dev.vellum/url".into(), json!(doc.url));
    }
    if !doc.version.is_empty() {
        let _ = meta.insert("dev.vellum/version".into(), json!(doc.version));
    }
    if !guarded.warnings.is_empty() {
        let _ =
            meta.insert("dev.vellum/warnings".into(), json!(guarded.warnings));
    }
    if !meta.is_empty()
        && let Some(o) = content.as_object_mut()
    {
        let _ = o.insert("_meta".into(), Value::Object(meta));
    }
    let mut body = Map::new();
    let _ = body.insert("contents".into(), json!([content]));
    let _ = body.insert("ttlMs".into(), json!(READ_TTL_MS));
    let _ = body.insert("cacheScope".into(), json!("private"));
    Ok(body)
}

fn not_found(uri: &str) -> RpcError {
    RpcError::invalid_params("Resource not found")
        .with_data(json!({ "uri": uri }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_template() {
        assert_eq!(
            VellumUri::parse("vellum://doc/01J9Z8"),
            Some(VellumUri::Doc("01J9Z8".into()))
        );
        assert_eq!(
            VellumUri::parse("vellum://book/01JB"),
            Some(VellumUri::Book("01JB".into()))
        );
        assert_eq!(
            VellumUri::parse("vellum://issue/ENG-142"),
            Some(VellumUri::Issue("ENG-142".into()))
        );
        assert_eq!(
            VellumUri::parse("vellum://collection/01JC?view=Open%20bugs"),
            Some(VellumUri::Collection {
                id: "01JC".into(),
                view: Some("Open bugs".into())
            })
        );
        assert_eq!(
            VellumUri::parse("vellum://space/eng/tree"),
            Some(VellumUri::SpaceTree("eng".into()))
        );
    }

    #[test]
    fn rejects_everything_else() {
        for bad in [
            "https://x/doc/1",
            "vellum://doc/../etc",
            "vellum://doc/",
            "vellum://issue/eng-1",
            "vellum://issue/ENG-x",
            "vellum://space/eng",
            "vellum://collection/1?sort=x",
            "vellum://doc/1?x=1",
        ] {
            assert_eq!(VellumUri::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn templates_are_listed_in_a_fixed_order() {
        let t = templates();
        assert_eq!(t.as_array().map(Vec::len), Some(5));
        assert_eq!(t[0]["uriTemplate"], "vellum://doc/{id}");
    }
}
