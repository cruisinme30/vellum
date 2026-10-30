// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The append-only audit log (brief §11).
//!
//! Every record carries the SHA-256 hash of the record before it, so any
//! edit, deletion or reordering breaks the chain and [`AuditLog::verify`]
//! reports where. The file adapter seals each record with the envelope
//! encryption from `vellum-crypto` and keeps the chain head in a separate
//! sealed file, so truncating the tail is detected too.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use opentelemetry::KeyValue;
use opentelemetry::metrics::Counter;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Mutex;
use vellum_crypto::KeyProvider;
use vellum_kernel::{Error, ErrorKind, Result, Timestamp};

/// The hash before the first record: 64 zeros.
pub const GENESIS: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// The most records one query returns.
pub const MAX_LIST: usize = 1000;

/// How an audited action ended.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The action happened.
    Success,
    /// Authorization refused the action.
    Denied,
    /// The action was allowed but failed.
    Failure,
}

/// Who acted, as recorded. Mirrors `vellum_kernel::Actor` with the API
/// token or MCP client when there is one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditActor {
    /// `user`, `model`, `api_token`, `break_glass`, `external` or `system`.
    pub kind: String,
    /// The person, when there is one.
    pub user_id: Option<String>,
    /// The API token, MCP client or session that carried the call.
    pub via: Option<String>,
}

impl AuditActor {
    /// Vellum itself.
    #[must_use]
    pub fn system() -> Self {
        Self {
            kind: "system".to_owned(),
            user_id: None,
            via: None,
        }
    }

    /// A signed-in person.
    #[must_use]
    pub fn user(user_id: impl Into<String>) -> Self {
        Self {
            kind: "user".to_owned(),
            user_id: Some(user_id.into()),
            via: None,
        }
    }
}

/// An event to append.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NewAuditEvent {
    /// Who acted.
    pub actor: AuditActor,
    /// What they did, as `<service>.<action>`, such as `admin.settings_update`.
    pub action: String,
    /// What they acted on, such as a document or setting id.
    pub target: Option<String>,
    /// How it ended.
    pub outcome: Outcome,
    /// The request id shown to the user.
    pub request_id: Option<String>,
    /// The W3C trace id, to open the request in Admin → Requests.
    pub trace_id: Option<String>,
    /// Action-specific details; never document content or secrets.
    pub details: serde_json::Map<String, serde_json::Value>,
}

impl NewAuditEvent {
    /// A successful action with no target or details.
    #[must_use]
    pub fn new(actor: AuditActor, action: impl Into<String>) -> Self {
        Self {
            actor,
            action: action.into(),
            target: None,
            outcome: Outcome::Success,
            request_id: None,
            trace_id: None,
            details: serde_json::Map::new(),
        }
    }

    /// Sets the target.
    #[must_use]
    pub fn target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    /// Sets the outcome.
    #[must_use]
    pub const fn outcome(mut self, outcome: Outcome) -> Self {
        self.outcome = outcome;
        self
    }

    /// Adds one detail.
    #[must_use]
    pub fn detail(
        mut self,
        key: &str,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        let _ = self.details.insert(key.to_owned(), value.into());
        self
    }
}

/// A stored, chained record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditRecord {
    /// Position in the log, from 1.
    pub seq: u64,
    /// When it was appended.
    pub at: Timestamp,
    /// The event.
    pub event: NewAuditEvent,
    /// The previous record's hash, or [`GENESIS`].
    pub prev_hash: String,
    /// SHA-256 over `prev_hash` and this record's canonical JSON.
    pub hash: String,
}

#[derive(Serialize)]
struct Hashed<'a> {
    seq: u64,
    at: Timestamp,
    event: &'a NewAuditEvent,
    prev_hash: &'a str,
}

fn hash_of(
    seq: u64,
    at: Timestamp,
    event: &NewAuditEvent,
    prev_hash: &str,
) -> Result<String> {
    let body = serde_json::to_vec(&Hashed {
        seq,
        at,
        event,
        prev_hash,
    })
    .map_err(|e| {
        Error::new(ErrorKind::Internal, "cannot encode audit record")
            .with_source(e)
    })?;
    let mut h = Sha256::new();
    h.update(prev_hash.as_bytes());
    h.update(b"\n");
    h.update(&body);
    Ok(hex::encode(h.finalize()))
}

