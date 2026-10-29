// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end encrypted private spaces on this device (ADR 0036).
//!
//! `vellum private key` prints the device's public key, which a space owner
//! enrols so new pages are sealed to this device too. `vellum mcp --private`
//! serves the spaces sealed to this device to a local AI client through
//! [`PrivateWorkspace`]: reads only, from memory, nothing written back.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use vellum_kernel::DocId;
use vellum_mcp::ports::{
    Actor, Attribution, Block, BlockSelector, ChangePage, CollectionQuery, Document, EditProposal,
    ExportArtifact, ExportRequest, Issue, IssueUpdate, Listing, ListingPage, NewComment, NewIssue,
    NewPage, NewTask, OutlineNode, RowPage, RowUpsert, SearchHit, SearchQuery, SiteInfo, Workspace,
    WorkspaceError, WriteMode, WriteReceipt,
};
use vellum_private::{PrivateAgent, PrivateDoc};

/// The keychain service the device key is stored under.
pub const KEYCHAIN_SERVICE: &str = "dev.vellum.desktop";

/// Why every write is refused.
const READ_ONLY: &str = "private spaces are read-only for AI tools: the device cannot seal a change for the other members yet";

/// Subcommands of `vellum private`.
#[derive(Debug, clap::Subcommand)]
pub enum PrivateCommand {
    /// Print this device's public key and fingerprint, creating the key in
    /// the OS keychain on first use. Give the key to a space owner to be
    /// enrolled.
    Key,
}

/// Runs `vellum private …` and returns the lines to print.
///
/// # Errors
///
/// The keychain cannot be opened or holds a damaged key.
pub fn run(command: &PrivateCommand) -> Result<Vec<String>, String> {
    match command {
        PrivateCommand::Key => {
            let identity = device_identity()?;
            Ok(vec![
                format!("public key:  {}", identity.public_hex()),
                format!("fingerprint: {}", identity.fingerprint_hex()),
            ])
        }
    }
}

/// This device's identity, from the OS keychain.
///
/// # Errors
///
/// The keychain cannot be opened or holds a damaged key.
pub fn device_identity() -> Result<vellum_private::DeviceIdentity, String> {
    let keychain = vellum_crypto::OsKeychain::open(KEYCHAIN_SERVICE)
        .map_err(|e| format!("cannot open the OS keychain: {e}"))?;
    vellum_private::DeviceIdentity::load_or_create(&keychain)
        .map_err(|e| format!("cannot load the device key: {e}"))
}

/// Parses a `--space slug=path` value.
///
/// # Errors
///
/// The value has no `=`, or either side is empty.
pub fn parse_space(value: &str) -> Result<(String, String), String> {
    match value.split_once('=') {
        Some((slug, path)) if !slug.is_empty() && !path.is_empty() => {
            Ok((slug.to_owned(), path.trim_matches('/').to_owned()))
        }
        _ => Err(format!(
            "`{value}`: expected slug=path, such as board=drives/shared/board"
        )),
    }
}

/// The private agent as an MCP workspace: search and reads over what this
/// device can open; every write and every non-page object is refused.
#[derive(Debug)]
pub struct PrivateWorkspace {
    agent: Arc<PrivateAgent>,
}

impl PrivateWorkspace {
    /// Serves `agent`.
    #[must_use]
    pub const fn new(agent: Arc<PrivateAgent>) -> Self {
        Self { agent }
    }

    fn slug_of(&self, doc: &PrivateDoc) -> String {
        self.agent
            .spaces()
            .iter()
            .find(|s| s.id == doc.space)
            .map(|s| s.slug.clone())
            .unwrap_or_default()
    }

    async fn doc(&self, id: &str) -> Result<Arc<PrivateDoc>, WorkspaceError> {
        let not_found = || WorkspaceError::NotFound(id.to_owned());
        let id: DocId = id.parse().map_err(|_| not_found())?;
        self.agent.document(id).await.ok_or_else(not_found)
    }

    fn document(&self, doc: &PrivateDoc) -> Document {
        Document {
            id: doc.id.to_string(),
            title: doc.title.clone(),
            markdown: doc.markdown.clone(),
            url: format!("vellum://doc/{}", doc.id),
            space: self.slug_of(doc),
            version: doc.modified.to_string(),
            updated_at: doc.modified.to_string(),
        }
    }
}

fn blocks_of(doc: &PrivateDoc) -> Vec<Block> {
    doc.blocks
        .iter()
        .map(|b| Block {
            id: b.id.to_string(),
            kind: serde_json::to_value(b.kind)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            markdown: b.markdown.clone(),
            heading_path: b.heading_path.clone(),
        })
        .collect()
}

