// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The optional Meilisearch engine, for shards past a million documents (D23).
//!
//! Same records as the other engines: one Meilisearch document per block
//! (`rec = b`) and one per title (`rec = d`). Writes wait for their tasks,
//! so a save is searchable when `apply` returns. Vectors are stored with a
//! `userProvided` embedder; similarity is recomputed exactly from the
//! returned vectors so the floor means the same on every engine.

use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vellum_kernel::{BlockId, DocId};

use crate::backend::{Change, IndexEntry, Scope, SearchBackend};
use crate::error::{Result, SearchError};
use crate::model::{StoredBlock, TitleHit};

/// The embedder name registered in the index settings.
const EMBEDDER: &str = "vellum";
/// The section value of blocks before the first heading.
const NO_SECTION: &str = "none";
/// How long a write may wait for Meilisearch to index it.
const TASK_TIMEOUT: Duration = Duration::from_mins(1);
/// The most blocks one section can return.
const MAX_SECTION: usize = 5_000;

/// How to reach Meilisearch.
#[derive(Clone)]
pub struct MeiliConfig {
    /// Base URL, for example `http://127.0.0.1:7700`.
    pub url: String,
    /// The API key (an admin key for the index).
    pub api_key: Option<String>,
    /// The index UID.
    pub index: String,
    /// The embedding size when hybrid ranking is on.
    pub dimensions: Option<usize>,
}

impl std::fmt::Debug for MeiliConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MeiliConfig")
            .field("url", &self.url)
            .field("api_key", &self.api_key.as_ref().map(|_| "…"))
            .field("index", &self.index)
            .field("dimensions", &self.dimensions)
            .finish()
    }
}

/// The Meilisearch search index.
#[derive(Debug, Clone)]
pub struct MeiliBackend {
    http: reqwest::Client,
    config: MeiliConfig,
}

/// A stored Meilisearch document.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    id: String,
    rec: String,
    space: String,
    doc: String,
    #[serde(default)]
    block: u64,
    #[serde(default)]
    position: u32,
    #[serde(default)]
    section: String,
    doc_kind: String,
    #[serde(default)]
    block_kind: String,
    title: String,
    path: String,
    #[serde(default)]
    headings: Vec<String>,
    #[serde(default)]
    body: String,
    owner: Option<String>,
    modified: i64,
    ancestors: Vec<String>,
    trashed: bool,
    #[serde(
        rename = "_vectors",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    vectors: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Enqueued {
    task_uid: u64,
}

#[derive(Debug, Deserialize)]
struct Task {
    status: String,
    error: Option<TaskError>,
}

#[derive(Debug, Deserialize)]
struct TaskError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct Hits {
    hits: Vec<Record>,
}

fn meili(error: impl std::fmt::Display) -> SearchError {
    SearchError::Meilisearch(error.to_string())
}

/// Quotes a filter value. IDs and kind names are alphanumeric, but quote
/// and escape anyway so no value can change the filter's shape.
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn any_of<T: ToString>(
    field: &str,
    values: impl IntoIterator<Item = T>,
) -> String {
    let quoted: Vec<String> =
        values.into_iter().map(|v| quote(&v.to_string())).collect();
    format!("{field} IN [{}]", quoted.join(", "))
}

/// The scope as a Meilisearch filter expression.
fn filter(rec: &str, scope: Scope<'_>) -> String {
    let f = scope.filters;
    let mut parts = vec![
        format!("rec = {}", quote(rec)),
        any_of("space", scope.spaces),
        format!("trashed = {}", f.trashed),
    ];
    if let Some(docs) = scope.docs {
        parts.push(any_of("doc", docs));
    }
    if !f.owners.is_empty() {
        parts.push(any_of("owner", &f.owners));
    }
    if !f.kinds.is_empty() {
        parts.push(any_of("doc_kind", f.kinds.iter().map(|k| k.as_str())));
    }
    if let Some(range) = &f.modified {
        parts.push(format!("modified {} TO {}", range.start(), range.end()));
    }
    if let Some(location) = f.location {
        parts.push(format!("ancestors = {}", quote(&location.to_string())));
    }
    parts.join(" AND ")
}

