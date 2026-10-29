// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The backend conformance suite: one behaviour, every engine.
//!
//! Each backend's tests call [`run`] with a fresh, empty backend. The suite
//! indexes a small Northstar corpus and checks ranking basics, every drive
//! filter, permission filtering, title search, sections, updates, removal
//! and paging through the [`SearchService`], so the UI and MCP see the same
//! search whichever engine is configured.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use vellum_kernel::{BlockId, DocId, ObjectId, SpaceId, Timestamp, UserId};

use crate::access::{Principal, ReadAccess, Stars};
use crate::backend::{Scope, SearchBackend};
use crate::error::{Result, SearchError};
use crate::model::{BlockKind, DocKind, Filters, IndexBlock, IndexDocument, SearchQuery};
use crate::service::{SearchService, Update};
use crate::text;
use crate::vector::{self, Embedder};

/// A failed conformance check.
#[derive(Debug, thiserror::Error)]
pub enum Failure {
    /// An expectation did not hold.
    #[error("{0}")]
    Check(String),
    /// The backend or service returned an error.
    #[error(transparent)]
    Search(#[from] SearchError),
}

macro_rules! check {
    ($cond:expr, $($msg:tt)+) => {
        if !$cond {
            return Err(Failure::Check(format!($($msg)+)));
        }
    };
}

/// Fixed IDs of the corpus.
#[derive(Debug, Clone, Copy)]
pub struct Corpus {
    /// The engineering space.
    pub eng: SpaceId,
    /// The finance space.
    pub finance: SpaceId,
    /// The runbooks folder in engineering.
    pub runbooks: ObjectId,
    /// Ada: reads everything.
    pub ada: UserId,
    /// Grace: reads engineering, except the incident page.
    pub grace: UserId,
    /// The gateway runbook page.
    pub runbook: DocId,
    /// The latency issue.
    pub issue: DocId,
    /// The budget page in finance.
    pub budget: DocId,
    /// A trashed page.
    pub trashed: DocId,
    /// The incident page Grace cannot read.
    pub incident: DocId,
}

impl Default for Corpus {
    fn default() -> Self {
        Self {
            eng: SpaceId::from_u128(0x10),
            finance: SpaceId::from_u128(0x11),
            runbooks: ObjectId::from_u128(0x20),
            ada: UserId::from_u128(0x30),
            grace: UserId::from_u128(0x31),
            runbook: DocId::from_u128(0x40),
            issue: DocId::from_u128(0x41),
            budget: DocId::from_u128(0x42),
            trashed: DocId::from_u128(0x43),
            incident: DocId::from_u128(0x44),
        }
    }
}

/// A block ID from a small number (always in range).
#[must_use]
pub fn bid(n: u64) -> BlockId {
    BlockId::from_u64(n & BlockId::MAX).unwrap_or_else(BlockId::generate)
}

fn at(ms: i64) -> Timestamp {
    Timestamp::from_unix_ms(ms).unwrap_or_else(|_| Timestamp::now())
}

fn block(
    id: u64,
    kind: BlockKind,
    section: Option<u64>,
    headings: &[&str],
    text: &str,
) -> IndexBlock {
    IndexBlock {
        id: bid(id),
        kind,
        section: section.map(bid),
        heading_path: headings.iter().map(|h| (*h).to_owned()).collect(),
        text: text.to_owned(),
    }
}

impl Corpus {
    /// The documents of the corpus.
    #[must_use]
    #[expect(clippy::too_many_lines, reason = "fixture data, one document per call")]
    pub fn documents(&self) -> Vec<IndexDocument> {
        let doc = |space,
                   doc,
                   kind,
                   title: &str,
                   owner,
                   modified,
                   ancestors: Vec<ObjectId>,
                   trashed,
                   blocks| {
            IndexDocument {
                space,
                doc,
                kind,
                title: title.to_owned(),
                path: format!("{}.md", title.to_lowercase().replace(' ', "-")),
                owner: Some(owner),
                modified: at(modified),
                ancestors,
                trashed,
                blocks,
            }
        };
        let eng_root = ObjectId::from(self.eng);
        vec![
            doc(
                self.eng,
                self.runbook,
                DocKind::Page,
                "Gateway runbook",
                self.ada,
                1_000,
                vec![eng_root, self.runbooks],
                false,
                vec![
                    block(
                        1,
                        BlockKind::Paragraph,
                        None,
                        &[],
                        "On-call guide for the Northstar API gateway.",
                    ),
                    block(2, BlockKind::Heading, Some(2), &["Deploying"], "Deploying"),
                    block(
                        3,
                        BlockKind::Paragraph,
                        Some(2),
                        &["Deploying"],
                        "Run the canary deploy at 10% of traffic, then 50%, then everyone.",
                    ),
                    block(4, BlockKind::Heading, Some(4), &["Rollback"], "Rollback"),
                    block(
                        5,
                        BlockKind::Paragraph,
                        Some(4),
                        &["Rollback"],
                        "Roll back with the release tool when the error budget burns too fast.",
                    ),
                    block(
                        6,
                        BlockKind::ListItem,
                        Some(4),
                        &["Rollback"],
                        "Page the gateway owner after every rollback.",
                    ),
                ],
            ),
            doc(
                self.eng,
                self.issue,
                DocKind::Issue,
                "Gateway latency regression",
                self.grace,
                2_000,
                vec![eng_root],
                false,
                vec![
                    block(1, BlockKind::Property, None, &[], "Status: In progress"),
                    block(
                        2,
                        BlockKind::Paragraph,
                        None,
                        &[],
                        "p95 latency rose after the policy engine change.",
                    ),
                ],
            ),
            doc(
                self.finance,
                self.budget,
                DocKind::Page,
                "Q3 budget",
                self.ada,
                3_000,
                vec![ObjectId::from(self.finance)],
                false,
                vec![block(
                    1,
                    BlockKind::Paragraph,
                    None,
                    &[],
                    "Gateway hosting costs rose twelve percent.",
                )],
            ),
            doc(
                self.eng,
                self.trashed,
                DocKind::Page,
                "Old gateway notes",
                self.ada,
                500,
                vec![eng_root],
                true,
                vec![block(
                    1,
                    BlockKind::Paragraph,
                    None,
                    &[],
                    "Deprecated gateway deploy notes.",
                )],
            ),
            doc(
                self.eng,
                self.incident,
                DocKind::Page,
                "Gateway security incident",
                self.ada,
                4_000,
                vec![eng_root],
                false,
                vec![block(
                    1,
                    BlockKind::Paragraph,
                    None,
                    &[],
                    "Gateway deploy keys rotated after the incident.",
                )],
            ),
        ]
    }