/// The blocks a selector names: listed IDs, a heading's section, or a
/// range (the whole page when the selector is empty).
fn select(blocks: Vec<Block>, selector: BlockSelector) -> Result<Vec<Block>, WorkspaceError> {
    if !selector.ids.is_empty() {
        return Ok(blocks
            .into_iter()
            .filter(|b| selector.ids.contains(&b.id))
            .collect());
    }
    if let Some(section) = selector.section {
        let start = blocks
            .iter()
            .position(|b| b.id == section && b.kind == "heading")
            .ok_or_else(|| WorkspaceError::NotFound(section.clone()))?;
        let depth = blocks.get(start).map_or(0, |b| b.heading_path.len());
        return Ok(blocks
            .into_iter()
            .enumerate()
            .skip(start)
            .take_while(|(i, b)| *i == start || b.kind != "heading" || b.heading_path.len() > depth)
            .map(|(_, b)| b)
            .collect());
    }
    let find = |id: Option<String>, default: usize| -> Result<usize, WorkspaceError> {
        id.map_or(Ok(default), |id| {
            blocks
                .iter()
                .position(|b| b.id == id)
                .ok_or(WorkspaceError::NotFound(id))
        })
    };
    let from = find(selector.from, 0)?;
    let to = find(selector.to, usize::MAX)?;
    Ok(blocks
        .into_iter()
        .enumerate()
        .filter(|(i, _)| *i >= from && *i <= to)
        .map(|(_, b)| b)
        .collect())
}

fn unsupported(what: &str) -> WorkspaceError {
    WorkspaceError::NotFound(format!("{what}: private spaces hold pages only"))
}

fn read_only() -> WorkspaceError {
    WorkspaceError::Forbidden(READ_ONLY.to_owned())
}