impl AuditRecord {
    /// Chains `event` after a record with `prev_seq` and `prev_hash`.
    ///
    /// # Errors
    ///
    /// Returns an internal error if the event cannot be encoded.
    pub fn chain(
        prev_seq: u64,
        prev_hash: &str,
        at: Timestamp,
        event: NewAuditEvent,
    ) -> Result<Self> {
        let seq = prev_seq + 1;
        let hash = hash_of(seq, at, &event, prev_hash)?;
        Ok(Self {
            seq,
            at,
            event,
            prev_hash: prev_hash.to_owned(),
            hash,
        })
    }

    /// Whether the stored hash matches the content.
    #[must_use]
    pub fn is_intact(&self) -> bool {
        hash_of(self.seq, self.at, &self.event, &self.prev_hash)
            .is_ok_and(|h| h == self.hash)
    }
}

/// Filters for [`AuditLog::list`].
#[derive(
    Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
)]
#[serde(default)]
pub struct AuditQuery {
    /// Only this action.
    pub action: Option<String>,
    /// Only this person.
    pub user_id: Option<String>,
    /// Only records before this sequence number (paging cursor).
    pub before: Option<u64>,
    /// At most this many records (default 100, capped at [`MAX_LIST`]).
    pub limit: Option<usize>,
}

impl AuditQuery {
    fn matches(&self, r: &AuditRecord) -> bool {
        self.before.is_none_or(|b| r.seq < b)
            && self.action.as_deref().is_none_or(|a| r.event.action == a)
            && self
                .user_id
                .as_deref()
                .is_none_or(|u| r.event.actor.user_id.as_deref() == Some(u))
    }

    fn limit(&self) -> usize {
        self.limit.unwrap_or(100).clamp(1, MAX_LIST)
    }
}

/// The result of checking the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChainReport {
    /// Records checked.
    pub records: u64,
    /// The head hash.
    pub head: String,
    /// The first problem found, if any.
    pub broken_at: Option<ChainBreak>,
}

/// Where and how the chain is broken.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChainBreak {
    /// The sequence number where the problem was found.
    pub seq: u64,
    /// What is wrong.
    pub reason: String,
}

/// Checks that `records` (in order) form an unbroken chain from genesis.
#[must_use]
pub fn verify_chain(records: &[AuditRecord]) -> ChainReport {
    let mut prev_hash = GENESIS.to_owned();
    let mut expected = 1_u64;
    for r in records {
        let problem = if r.seq != expected {
            Some(format!("expected record {expected}, found {}", r.seq))
        } else if r.prev_hash != prev_hash {
            Some("previous hash does not match".to_owned())
        } else if !r.is_intact() {
            Some("record content does not match its hash".to_owned())
        } else {
            None
        };
        if let Some(reason) = problem {
            return ChainReport {
                records: expected - 1,
                head: prev_hash,
                broken_at: Some(ChainBreak {
                    seq: expected,
                    reason,
                }),
            };
        }
        prev_hash.clone_from(&r.hash);
        expected += 1;
    }
    ChainReport {
        records: expected - 1,
        head: prev_hash,
        broken_at: None,
    }
}

/// The audit log port.
#[async_trait]
pub trait AuditLog: Send + Sync {
    /// Appends one event and returns the stored record.
    async fn append(&self, event: NewAuditEvent) -> Result<AuditRecord>;
    /// Matching records, newest first.
    async fn list(&self, query: &AuditQuery) -> Result<Vec<AuditRecord>>;
    /// Checks the whole chain.
    async fn verify(&self) -> Result<ChainReport>;
}

fn audit_counter() -> Counter<u64> {
    opentelemetry::global::meter("vellum-admin")
        .u64_counter("vellum.audit.events")
        .with_description("Audit records appended")
        .build()
}

fn count(counter: &Counter<u64>, record: &AuditRecord) {
    let outcome = match record.event.outcome {
        Outcome::Success => "success",
        Outcome::Denied => "denied",
        Outcome::Failure => "failure",
    };
    counter.add(
        1,
        &[
            KeyValue::new("action", record.event.action.clone()),
            KeyValue::new("outcome", outcome),
        ],
    );
}