    /// Permissions for the corpus: Ada reads all; Grace reads engineering
    /// except the incident; anyone else reads nothing.
    #[must_use]
    pub fn access(&self) -> StaticAccess {
        let mut access = StaticAccess::default();
        access.grant(
            Principal::User(self.ada),
            &[self.eng, self.finance],
            &[
                self.runbook,
                self.issue,
                self.budget,
                self.trashed,
                self.incident,
            ],
        );
        access.grant(
            Principal::User(self.grace),
            &[self.eng],
            &[self.runbook, self.issue, self.trashed],
        );
        access
    }

    /// Stars for the corpus: Ada starred the budget.
    #[must_use]
    pub fn stars(&self) -> StaticStars {
        StaticStars(HashMap::from([(self.ada, HashSet::from([self.budget]))]))
    }
}

/// In-memory permissions for tests and the conformance suite.
#[derive(Debug, Clone, Default)]
pub struct StaticAccess {
    spaces: HashMap<Principal, Vec<SpaceId>>,
    docs: HashMap<Principal, HashSet<DocId>>,
}

impl StaticAccess {
    /// Lets `principal` read `docs` in `spaces`.
    pub fn grant(&mut self, principal: Principal, spaces: &[SpaceId], docs: &[DocId]) {
        self.spaces
            .entry(principal)
            .or_default()
            .extend_from_slice(spaces);
        self.docs
            .entry(principal)
            .or_default()
            .extend(docs.iter().copied());
    }

    /// Revokes `doc` from `principal`.
    pub fn revoke(&mut self, principal: Principal, doc: DocId) {
        if let Some(docs) = self.docs.get_mut(&principal) {
            let _ = docs.remove(&doc);
        }
    }
}

#[async_trait]
impl ReadAccess for StaticAccess {
    async fn readable_spaces(&self, principal: &Principal) -> Result<Vec<SpaceId>> {
        Ok(self.spaces.get(principal).cloned().unwrap_or_default())
    }