#[async_trait]
impl Workspace for PrivateWorkspace {
    async fn search(
        &self,
        _actor: &Actor,
        query: SearchQuery,
    ) -> Result<Vec<SearchHit>, WorkspaceError> {
        if query.kind.as_deref().is_some_and(|k| k != "page") {
            return Ok(Vec::new());
        }
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);
        let hits = self
            .agent
            .search(&query.query, query.space.as_deref(), limit)
            .await
            .map_err(|e| WorkspaceError::Unavailable(e.to_string()))?;
        Ok(hits
            .into_iter()
            .map(|hit| SearchHit {
                doc_id: hit.doc.to_string(),
                block_id: Some(hit.block.to_string()),
                title: hit.title,
                heading_path: hit.heading_path,
                snippet: hit
                    .snippet
                    .iter()
                    .map(|f| {
                        if f.hit {
                            format!("**{}**", f.text)
                        } else {
                            f.text.clone()
                        }
                    })
                    .collect(),
                citation_url: hit.citation,
                score: hit.score,
            })
            .collect())
    }

    async fn get_doc(&self, _actor: &Actor, id: &str) -> Result<Document, WorkspaceError> {
        Ok(self.document(&*self.doc(id).await?))
    }

    async fn get_doc_by_path(
        &self,
        _actor: &Actor,
        path: &str,
    ) -> Result<Document, WorkspaceError> {
        self.agent
            .document_by_path(path.trim_start_matches('/'))
            .await
            .map(|d| self.document(&d))
            .ok_or_else(|| WorkspaceError::NotFound(path.to_owned()))
    }

    async fn get_blocks(
        &self,
        _actor: &Actor,
        doc_id: &str,
        selector: BlockSelector,
    ) -> Result<Vec<Block>, WorkspaceError> {
        select(blocks_of(&*self.doc(doc_id).await?), selector)
    }

    async fn get_outline(
        &self,
        _actor: &Actor,
        id: &str,
    ) -> Result<Vec<OutlineNode>, WorkspaceError> {
        Ok(blocks_of(&*self.doc(id).await?)
            .into_iter()
            .filter(|b| b.kind == "heading")
            .map(|b| OutlineNode {
                id: b.id,
                number: None,
                title: b.heading_path.last().cloned().unwrap_or_default(),
                depth: u8::try_from(b.heading_path.len()).unwrap_or(u8::MAX),
                children: Vec::new(),
            })
            .collect())
    }

    async fn get_book(&self, _actor: &Actor, id: &str) -> Result<Document, WorkspaceError> {
        Err(unsupported(id))
    }

    async fn space_tree(&self, _actor: &Actor, slug: &str) -> Result<Document, WorkspaceError> {
        let space = self
            .agent
            .space(slug)
            .ok_or_else(|| WorkspaceError::NotFound(slug.to_owned()))?;
        let prefix = format!("{}/", space.root);
        let lines: Vec<String> = self
            .agent
            .documents(slug)
            .await
            .iter()
            .map(|d| {
                let rel = d.path.strip_prefix(&prefix).unwrap_or(&d.path);
                format!("- [{}](vellum://doc/{}) `{rel}`", d.title, d.id)
            })
            .collect();
        Ok(Document {
            id: slug.to_owned(),
            title: slug.to_owned(),
            markdown: format!("# {slug}\n\n{}\n", lines.join("\n")),
            url: format!("vellum://space/{slug}"),
            space: slug.to_owned(),
            version: String::new(),
            updated_at: String::new(),
        })
    }

    async fn create_page(
        &self,
        _actor: &Actor,
        _page: NewPage,
        _mode: WriteMode,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        Err(read_only())
    }

    async fn edit_page(
        &self,
        _actor: &Actor,
        _edit: EditProposal,
        _mode: WriteMode,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        Err(read_only())
    }

    async fn get_issue(&self, _actor: &Actor, key: &str) -> Result<Issue, WorkspaceError> {
        Err(unsupported(key))
    }

    async fn create_issue(
        &self,
        _actor: &Actor,
        _issue: NewIssue,
        _by: Attribution,
    ) -> Result<Issue, WorkspaceError> {
        Err(read_only())
    }

    async fn update_issue(
        &self,
        _actor: &Actor,
        _update: IssueUpdate,
        _by: Attribution,
    ) -> Result<Issue, WorkspaceError> {
        Err(read_only())
    }

    async fn add_comment(
        &self,
        _actor: &Actor,
        _comment: NewComment,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        Err(read_only())
    }

    async fn create_task(
        &self,
        _actor: &Actor,
        _task: NewTask,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        Err(read_only())
    }

    async fn query_collection(
        &self,
        _actor: &Actor,
        query: CollectionQuery,
    ) -> Result<RowPage, WorkspaceError> {
        Err(unsupported(&query.collection))
    }

    async fn get_collection(
        &self,
        _actor: &Actor,
        id: &str,
        _view: Option<&str>,
    ) -> Result<Document, WorkspaceError> {
        Err(unsupported(id))
    }

    async fn upsert_rows(
        &self,
        _actor: &Actor,
        _upsert: RowUpsert,
        _mode: WriteMode,
        _by: Attribution,
    ) -> Result<WriteReceipt, WorkspaceError> {
        Err(read_only())
    }

    async fn export(
        &self,
        _actor: &Actor,
        _request: ExportRequest,
    ) -> Result<ExportArtifact, WorkspaceError> {
        // An export would write the plaintext to a file.
        Err(WorkspaceError::Forbidden(
            "private pages are not exported: that would write them to disk unsealed".to_owned(),
        ))
    }

    async fn list_changes(
        &self,
        _actor: &Actor,
        _since: &str,
        _limit: u32,
        _cursor: Option<String>,
    ) -> Result<ChangePage, WorkspaceError> {
        Ok(ChangePage {
            changes: Vec::new(),
            next_cursor: None,
        })
    }

    async fn list_readable(
        &self,
        _actor: &Actor,
        cursor: Option<String>,
        limit: u32,
    ) -> Result<ListingPage, WorkspaceError> {
        let start = match cursor {
            None => 0,
            Some(c) => c
                .parse::<usize>()
                .map_err(|_| WorkspaceError::Invalid("cursor".into()))?,
        };
        let mut all: BTreeMap<(String, String), Listing> = BTreeMap::new();
        for space in self.agent.spaces() {
            for doc in self.agent.documents(&space.slug).await {
                let _ = all.insert(
                    (space.slug.clone(), doc.path.clone()),
                    Listing {
                        uri: format!("vellum://doc/{}", doc.id),
                        title: doc.title.clone(),
                        description: None,
                        url: format!("vellum://doc/{}", doc.id),
                        space: space.slug.clone(),
                    },
                );
            }
        }
        let all: Vec<Listing> = all.into_values().collect();
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let end = start.saturating_add(limit).min(all.len());
        Ok(ListingPage {
            items: all
                .get(start..end)
                .map(<[Listing]>::to_vec)
                .unwrap_or_default(),
            next_cursor: (end < all.len()).then(|| end.to_string()),
        })
    }

    async fn get_template(&self, _actor: &Actor, name: &str) -> Result<String, WorkspaceError> {
        Err(unsupported(name))
    }

    async fn site_info(&self) -> Result<SiteInfo, WorkspaceError> {
        Ok(SiteInfo {
            name: "Private spaces on this device".to_owned(),
            summary: "End-to-end encrypted spaces, opened on this device only. Read-only."
                .to_owned(),
            base_url: "vellum://".to_owned(),
        })
    }
}