/// An in-memory log, for tests and ephemeral servers.
#[derive(Debug)]
pub struct MemoryAuditLog {
    records: Mutex<Vec<AuditRecord>>,
    counter: Counter<u64>,
}

impl Default for MemoryAuditLog {
    fn default() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            counter: audit_counter(),
        }
    }
}

#[async_trait]
impl AuditLog for MemoryAuditLog {
    async fn append(&self, event: NewAuditEvent) -> Result<AuditRecord> {
        let mut records = self.records.lock().await;
        let (seq, prev) = records
            .last()
            .map_or((0, GENESIS), |r| (r.seq, r.hash.as_str()));
        let record = AuditRecord::chain(seq, prev, Timestamp::now(), event)?;
        records.push(record.clone());
        drop(records);
        count(&self.counter, &record);
        Ok(record)
    }

    async fn list(&self, query: &AuditQuery) -> Result<Vec<AuditRecord>> {
        Ok(self
            .records
            .lock()
            .await
            .iter()
            .rev()
            .filter(|r| query.matches(r))
            .take(query.limit())
            .cloned()
            .collect())
    }

    async fn verify(&self) -> Result<ChainReport> {
        Ok(verify_chain(&self.records.lock().await))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Head {
    seq: u64,
    hash: String,
}

fn io_error(what: &'static str) -> impl FnOnce(std::io::Error) -> Error {
    move |e| Error::new(ErrorKind::Unavailable, what).with_source(e)
}

fn crypto_error(
    what: &'static str,
) -> impl FnOnce(vellum_crypto::CryptoError) -> Error {
    move |e| Error::new(ErrorKind::Internal, what).with_source(e)
}

/// A file-backed, sealed, hash-chained audit log.
///
/// It lives in one directory (normally `.vellum/audit/`): one segment file
/// per month, one base64 sealed record per line, and a sealed `HEAD` file
/// holding the last sequence number and hash.
pub struct FileAuditLog<K: KeyProvider> {
    dir: PathBuf,
    keys: Arc<K>,
    head: Mutex<Head>,
    counter: Counter<u64>,
}

impl<K: KeyProvider> std::fmt::Debug for FileAuditLog<K> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileAuditLog")
            .field("dir", &self.dir)
            .finish_non_exhaustive()
    }
}

const HEAD_FILE: &str = "HEAD";

impl<K: KeyProvider + 'static> FileAuditLog<K> {
    /// Opens (or creates) the log in `dir` and checks it.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be read or created, or a
    /// conflict error if the chain is broken: an append-only log must not
    /// silently continue after tampering.
    pub async fn open(dir: impl Into<PathBuf>, keys: Arc<K>) -> Result<Self> {
        let dir = dir.into();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(io_error("cannot create the audit directory"))?;
        let log = Self {
            dir,
            keys,
            head: Mutex::new(Head {
                seq: 0,
                hash: GENESIS.to_owned(),
            }),
            counter: audit_counter(),
        };
        let report = log.verify().await?;
        if let Some(b) = report.broken_at {
            return Err(Error::new(
                ErrorKind::Conflict,
                format!(
                    "audit log is broken at record {}: {}",
                    b.seq, b.reason
                ),
            ));
        }
        *log.head.lock().await = Head {
            seq: report.records,
            hash: report.head,
        };
        Ok(log)
    }

    fn segment(&self, at: Timestamp) -> PathBuf {
        let text = at.to_string();
        let month = text.get(..7).unwrap_or("unknown");
        self.dir.join(format!("{month}.log"))
    }

    async fn segments(&self) -> Result<Vec<PathBuf>> {
        let mut entries = tokio::fs::read_dir(&self.dir)
            .await
            .map_err(io_error("cannot list the audit directory"))?;
        let mut files = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(io_error("cannot list the audit directory"))?
        {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "log") {
                files.push(path);
            }
        }
        files.sort();
        Ok(files)
    }

    async fn read_segment(&self, path: &Path) -> Result<Vec<AuditRecord>> {
        let text = tokio::fs::read_to_string(path)
            .await
            .map_err(io_error("cannot read an audit segment"))?;
        let mut records = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let sealed = B64.decode(line.trim()).map_err(|e| {
                Error::new(ErrorKind::Conflict, "audit segment is corrupt")
                    .with_source(e)
            })?;
            let plain = vellum_crypto::open(&*self.keys, &sealed)
                .await
                .map_err(crypto_error("cannot open an audit record"))?;
            let record: AuditRecord =
                serde_json::from_slice(&plain).map_err(|e| {
                    Error::new(ErrorKind::Conflict, "audit record is corrupt")
                        .with_source(e)
                })?;
            records.push(record);
        }
        Ok(records)
    }

    async fn all(&self) -> Result<Vec<AuditRecord>> {
        let mut all = Vec::new();
        for path in self.segments().await? {
            all.extend(self.read_segment(&path).await?);
        }
        Ok(all)
    }

    async fn read_head(&self) -> Result<Option<Head>> {
        let path = self.dir.join(HEAD_FILE);
        match tokio::fs::read(&path).await {
            Ok(sealed) => {
                let plain = vellum_crypto::open(&*self.keys, &sealed)
                    .await
                    .map_err(crypto_error("cannot open the audit head"))?;
                serde_json::from_slice(&plain).map(Some).map_err(|e| {
                    Error::new(ErrorKind::Conflict, "audit head is corrupt")
                        .with_source(e)
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error("cannot read the audit head")(e)),
        }
    }

    async fn write_head(&self, head: &Head) -> Result<()> {
        let plain = serde_json::to_vec(head).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot encode the audit head")
                .with_source(e)
        })?;
        let sealed = vellum_crypto::seal(&*self.keys, &plain)
            .await
            .map_err(crypto_error("cannot seal the audit head"))?;
        crate::fsutil::write_atomic(&self.dir.join(HEAD_FILE), &sealed).await
    }
}