    async fn filter_readable(
        &self,
        principal: &Principal,
        docs: &[DocId],
    ) -> Result<HashSet<DocId>> {
        let Some(readable) = self.docs.get(principal) else {
            return Ok(HashSet::new());
        };
        Ok(docs
            .iter()
            .copied()
            .filter(|doc| readable.contains(doc))
            .collect())
    }
}

/// In-memory stars.
#[derive(Debug, Clone, Default)]
pub struct StaticStars(pub HashMap<UserId, HashSet<DocId>>);

#[async_trait]
impl Stars for StaticStars {
    async fn starred(&self, user: UserId) -> Result<HashSet<DocId>> {
        Ok(self.0.get(&user).cloned().unwrap_or_default())
    }
}

/// A deterministic bag-of-words embedder: each analysed term hashes to one
/// dimension. Good enough to test hybrid plumbing without a model file.
#[derive(Debug, Clone, Copy)]
pub struct HashEmbedder {
    /// The vector length.
    pub dimensions: usize,
}

impl HashEmbedder {
    fn embed(self, input: &str) -> Vec<f32> {
        let mut v = vec![0.0; self.dimensions.max(1)];
        let len = v.len();
        for token in text::tokens(input) {
            // FNV-1a: stable across runs and platforms.
            let hash = token.term.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            });
            let slot = usize::try_from(hash % u64::try_from(len).unwrap_or(1)).unwrap_or(0);
            if let Some(x) = v.get_mut(slot) {
                *x += 1.0;
            }
        }
        vector::normalise(&mut v);
        v
    }
}

#[async_trait]
impl Embedder for HashEmbedder {
    fn model_id(&self) -> &'static str {
        "hash-bow"
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn embed_passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed(t)).collect())
    }

    async fn embed_query(&self, input: &str) -> Result<Vec<f32>> {
        Ok(self.embed(input))
    }
}

fn docs_of(page: &crate::model::SearchPage) -> HashSet<DocId> {
    page.hits.iter().map(|hit| hit.doc).collect()
}

/// Runs every check against an empty backend.
///
/// # Errors
/// The first check that fails.
pub async fn run(backend: Arc<dyn SearchBackend>) -> Result<(), Failure> {
    let corpus = Corpus::default();
    let service = SearchService::new(
        Arc::clone(&backend),
        Arc::new(corpus.access()),
        Arc::new(corpus.stars()),
    )
    .with_embedder(Arc::new(HashEmbedder { dimensions: 64 }));
    service
        .write(corpus.documents().into_iter().map(Update::Upsert).collect())
        .await?;

    ranking(&service, &corpus).await?;
    permissions(&service, &corpus).await?;
    filters(&service, &corpus).await?;
    titles(&service, &corpus).await?;
    sections(&service, &corpus).await?;
    semantic(backend.as_ref(), &corpus).await?;
    paging(&service, &corpus).await?;
    updates(&service, &corpus).await?;
    Ok(())
}

async fn ranking(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let ada = Principal::User(c.ada);
    let page = service
        .search(&ada, &SearchQuery::new("canary deploy", 10))
        .await?;
    let top = page
        .hits
        .first()
        .ok_or_else(|| Failure::Check("no hits for `canary deploy`".into()))?;
    check!(
        top.doc == c.runbook && top.block == bid(3),
        "best hit for `canary deploy` was {top:?}"
    );
    check!(
        top.heading_path == ["Deploying"],
        "heading path {:?}",
        top.heading_path
    );
    check!(
        top.snippet.iter().any(|f| f.hit && f.text == "canary"),
        "no highlight in {:?}",
        top.snippet
    );
    check!(
        top.citation == crate::model::citation(c.runbook, bid(3)),
        "citation {}",
        top.citation
    );
    check!(page.hybrid, "hybrid ranking was off with an embedder");
    check!(
        !docs_of(&page).contains(&c.trashed),
        "trash leaked into live search"
    );

    // Stemming: `rolling` finds `Roll back` and `rollback`.
    let page = service
        .search(&ada, &SearchQuery::new("rollbacks", 10))
        .await?;
    check!(
        page.hits
            .iter()
            .any(|h| h.doc == c.runbook && h.block == bid(6)),
        "stemmed search missed: {page:?}"
    );
    Ok(())
}

