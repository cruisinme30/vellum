// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The Postgres backend for the HA profile (D23).
//!
//! Full-text search with weighted `tsvector`s (title A, headings B, body D)
//! ranked by `ts_rank_cd`, and `pg_trgm` for the instant title search, so
//! HA needs no extra service. The tables live in the `vellum_search` schema
//! with their own migrations; the service never reads another service's
//! tables. Vectors are `real[]` with an exact filtered scan.

use std::collections::HashSet;

use async_trait::async_trait;
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Postgres, QueryBuilder, Row};
use uuid::Uuid;
use vellum_kernel::{BlockId, DocId, SpaceId};

use crate::backend::{Change, IndexEntry, Scope, SearchBackend};
use crate::error::{Result, SearchError};
use crate::model::{StoredBlock, TitleHit};

/// The schema that holds every search table.
pub const SCHEMA: &str = "vellum_search";

/// Rows per multi-row insert (well under the 65 535 bind limit).
const INSERT_CHUNK: usize = 500;

/// The Postgres search index.
#[derive(Debug, Clone)]
pub struct PostgresBackend {
    pool: PgPool,
}

impl PostgresBackend {
    /// Uses `pool` and brings the search schema up to date.
    ///
    /// # Errors
    /// Migration failures (for example, `pg_trgm` not installable).
    pub async fn connect(pool: PgPool) -> Result<Self> {
        let mut migrator = sqlx::migrate!("migrations/postgres");
        let _ = migrator.create_schema(SCHEMA);
        let _ = migrator.dangerous_set_table_name(format!("{SCHEMA}._sqlx_migrations"));
        migrator.run(&pool).await?;
        Ok(Self { pool })
    }

    /// Documents with a block that has no embedding: the ones to embed
    /// when hybrid search is turned on over an existing index.
    ///
    /// # Errors
    /// Database failures.
    pub async fn docs_without_vectors(&self) -> Result<Vec<DocId>> {
        let rows =
            sqlx::query("SELECT DISTINCT doc_id FROM vellum_search.blocks WHERE embedding IS NULL")
                .fetch_all(&self.pool)
                .await?;
        rows.iter()
            .map(|row| Ok(DocId::from(row.try_get::<Uuid, _>("doc_id")?)))
            .collect()
    }
}

/// Splits a query into positive terms, quoted phrases and negated terms,
/// and renders a `to_tsquery` expression: any positive term or phrase may
/// match (ranking rewards more), negated terms must not.
fn tsquery(text: &str) -> Option<String> {
    let mut positive = Vec::new();
    let mut negative = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let (before, after) = rest.split_at(start);
        words(before, &mut positive, &mut negative);
        let after = after.get(1..).unwrap_or_default();
        let (phrase, tail) = after.split_once('"').unwrap_or((after, ""));
        let terms = crate::tantivy_index::plain_words(phrase);
        if terms.len() > 1 {
            positive.push(format!("({})", terms.join(" <-> ")));
        } else {
            positive.extend(terms);
        }
        rest = tail;
    }
    words(rest, &mut positive, &mut negative);
    if positive.is_empty() {
        return None;
    }
    let mut out = format!("({})", positive.join(" | "));
    for term in negative {
        out.push_str(" & !");
        out.push_str(&term);
    }
    Some(out)
}

fn words(text: &str, positive: &mut Vec<String>, negative: &mut Vec<String>) {
    for word in text.split_whitespace() {
        if let Some(w) = word.strip_prefix('-') {
            negative.extend(crate::tantivy_index::plain_words(w));
            continue;
        }
        let word = word.trim_start_matches('+');
        positive.extend(crate::tantivy_index::plain_words(word));
    }
}