fn records(entry: &IndexEntry, with_vectors: bool) -> Vec<Record> {
    let d = &entry.doc;
    let base = |id: String, rec: &str| Record {
        id,
        rec: rec.to_owned(),
        space: d.space.to_string(),
        doc: d.doc.to_string(),
        block: 0,
        position: 0,
        section: NO_SECTION.to_owned(),
        doc_kind: d.kind.as_str().to_owned(),
        block_kind: String::new(),
        title: d.title.clone(),
        path: d.path.clone(),
        headings: Vec::new(),
        body: String::new(),
        owner: d.owner.map(|o| o.to_string()),
        modified: d.modified.unix_ms(),
        ancestors: d.ancestors.iter().map(ToString::to_string).collect(),
        trashed: d.trashed,
        vectors: with_vectors.then(|| json!({ EMBEDDER: Value::Null })),
    };
    let mut out = vec![base(format!("{}-d", d.doc), "d")];
    for (i, block) in d.blocks.iter().enumerate() {
        let mut r = base(format!("{}-{}", d.doc, block.id.as_u64()), "b");
        r.block = block.id.as_u64();
        r.position = u32::try_from(i).unwrap_or(u32::MAX);
        r.section = block
            .section
            .map_or_else(|| NO_SECTION.to_owned(), |s| s.as_u64().to_string());
        block.kind.as_str().clone_into(&mut r.block_kind);
        r.headings.clone_from(&block.heading_path);
        r.body.clone_from(&block.text);
        if with_vectors {
            let v = entry.vectors.as_ref().and_then(|vs| vs.get(i));
            r.vectors = Some(json!({ EMBEDDER: v }));
        }
        out.push(r);
    }
    out
}

fn parse<T: std::str::FromStr>(text: &str, what: &str) -> Result<T> {
    text.parse().map_err(|_| {
        SearchError::Corrupt(format!("bad `{what}` from Meilisearch"))
    })
}

fn stored(r: &Record) -> Result<StoredBlock> {
    let block = |raw: u64| {
        BlockId::from_u64(raw)
            .ok_or_else(|| SearchError::Corrupt("block id out of range".into()))
    };
    Ok(StoredBlock {
        space: parse(&r.space, "space")?,
        doc: parse(&r.doc, "doc")?,
        block: block(r.block)?,
        position: r.position,
        section: if r.section == NO_SECTION {
            None
        } else {
            Some(block(parse(&r.section, "section")?)?)
        },
        doc_kind: parse(&r.doc_kind, "doc_kind")?,
        block_kind: parse(&r.block_kind, "block_kind")?,
        title: r.title.clone(),
        path: r.path.clone(),
        heading_path: r.headings.clone(),
        text: r.body.clone(),
    })
}

/// The stored vector of a record, if any.
fn stored_vector(r: &Record) -> Option<Vec<f32>> {
    let embeddings = r.vectors.as_ref()?.get(EMBEDDER)?;
    // Retrieved vectors come back as {"embeddings": [[…]], "regenerate":
    // false}.
    let first = embeddings
        .get("embeddings")
        .and_then(|e| e.get(0))
        .unwrap_or(embeddings);
    first
        .as_array()?
        .iter()
        .map(|x| {
            x.as_f64().map(|f| {
                #[allow(clippy::cast_possible_truncation)]
                // stored as f32 originally
                let v = f as f32;
                v
            })
        })
        .collect()
}