async fn permissions(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let grace = Principal::User(c.grace);
    let page = service
        .search(&grace, &SearchQuery::new("gateway", 50))
        .await?;
    let docs = docs_of(&page);
    check!(!docs.contains(&c.budget), "another space leaked: {docs:?}");
    check!(
        !docs.contains(&c.incident),
        "an unreadable page leaked: {docs:?}"
    );
    check!(
        docs.contains(&c.runbook) && docs.contains(&c.issue),
        "readable pages missing: {docs:?}"
    );

    let titles = service
        .titles(&grace, "gateway", &Filters::default(), 10)
        .await?;
    check!(
        titles.iter().all(|t| t.doc != c.incident),
        "title search leaked: {titles:?}"
    );

    let stranger = Principal::User(UserId::from_u128(0xdead));
    let page = service
        .search(&stranger, &SearchQuery::new("gateway", 10))
        .await?;
    check!(page.hits.is_empty(), "an unknown principal saw {page:?}");
    let context = service.context(&grace, c.incident, bid(1)).await?;
    check!(
        context.is_none(),
        "context of an unreadable page was returned"
    );
    Ok(())
}

async fn filtered(
    service: &SearchService,
    who: UserId,
    text: &str,
    filters: Filters,
) -> Result<HashSet<DocId>> {
    let mut query = SearchQuery::new(text, 50);
    query.filters = filters;
    Ok(docs_of(
        &service.search(&Principal::User(who), &query).await?,
    ))
}

async fn filters(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let only = |doc: DocId| HashSet::from([doc]);
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            kinds: vec![DocKind::Issue],
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.issue), "type filter gave {got:?}");
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            owners: vec![c.grace],
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.issue), "owner filter gave {got:?}");
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            location: Some(c.runbooks),
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.runbook), "location filter gave {got:?}");
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            modified: Some(2_500..=3_500),
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.budget), "modified filter gave {got:?}");
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            trashed: true,
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.trashed), "trash search gave {got:?}");
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            starred: true,
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.budget), "starred filter gave {got:?}");
    let got = filtered(
        service,
        c.ada,
        "gateway",
        Filters {
            spaces: vec![c.finance],
            ..Filters::default()
        },
    )
    .await?;
    check!(got == only(c.budget), "space filter gave {got:?}");
    // Asking for a space you cannot read narrows to nothing, never widens.
    let got = filtered(
        service,
        c.grace,
        "gateway",
        Filters {
            spaces: vec![c.finance],
            ..Filters::default()
        },
    )
    .await?;
    check!(got.is_empty(), "space filter widened access: {got:?}");
    Ok(())
}

async fn titles(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let ada = Principal::User(c.ada);
    let found = service
        .titles(&ada, "gateway ru", &Filters::default(), 10)
        .await?;
    let docs: Vec<DocId> = found.iter().map(|t| t.doc).collect();
    check!(docs == [c.runbook], "title prefix search gave {found:?}");
    let found = service
        .titles(&ada, "Q3 BUDG", &Filters::default(), 10)
        .await?;
    check!(
        found
            .first()
            .is_some_and(|t| t.doc == c.budget && t.title == "Q3 budget"),
        "title search gave {found:?}"
    );
    let found = service
        .titles(&ada, "notes", &Filters::default(), 10)
        .await?;
    check!(found.is_empty(), "trashed title surfaced: {found:?}");
    Ok(())
}

async fn sections(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let ada = Principal::User(c.ada);
    let context = service
        .context(&ada, c.runbook, bid(5))
        .await?
        .ok_or_else(|| Failure::Check("no context for a readable block".into()))?;
    let blocks: Vec<BlockId> = context.section.iter().map(|b| b.block).collect();
    check!(blocks == [bid(4), bid(5), bid(6)], "section was {blocks:?}");
    let lead = service
        .context(&ada, c.runbook, bid(1))
        .await?
        .ok_or_else(|| Failure::Check("no context for the lead block".into()))?;
    let blocks: Vec<BlockId> = lead.section.iter().map(|b| b.block).collect();
    check!(blocks == [bid(1)], "lead section was {blocks:?}");
    check!(
        service.context(&ada, c.runbook, bid(99)).await?.is_none(),
        "missing block had context"
    );
    Ok(())
}