/// Escapes a word for a Postgres regular expression.
fn regex_escape(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if !c.is_alphanumeric() {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn uuids<T: Copy + Into<Uuid>>(ids: impl IntoIterator<Item = T>) -> Vec<Uuid> {
    ids.into_iter().map(Into::into).collect()
}

/// Appends the scope as `AND …` clauses.
fn push_scope(q: &mut QueryBuilder<Postgres>, scope: Scope<'_>) {
    let f = scope.filters;
    let _ = q
        .push(" AND space_id = ANY(")
        .push_bind(uuids(scope.spaces.iter().copied()))
        .push(") AND trashed = ")
        .push_bind(f.trashed);
    if let Some(docs) = scope.docs {
        let _ = q
            .push(" AND doc_id = ANY(")
            .push_bind(uuids(docs.iter().copied()))
            .push(")");
    }
    if !f.owners.is_empty() {
        let _ = q
            .push(" AND owner_id = ANY(")
            .push_bind(uuids(f.owners.iter().copied()))
            .push(")");
    }
    if !f.kinds.is_empty() {
        let kinds: Vec<String> = f.kinds.iter().map(|k| k.as_str().to_owned()).collect();
        let _ = q.push(" AND doc_kind = ANY(").push_bind(kinds).push(")");
    }
    if let Some(range) = &f.modified {
        let _ = q
            .push(" AND modified_at BETWEEN ")
            .push_bind(*range.start())
            .push(" AND ")
            .push_bind(*range.end());
    }
    if let Some(location) = f.location {
        let _ = q
            .push(" AND ancestors @> ARRAY[")
            .push_bind(Uuid::from(location))
            .push("]::uuid[]");
    }
}

macro_rules! block_columns {
    () => {
        "space_id, doc_id, block_id, position, section_id, doc_kind, block_kind, title, path, \
         headings, body"
    };
}

const BLOCK_COLUMNS: &str = block_columns!();

fn corrupt(what: &str) -> SearchError {
    SearchError::Corrupt(format!("bad `{what}` in the search table"))
}

fn read_block(row: &PgRow) -> Result<StoredBlock> {
    let block = |raw: i64| u64::try_from(raw).ok().and_then(BlockId::from_u64);
    Ok(StoredBlock {
        space: SpaceId::from(row.try_get::<Uuid, _>("space_id")?),
        doc: DocId::from(row.try_get::<Uuid, _>("doc_id")?),
        block: block(row.try_get("block_id")?).ok_or_else(|| corrupt("block_id"))?,
        position: u32::try_from(row.try_get::<i32, _>("position")?)
            .map_err(|_| corrupt("position"))?,
        section: match row.try_get::<Option<i64>, _>("section_id")? {
            None => None,
            Some(raw) => Some(block(raw).ok_or_else(|| corrupt("section_id"))?),
        },
        doc_kind: row
            .try_get::<String, _>("doc_kind")?
            .parse()
            .map_err(|_| corrupt("doc_kind"))?,
        block_kind: row
            .try_get::<String, _>("block_kind")?
            .parse()
            .map_err(|_| corrupt("block_kind"))?,
        title: row.try_get("title")?,
        path: row.try_get("path")?,
        heading_path: row.try_get("headings")?,
        text: row.try_get("body")?,
    })
}

fn block_number(id: BlockId) -> i64 {
    // Block IDs are 40 bits, so this never saturates.
    i64::try_from(id.as_u64()).unwrap_or(i64::MAX)
}

async fn insert(tx: &mut sqlx::Transaction<'_, Postgres>, entries: &[&IndexEntry]) -> Result<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let mut docs = QueryBuilder::<Postgres>::new(
        "INSERT INTO vellum_search.docs (space_id, doc_id, doc_kind, title, path, owner_id, \
         modified_at, ancestors, trashed) ",
    );
    let _ = docs.push_values(entries, |mut row, entry| {
        let d = &entry.doc;
        let _ = row
            .push_bind(Uuid::from(d.space))
            .push_bind(Uuid::from(d.doc))
            .push_bind(d.kind.as_str())
            .push_bind(d.title.clone())
            .push_bind(d.path.clone())
            .push_bind(d.owner.map(Uuid::from))
            .push_bind(d.modified.unix_ms())
            .push_bind(uuids(d.ancestors.iter().copied()))
            .push_bind(d.trashed);
    });
    let _ = docs.build().execute(&mut **tx).await?;

    let rows: Vec<(&IndexEntry, usize)> = entries
        .iter()
        .flat_map(|entry| (0..entry.doc.blocks.len()).map(move |i| (*entry, i)))
        .collect();
    for chunk in rows.chunks(INSERT_CHUNK) {
        let mut q = QueryBuilder::<Postgres>::new(
            "INSERT INTO vellum_search.blocks (space_id, doc_id, block_id, position, section_id, \
             doc_kind, block_kind, title, path, headings, headings_text, body, owner_id, \
             modified_at, ancestors, trashed, embedding) ",
        );
        let _ = q.push_values(chunk, |mut row, (entry, i)| {
            let d = &entry.doc;
            let Some(block) = d.blocks.get(*i) else {
                return;
            };
            let _ = row
                .push_bind(Uuid::from(d.space))
                .push_bind(Uuid::from(d.doc))
                .push_bind(block_number(block.id))
                .push_bind(i32::try_from(*i).unwrap_or(i32::MAX))
                .push_bind(block.section.map(block_number))
                .push_bind(d.kind.as_str())
                .push_bind(block.kind.as_str())
                .push_bind(d.title.clone())
                .push_bind(d.path.clone())
                .push_bind(block.heading_path.clone())
                .push_bind(block.heading_path.join(" "))
                .push_bind(block.text.clone())
                .push_bind(d.owner.map(Uuid::from))
                .push_bind(d.modified.unix_ms())
                .push_bind(uuids(d.ancestors.iter().copied()))
                .push_bind(d.trashed)
                .push_bind(entry.vectors.as_ref().and_then(|v| v.get(*i).cloned()));
        });
        let _ = q.build().execute(&mut **tx).await?;
    }
    Ok(())
}

#[async_trait]
impl SearchBackend for PostgresBackend {
    fn name(&self) -> &'static str {
        "postgres"
    }

    async fn apply(&self, changes: &[Change]) -> Result<()> {
        let mut removed: Vec<Uuid> = Vec::with_capacity(changes.len());
        let mut upserts: Vec<&IndexEntry> = Vec::new();
        // The last change to a document wins.
        let mut seen = HashSet::new();
        for change in changes.iter().rev() {
            let doc = match change {
                Change::Remove(doc) => *doc,
                Change::Upsert(entry) => entry.doc.doc,
            };
            if !seen.insert(doc) {
                continue;
            }
            removed.push(doc.into());
            if let Change::Upsert(entry) = change {
                if let Some(v) = &entry.vectors
                    && v.len() != entry.doc.blocks.len()
                {
                    return Err(SearchError::Embedding(format!(
                        "{} vectors for {} blocks",
                        v.len(),
                        entry.doc.blocks.len()
                    )));
                }
                upserts.push(entry);
            }
        }
        let mut tx = self.pool.begin().await?;
        let _ = sqlx::query("DELETE FROM vellum_search.blocks WHERE doc_id = ANY($1)")
            .bind(&removed)
            .execute(&mut *tx)
            .await?;
        let _ = sqlx::query("DELETE FROM vellum_search.docs WHERE doc_id = ANY($1)")
            .bind(&removed)
            .execute(&mut *tx)
            .await?;
        insert(&mut tx, &upserts).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let _ = sqlx::query("TRUNCATE vellum_search.blocks, vellum_search.docs")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn lexical(
        &self,
        text: &str,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        let Some(tsquery) = tsquery(text) else {
            return Ok(Vec::new());
        };
        let mut q = QueryBuilder::<Postgres>::new("SELECT ");
        let _ = q
            .push(BLOCK_COLUMNS)
            .push(" FROM vellum_search.blocks, to_tsquery('english', ")
            .push_bind(tsquery)
            .push(") AS query WHERE tsv @@ query");
        push_scope(&mut q, scope);
        let _ = q
            .push(" ORDER BY ts_rank_cd(tsv, query) DESC, doc_id, position LIMIT ")
            .push_bind(i64::try_from(limit).unwrap_or(i64::MAX));
        let rows = q.build().fetch_all(&self.pool).await?;
        rows.iter().map(read_block).collect()
    }

    async fn semantic(
        &self,
        vector: &[f32],
        min_similarity: f32,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        // A subquery, so the similarity is computed once per row.
        let mut q = QueryBuilder::<Postgres>::new("SELECT * FROM (SELECT ");
        let _ = q
            .push(BLOCK_COLUMNS)
            .push(", vellum_search.dot(embedding, ")
            .push_bind(vector.to_vec())
            .push(") AS similarity FROM vellum_search.blocks WHERE embedding IS NOT NULL");
        push_scope(&mut q, scope);
        let _ = q
            .push(") AS scored WHERE similarity >= ")
            .push_bind(min_similarity)
            .push(" ORDER BY similarity DESC, doc_id, position LIMIT ")
            .push_bind(i64::try_from(limit).unwrap_or(i64::MAX));
        let rows = q.build().fetch_all(&self.pool).await?;
        rows.iter().map(read_block).collect()
    }

    async fn titles(&self, text: &str, scope: Scope<'_>, limit: usize) -> Result<Vec<TitleHit>> {
        let words = crate::tantivy_index::plain_words(text);
        if words.is_empty() {
            return Ok(Vec::new());
        }
        let mut q = QueryBuilder::<Postgres>::new(
            "SELECT space_id, doc_id, doc_kind, title, path FROM vellum_search.docs WHERE TRUE",
        );
        for word in &words {
            // Word-start match; the trigram index serves the regex.
            let _ = q
                .push(" AND lower(title) ~ ")
                .push_bind(format!("\\m{}", regex_escape(word)));
        }
        push_scope(&mut q, scope);
        let _ = q
            .push(" ORDER BY similarity(lower(title), ")
            .push_bind(words.join(" "))
            .push(") DESC, length(title), doc_id LIMIT ")
            .push_bind(i64::try_from(limit).unwrap_or(i64::MAX));
        let rows = q.build().fetch_all(&self.pool).await?;
        rows.iter()
            .map(|row| {
                Ok(TitleHit {
                    space: SpaceId::from(row.try_get::<Uuid, _>("space_id")?),
                    doc: DocId::from(row.try_get::<Uuid, _>("doc_id")?),
                    kind: row
                        .try_get::<String, _>("doc_kind")?
                        .parse()
                        .map_err(|_| corrupt("doc_kind"))?,
                    title: row.try_get("title")?,
                    path: row.try_get("path")?,
                })
            })
            .collect()
    }

    async fn block(&self, doc: DocId, block: BlockId) -> Result<Option<StoredBlock>> {
        let sql = concat!(
            "SELECT ",
            block_columns!(),
            " FROM vellum_search.blocks WHERE doc_id = $1 AND block_id = $2"
        );
        let row = sqlx::query(sql)
            .bind(Uuid::from(doc))
            .bind(block_number(block))
            .fetch_optional(&self.pool)
            .await?;
        row.as_ref().map(read_block).transpose()
    }

    async fn section(&self, doc: DocId, section: Option<BlockId>) -> Result<Vec<StoredBlock>> {
        let sql = concat!(
            "SELECT ",
            block_columns!(),
            " FROM vellum_search.blocks WHERE doc_id = $1 \
             AND section_id IS NOT DISTINCT FROM $2 ORDER BY position"
        );
        let rows = sqlx::query(sql)
            .bind(Uuid::from(doc))
            .bind(section.map(block_number))
            .fetch_all(&self.pool)
            .await?;
        rows.iter().map(read_block).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tsquery_ors_terms_keeps_phrases_and_negates() {
        assert_eq!(
            tsquery("canary deploy").as_deref(),
            Some("(canary | deploy)")
        );
        assert_eq!(
            tsquery("\"error budget\" burn -legacy").as_deref(),
            Some("((error <-> budget) | burn) & !legacy")
        );
        assert_eq!(
            tsquery("\"unclosed phrase").as_deref(),
            Some("((unclosed <-> phrase))")
        );
        assert_eq!(tsquery("&|!():*'").as_deref(), None);
        assert_eq!(tsquery("-only").as_deref(), None);
    }

    #[test]
    fn regex_escape_is_safe() {
        assert_eq!(regex_escape("a.b*"), "a\\.b\\*");
    }
}