impl MeiliBackend {
    /// Connects, creating the index and applying settings if needed.
    ///
    /// # Errors
    /// Network or Meilisearch errors.
    pub async fn connect(config: MeiliConfig) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(meili)?;
        let backend = Self { http, config };
        let created = backend
            .send(
                reqwest::Method::POST,
                "/indexes",
                Some(
                    &json!({ "uid": backend.config.index, "primaryKey": "id" }),
                ),
            )
            .await?;
        // An existing index fails the task with `index_already_exists`; that is
        // fine.
        if let Ok(task) = serde_json::from_value::<Enqueued>(created) {
            let _ = backend.wait(task.task_uid).await;
        }
        let mut settings = json!({
            "searchableAttributes": ["title", "headings", "body"],
            "filterableAttributes": [
                "rec",
                "space",
                "doc",
                "block",
                "section",
                "doc_kind",
                "owner",
                "modified",
                "ancestors",
                "trashed"
            ],
            "sortableAttributes": ["position"],
            "displayedAttributes": ["*"],
        });
        if let (Some(dimensions), Some(object)) =
            (backend.config.dimensions, settings.as_object_mut())
        {
            let _ = object.insert(
                "embedders".into(),
                json!({
                    EMBEDDER: {
                        "source": "userProvided",
                        "dimensions": dimensions
                    }
                }),
            );
        }
        let path = format!("/indexes/{}/settings", backend.config.index);
        let task = backend
            .send(reqwest::Method::PATCH, &path, Some(&settings))
            .await?;
        backend.wait_enqueued(task).await?;
        Ok(backend)
    }

    async fn send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        let mut request = self.http.request(
            method,
            format!("{}{path}", self.config.url.trim_end_matches('/')),
        );
        if let Some(key) = &self.config.api_key {
            request = request.bearer_auth(key);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(meili)?;
        let status = response.status();
        let value: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(value)
        } else {
            let message = value
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request failed");
            Err(meili(format!("{status}: {message}")))
        }
    }

    async fn wait_enqueued(&self, response: Value) -> Result<()> {
        let task: Enqueued = serde_json::from_value(response).map_err(meili)?;
        self.wait(task.task_uid).await
    }

    /// Waits until a task finishes, failing if it failed.
    async fn wait(&self, uid: u64) -> Result<()> {
        let start = tokio::time::Instant::now();
        let mut pause = Duration::from_millis(5);
        loop {
            let task: Task = serde_json::from_value(
                self.send(reqwest::Method::GET, &format!("/tasks/{uid}"), None)
                    .await?,
            )
            .map_err(meili)?;
            match task.status.as_str() {
                "succeeded" => return Ok(()),
                "failed" | "canceled" => {
                    let message = task
                        .error
                        .map_or_else(|| task.status.clone(), |e| e.message);
                    return Err(meili(message));
                }
                _ if start.elapsed() > TASK_TIMEOUT => {
                    return Err(meili("indexing timed out"));
                }
                _ => {
                    tokio::time::sleep(pause).await;
                    pause = (pause * 2).min(Duration::from_millis(250));
                }
            }
        }
    }

    async fn search(&self, body: &Value) -> Result<Vec<Record>> {
        let path = format!("/indexes/{}/search", self.config.index);
        let hits: Hits = serde_json::from_value(
            self.send(reqwest::Method::POST, &path, Some(body)).await?,
        )
        .map_err(meili)?;
        Ok(hits.hits)
    }

    async fn documents(
        &self,
        filter: &str,
        limit: usize,
    ) -> Result<Vec<Record>> {
        let path = format!("/indexes/{}/documents/fetch", self.config.index);
        let body = json!({ "filter": filter, "limit": limit });
        let hits: Value =
            self.send(reqwest::Method::POST, &path, Some(&body)).await?;
        let results = hits
            .get("results")
            .cloned()
            .unwrap_or(Value::Array(Vec::new()));
        serde_json::from_value(results).map_err(meili)
    }
}