async fn semantic(backend: &dyn SearchBackend, c: &Corpus) -> Result<(), Failure> {
    let embedder = HashEmbedder { dimensions: 64 };
    let spaces = [c.eng, c.finance];
    let filters = Filters::default();
    let scope = Scope {
        spaces: &spaces,
        docs: None,
        filters: &filters,
    };
    let probe =
        "Runbook › Rollback\nRoll back with the release tool when the error budget burns too fast.";
    let nearest = backend
        .semantic(&embedder.embed(probe), 0.35, scope, 3)
        .await?;
    let first = nearest
        .first()
        .ok_or_else(|| Failure::Check("no semantic hits".into()))?;
    check!(
        first.doc == c.runbook && first.block == bid(5),
        "nearest block was {first:?}"
    );
    check!(nearest.len() <= 3, "semantic ignored the limit");
    let finance_only = [c.finance];
    let scope = Scope {
        spaces: &finance_only,
        docs: None,
        filters: &filters,
    };
    let nearest = backend
        .semantic(&embedder.embed(probe), 0.0, scope, 10)
        .await?;
    check!(
        nearest.iter().all(|b| b.space == c.finance),
        "semantic ignored the scope"
    );
    Ok(())
}

async fn paging(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let ada = Principal::User(c.ada);
    let all = service
        .search(&ada, &SearchQuery::new("gateway", 100))
        .await?;
    check!(
        all.hits.len() >= 4 && !all.has_more,
        "full page: {} hits, more={}",
        all.hits.len(),
        all.has_more
    );
    let mut first = SearchQuery::new("gateway", 2);
    let page1 = service.search(&ada, &first).await?;
    first.offset = 2;
    let page2 = service.search(&ada, &first).await?;
    check!(page1.has_more, "first page did not report more");
    let keys =
        |p: &crate::model::SearchPage| p.hits.iter().map(|h| (h.doc, h.block)).collect::<Vec<_>>();
    let expected: Vec<_> = keys(&all).into_iter().take(4).collect();
    let mut got = keys(&page1);
    got.extend(keys(&page2));
    check!(got == expected, "pages {got:?} differ from {expected:?}");
    let empty = service.search(&ada, &SearchQuery::new("   ", 10)).await;
    check!(
        matches!(empty, Err(SearchError::Invalid(_))),
        "an empty query was accepted"
    );
    Ok(())
}

async fn updates(service: &SearchService, c: &Corpus) -> Result<(), Failure> {
    let ada = Principal::User(c.ada);
    let mut runbook = c
        .documents()
        .into_iter()
        .find(|d| d.doc == c.runbook)
        .ok_or_else(|| Failure::Check("corpus lost the runbook".into()))?;
    if let Some(b) = runbook.blocks.get_mut(2) {
        "Ship with a blue-green switch instead.".clone_into(&mut b.text);
    }
    service
        .write(vec![Update::Upsert(runbook), Update::Remove(c.issue)])
        .await?;
    let page = service
        .search(&ada, &SearchQuery::new("canary", 10))
        .await?;
    check!(page.hits.is_empty(), "stale text still found: {page:?}");
    let page = service
        .search(&ada, &SearchQuery::new("blue-green", 10))
        .await?;
    check!(
        page.hits.first().is_some_and(|h| h.doc == c.runbook),
        "new text not found: {page:?}"
    );
    let page = service
        .search(&ada, &SearchQuery::new("latency", 10))
        .await?;
    check!(page.hits.is_empty(), "removed doc still found: {page:?}");
    let found = service
        .titles(&ada, "latency", &Filters::default(), 10)
        .await?;
    check!(found.is_empty(), "removed title still found: {found:?}");
    service.clear().await?;
    let page = service
        .search(&ada, &SearchQuery::new("gateway", 10))
        .await?;
    check!(page.hits.is_empty(), "clear left {page:?}");
    Ok(())
}