#[async_trait]
impl<K: KeyProvider + 'static> AuditLog for FileAuditLog<K> {
    #[tracing::instrument(
        name = "audit.append",
        skip_all,
        fields(action = %event.action)
    )]
    async fn append(&self, event: NewAuditEvent) -> Result<AuditRecord> {
        let mut head = self.head.lock().await;
        let record =
            AuditRecord::chain(head.seq, &head.hash, Timestamp::now(), event)?;
        let plain = serde_json::to_vec(&record).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot encode audit record")
                .with_source(e)
        })?;
        let sealed = vellum_crypto::seal(&*self.keys, &plain)
            .await
            .map_err(crypto_error("cannot seal an audit record"))?;
        let mut line = B64.encode(sealed);
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.segment(record.at))
            .await
            .map_err(io_error("cannot open the audit segment"))?;
        file.write_all(line.as_bytes())
            .await
            .map_err(io_error("cannot write the audit record"))?;
        file.sync_data()
            .await
            .map_err(io_error("cannot sync the audit record"))?;
        let next = Head {
            seq: record.seq,
            hash: record.hash.clone(),
        };
        self.write_head(&next).await?;
        *head = next;
        drop(head);
        count(&self.counter, &record);
        Ok(record)
    }

    async fn list(&self, query: &AuditQuery) -> Result<Vec<AuditRecord>> {
        let limit = query.limit();
        let mut out = Vec::new();
        for path in self.segments().await?.iter().rev() {
            let records = self.read_segment(path).await?;
            out.extend(records.into_iter().rev().filter(|r| query.matches(r)));
            if out.len() >= limit {
                break;
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    #[tracing::instrument(name = "audit.verify", skip_all)]
    async fn verify(&self) -> Result<ChainReport> {
        let records = self.all().await?;
        let mut report = verify_chain(&records);
        if report.broken_at.is_none() {
            let head = self.read_head().await?;
            let consistent = match &head {
                None => report.records == 0,
                Some(h) => h.seq == report.records && h.hash == report.head,
            };
            if !consistent {
                report.broken_at = Some(ChainBreak {
                    seq: report.records + 1,
                    reason:
                        "records are missing after the last one (head mismatch)"
                            .to_owned(),
                });
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn keyring() -> Arc<vellum_crypto::LocalKeyring> {
        Arc::new(vellum_crypto::LocalKeyring::new(
            vellum_crypto::KekId::new("test").unwrap(),
            vellum_crypto::SecretKey::from_bytes([7; 32]),
        ))
    }

    fn event(action: &str) -> NewAuditEvent {
        NewAuditEvent::new(AuditActor::user("u1"), action)
            .target("settings")
            .detail("field", "telemetry.enabled")
    }

    #[tokio::test]
    async fn memory_log_chains_and_filters() {
        let log = MemoryAuditLog::default();
        let first = log.append(event("admin.a")).await.unwrap();
        assert_eq!(first.prev_hash, GENESIS);
        let second = log
            .append(
                NewAuditEvent::new(AuditActor::system(), "admin.b")
                    .outcome(Outcome::Denied),
            )
            .await
            .unwrap();
        assert_eq!(second.prev_hash, first.hash);
        assert_eq!(log.list(&AuditQuery::default()).await.unwrap().len(), 2);
        let only_a = AuditQuery {
            action: Some("admin.a".into()),
            ..AuditQuery::default()
        };
        assert_eq!(log.list(&only_a).await.unwrap().len(), 1);
        let by_user = AuditQuery {
            user_id: Some("u1".into()),
            ..AuditQuery::default()
        };
        assert_eq!(log.list(&by_user).await.unwrap().len(), 1);
        let paged = AuditQuery {
            before: Some(2),
            ..AuditQuery::default()
        };
        assert_eq!(
            log.list(&paged).await.unwrap().first().map(|r| r.seq),
            Some(1)
        );
        let report = log.verify().await.unwrap();
        assert_eq!(report.records, 2);
        assert_eq!(report.broken_at, None);
    }

    #[test]
    fn verify_finds_tampering() {
        let a = AuditRecord::chain(0, GENESIS, Timestamp::now(), event("x"))
            .unwrap();
        let b = AuditRecord::chain(1, &a.hash, Timestamp::now(), event("y"))
            .unwrap();
        let mut edited = b.clone();
        edited.event.action = "z".into();
        let report = verify_chain(&[a.clone(), edited]);
        assert_eq!(report.broken_at.map(|x| x.seq), Some(2));
        let report = verify_chain(std::slice::from_ref(&b));
        assert!(
            report
                .broken_at
                .unwrap()
                .reason
                .contains("expected record 1")
        );
        let mut relinked = b;
        relinked.prev_hash = GENESIS.into();
        let report = verify_chain(&[a, relinked]);
        assert!(report.broken_at.unwrap().reason.contains("previous hash"));
    }

    #[tokio::test]
    async fn file_log_survives_reopen_and_detects_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let keys = keyring();
        {
            let log = FileAuditLog::open(dir.path(), Arc::clone(&keys))
                .await
                .unwrap();
            assert!(format!("{log:?}").contains("FileAuditLog"));
            for i in 0..3 {
                let _ =
                    log.append(event(&format!("admin.e{i}"))).await.unwrap();
            }
            let newest = log
                .list(&AuditQuery {
                    limit: Some(2),
                    ..AuditQuery::default()
                })
                .await
                .unwrap();
            assert_eq!(
                newest.iter().map(|r| r.seq).collect::<Vec<_>>(),
                vec![3, 2]
            );
        }
        let log = FileAuditLog::open(dir.path(), Arc::clone(&keys))
            .await
            .unwrap();
        let fourth = log.append(event("admin.e3")).await.unwrap();
        assert_eq!(fourth.seq, 4);
        let raw = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(std::result::Result::ok)
            .find(|e| e.path().extension().is_some_and(|x| x == "log"))
            .unwrap()
            .path();
        let text = std::fs::read_to_string(&raw).unwrap();
        assert!(!text.contains("admin.e3"), "records are sealed");
        // Drop the last line: the head no longer matches.
        let kept: Vec<&str> = text.lines().take(3).collect();
        std::fs::write(&raw, format!("{}\n", kept.join("\n"))).unwrap();
        let err = FileAuditLog::open(dir.path(), keys).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
        assert!(err.message().contains("head mismatch"));
    }

    #[tokio::test]
    async fn file_log_rejects_garbage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("2026-10.log"), "not base64!\n")
            .unwrap();
        let err = FileAuditLog::open(dir.path(), keyring()).await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Conflict);
    }
}