#[async_trait]
impl SearchBackend for MeiliBackend {
    fn name(&self) -> &'static str {
        "meilisearch"
    }

    async fn apply(&self, changes: &[Change]) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let index = &self.config.index;
        let with_vectors = self.config.dimensions.is_some();
        let mut touched = Vec::new();
        let mut adds = Vec::new();
        let mut last: std::collections::HashMap<DocId, &Change> =
            std::collections::HashMap::new();
        for change in changes {
            let doc = match change {
                Change::Remove(doc) => *doc,
                Change::Upsert(entry) => entry.doc.doc,
            };
            let _ = last.insert(doc, change);
        }
        for (doc, change) in last {
            touched.push(doc);
            if let Change::Upsert(entry) = change {
                adds.extend(records(entry, with_vectors));
            }
        }
        let delete = json!({ "filter": any_of("doc", &touched) });
        let first = self
            .send(
                reqwest::Method::POST,
                &format!("/indexes/{index}/documents/delete"),
                Some(&delete),
            )
            .await?;
        // Tasks on one index run in order, so waiting on each in turn is
        // enough; both must succeed.
        self.wait_enqueued(first).await?;
        if !adds.is_empty() {
            let body = serde_json::to_value(&adds).map_err(meili)?;
            let second = self
                .send(
                    reqwest::Method::POST,
                    &format!("/indexes/{index}/documents"),
                    Some(&body),
                )
                .await?;
            self.wait_enqueued(second).await?;
        }
        Ok(())
    }

    async fn clear(&self) -> Result<()> {
        let path = format!("/indexes/{}/documents", self.config.index);
        let task = self.send(reqwest::Method::DELETE, &path, None).await?;
        self.wait_enqueued(task).await
    }

    async fn lexical(
        &self,
        text: &str,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        let body = json!({
            "q": text,
            "filter": filter("b", scope),
            "limit": limit,
            "attributesToRetrieve": ["*"],
        });
        self.search(&body).await?.iter().map(stored).collect()
    }

    async fn semantic(
        &self,
        vector: &[f32],
        min_similarity: f32,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<StoredBlock>> {
        if self.config.dimensions.is_none() {
            return Ok(Vec::new());
        }
        let body = json!({
            "q": "",
            "vector": vector,
            "hybrid": { "embedder": EMBEDDER, "semanticRatio": 1.0 },
            "filter": filter("b", scope),
            "limit": limit,
            "retrieveVectors": true,
        });
        let mut scored = Vec::new();
        for record in self.search(&body).await? {
            let Some(stored_v) = stored_vector(&record) else {
                continue;
            };
            let similarity: f32 =
                stored_v.iter().zip(vector).map(|(a, b)| a * b).sum();
            if similarity >= min_similarity {
                scored.push((similarity, stored(&record)?));
            }
        }
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        Ok(scored.into_iter().map(|(_, block)| block).collect())
    }

    async fn titles(
        &self,
        text: &str,
        scope: Scope<'_>,
        limit: usize,
    ) -> Result<Vec<TitleHit>> {
        let body = json!({
            "q": text,
            "filter": filter("d", scope),
            "limit": limit,
            "attributesToSearchOn": ["title"],
            "matchingStrategy": "all",
        });
        self.search(&body)
            .await?
            .iter()
            .map(|r| {
                Ok(TitleHit {
                    space: parse(&r.space, "space")?,
                    doc: parse(&r.doc, "doc")?,
                    kind: parse(&r.doc_kind, "doc_kind")?,
                    title: r.title.clone(),
                    path: r.path.clone(),
                })
            })
            .collect()
    }

    async fn block(
        &self,
        doc: DocId,
        block: BlockId,
    ) -> Result<Option<StoredBlock>> {
        let filter = format!(
            "rec = \"b\" AND doc = {} AND block = {}",
            quote(&doc.to_string()),
            block.as_u64()
        );
        self.documents(&filter, 1)
            .await?
            .first()
            .map(stored)
            .transpose()
    }

    async fn section(
        &self,
        doc: DocId,
        section: Option<BlockId>,
    ) -> Result<Vec<StoredBlock>> {
        let key = section
            .map_or_else(|| NO_SECTION.to_owned(), |s| s.as_u64().to_string());
        let filter = format!(
            "rec = \"b\" AND doc = {} AND section = {}",
            quote(&doc.to_string()),
            quote(&key)
        );
        let mut blocks = self
            .documents(&filter, MAX_SECTION)
            .await?
            .iter()
            .map(stored)
            .collect::<Result<Vec<_>>>()?;
        blocks.sort_by_key(|b| b.position);
        Ok(blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::Corpus;
    use crate::model::{DocKind, Filters};

    #[test]
    fn filters_quote_every_value() {
        let corpus = Corpus::default();
        let spaces = [corpus.eng];
        let filters = Filters {
            kinds: vec![DocKind::Issue],
            modified: Some(1..=2),
            location: Some(corpus.runbooks),
            ..Filters::default()
        };
        let scope = Scope {
            spaces: &spaces,
            docs: None,
            filters: &filters,
        };
        let f = filter("b", scope);
        assert!(f.starts_with("rec = \"b\" AND space IN [\""));
        assert!(f.contains(
            "AND trashed = false AND doc_kind IN [\"issue\"] \
            AND modified 1 TO 2"
        ));
        assert!(f.ends_with(&format!("ancestors = \"{}\"", corpus.runbooks)));
        assert_eq!(quote("a\"b\\"), "\"a\\\"b\\\\\"");
    }

    #[test]
    fn records_round_trip_and_carry_vectors() {
        let corpus = Corpus::default();
        let doc = corpus.documents().remove(0);
        let n = doc.blocks.len();
        let entry = IndexEntry {
            doc,
            vectors: Some(vec![vec![0.5; 4]; n]),
        };
        let recs = records(&entry, true);
        assert_eq!(recs.len(), n + 1);
        assert_eq!(recs[0].rec, "d");
        let block = stored(&recs[3]).unwrap();
        assert_eq!(block.block, entry.doc.blocks[2].id);
        assert_eq!(block.section, entry.doc.blocks[2].section);
        assert_eq!(stored_vector(&recs[3]), Some(vec![0.5; 4]));
        let retrieved = Record {
            vectors: Some(json!({
                EMBEDDER: {
                    "embeddings": [[0.25, 0.75]],
                    "regenerate": false
                }
            })),
            ..recs[3].clone()
        };
        assert_eq!(stored_vector(&retrieved), Some(vec![0.25, 0.75]));
        let lead = stored(&recs[1]).unwrap();
        assert_eq!(lead.section, None);
    }

    #[test]
    fn the_api_key_is_not_printed() {
        let config = MeiliConfig {
            url: "http://x".into(),
            api_key: Some("secret".into()),
            index: "i".into(),
            dimensions: None,
        };
        assert!(!format!("{config:?}").contains("secret"));
    }
}
