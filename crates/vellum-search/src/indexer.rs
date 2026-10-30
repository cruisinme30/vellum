// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Incremental indexing on every save, and rebuilds from the files (R-SR-4).
//!
//! The [`Indexer`] consumes domain events. Events carry IDs, not content,
//! so it re-reads each touched document through a [`DocumentSource`] and
//! writes the batch. Bursts coalesce: a page saved ten times in a second is
//! indexed once. The queue is bounded, so a stalled index pushes back on
//! the publisher instead of growing without limit, and [`Indexer::flush`]
//! gives read-your-writes to callers that need it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tracing::Instrument as _;
use vellum_kernel::{DocId, Event, EventKind};

use crate::error::{Result, SearchError};
use crate::model::IndexDocument;
use crate::service::{SearchService, Update};

/// Where the indexer reads documents from (the pages service, from files).
#[async_trait]
pub trait DocumentSource: Send + Sync {
    /// The document as the index should see it, or `None` once it is gone.
    async fn load(&self, doc: DocId) -> Result<Option<IndexDocument>>;

    /// Every document, for a rebuild.
    async fn list(&self) -> Result<Vec<DocId>>;
}

/// Indexer settings.
#[derive(Debug, Clone, Copy)]
pub struct IndexerConfig {
    /// Jobs the queue holds before senders wait.
    pub queue: usize,
    /// Documents written per batch.
    pub batch: usize,
    /// How long to gather a burst before writing.
    pub debounce: Duration,
    /// Attempts per document before it is dropped and logged.
    pub attempts: u32,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            queue: 4_096,
            batch: 64,
            debounce: Duration::from_millis(25),
            attempts: 3,
        }
    }
}

enum Job {
    Doc(DocId),
    Flush(oneshot::Sender<Result<()>>),
}

/// The handle that feeds the indexing task.
#[derive(Debug, Clone)]
pub struct Indexer {
    jobs: mpsc::Sender<Job>,
}

/// The documents an event touches.
#[must_use]
pub fn touched(event: &Event) -> Vec<DocId> {
    match &event.kind {
        EventKind::DocCreated { doc, .. }
        | EventKind::DocUpdated { doc, .. }
        | EventKind::DocRenamed { doc, .. }
        | EventKind::DocMoved { doc, .. }
        | EventKind::DocTrashed { doc, .. }
        | EventKind::DocRestored { doc, .. }
        | EventKind::DocPurged { doc, .. } => vec![*doc],
        EventKind::ConflictCopyCreated { doc, copy, .. } => vec![*doc, *copy],
        _ => Vec::new(),
    }
}

fn closed() -> SearchError {
    SearchError::Source("the indexer has stopped".into())
}

impl Indexer {
    /// Starts the indexing task on the current runtime.
    #[must_use]
    pub fn spawn(
        service: SearchService,
        source: Arc<dyn DocumentSource>,
        config: IndexerConfig,
    ) -> (Self, JoinHandle<()>) {
        let (jobs, rx) = mpsc::channel(config.queue.max(1));
        let task = tokio::spawn(run(service, source, config, rx));
        (Self { jobs }, task)
    }

    /// Queues the documents an event touches. Waits when the queue is full.
    ///
    /// # Errors
    /// When the indexing task has stopped.
    pub async fn on_event(&self, event: &Event) -> Result<()> {
        for doc in touched(event) {
            self.jobs.send(Job::Doc(doc)).await.map_err(|_| closed())?;
        }
        Ok(())
    }

    /// Queues one document, for feeds that know IDs rather than events.
    /// Waits when the queue is full.
    ///
    /// # Errors
    /// When the indexing task has stopped.
    pub async fn touch(&self, doc: DocId) -> Result<()> {
        self.jobs.send(Job::Doc(doc)).await.map_err(|_| closed())
    }

    /// Waits until everything queued before this call is indexed.
    ///
    /// # Errors
    /// The last write's error, or when the indexing task has stopped.
    pub async fn flush(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.jobs.send(Job::Flush(tx)).await.map_err(|_| closed())?;
        rx.await.map_err(|_| closed())?
    }
}

async fn run(
    service: SearchService,
    source: Arc<dyn DocumentSource>,
    config: IndexerConfig,
    mut rx: mpsc::Receiver<Job>,
) {
    let mut pending: BTreeSet<DocId> = BTreeSet::new();
    let mut failures: std::collections::HashMap<DocId, u32> = std::collections::HashMap::new();
    let mut waiters: Vec<oneshot::Sender<Result<()>>> = Vec::new();
    loop {
        // Block for the first job, then gather a burst until quiet.
        let first = if pending.is_empty() {
            match rx.recv().await {
                Some(job) => Some(job),
                None => return,
            }
        } else {
            None
        };
        let mut open = true;
        let mut next = first;
        loop {
            match next.take() {
                Some(Job::Doc(doc)) => {
                    let _ = pending.insert(doc);
                }
                Some(Job::Flush(tx)) => {
                    waiters.push(tx);
                    break;
                }
                None => {}
            }
            if pending.len() >= config.batch {
                break;
            }
            match tokio::time::timeout(config.debounce, rx.recv()).await {
                Ok(Some(job)) => next = Some(job),
                Ok(None) => {
                    open = false;
                    break;
                }
                Err(_) => break,
            }
        }

        let mut outcome = Ok(());
        while !pending.is_empty() {
            let batch: Vec<DocId> = pending.iter().take(config.batch).copied().collect();
            for doc in &batch {
                let _ = pending.remove(doc);
            }
            if let Err(error) = index_batch(&service, source.as_ref(), &batch).await {
                tracing::warn!(%error, documents = batch.len(), "indexing batch failed");
                tracing::debug!(counter.vellum.search.index_failures = 1_u64);
                for doc in &batch {
                    let tries = failures.entry(*doc).or_insert(0);
                    *tries += 1;
                    if *tries < config.attempts {
                        let _ = pending.insert(*doc);
                    } else {
                        let _ = failures.remove(doc);
                        tracing::error!(%doc, "gave up indexing a document; a rebuild will pick it up");
                    }
                }
                outcome = Err(error);
                if !waiters.is_empty() {
                    break;
                }
                tokio::time::sleep(config.debounce).await;
            } else {
                for doc in &batch {
                    let _ = failures.remove(doc);
                }
            }
        }
        for waiter in std::mem::take(&mut waiters) {
            let reply = match &outcome {
                Ok(()) => Ok(()),
                Err(error) => Err(SearchError::Source(error.to_string())),
            };
            let _ = waiter.send(reply);
        }
        if !open && pending.is_empty() {
            return;
        }
    }
}

async fn index_batch(
    service: &SearchService,
    source: &dyn DocumentSource,
    docs: &[DocId],
) -> Result<()> {
    let mut updates = Vec::with_capacity(docs.len());
    for doc in docs {
        let loaded = source.load(*doc).await?;
        updates.push(loaded.map_or(Update::Remove(*doc), Update::Upsert));
    }
    tracing::debug!(
        counter.vellum.search.indexed_documents = u64::try_from(docs.len()).unwrap_or(u64::MAX)
    );
    service.write(updates).await
}

/// What a rebuild did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rebuilt {
    /// Documents indexed.
    pub documents: usize,
}

/// Empties the index and indexes every document again (`vellum reindex`).
///
/// # Errors
/// Source or backend failures; the index may then be partial, and running
/// the rebuild again completes it.
pub async fn rebuild(
    service: &SearchService,
    source: &dyn DocumentSource,
    batch: usize,
) -> Result<Rebuilt> {
    let span = tracing::info_span!("search.rebuild", search.engine = service.engine());
    async {
        service.clear().await?;
        let docs = source.list().await?;
        for chunk in docs.chunks(batch.max(1)) {
            index_batch(service, source, chunk).await?;
        }
        tracing::info!(documents = docs.len(), "search index rebuilt");
        Ok(Rebuilt {
            documents: docs.len(),
        })
    }
    .instrument(span)
    .await
}

/// Indexes `docs` again without emptying the index first, `batch` at a
/// time, logging progress as it goes. Used to add embeddings to documents
/// indexed before hybrid search was turned on.
///
/// # Errors
/// Source or backend failures; the documents done so far stay done.
pub async fn reindex(
    service: &SearchService,
    source: &dyn DocumentSource,
    docs: &[DocId],
    batch: usize,
) -> Result<usize> {
    let span = tracing::info_span!("search.reindex", search.engine = service.engine());
    async {
        let total = docs.len();
        let mut done = 0;
        for chunk in docs.chunks(batch.max(1)) {
            index_batch(service, source, chunk).await?;
            done += chunk.len();
            tracing::info!(done, total, "search: indexing again");
        }
        Ok(done)
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use vellum_kernel::{Actor, VersionId};

    use super::*;
    use crate::access::Principal;
    use crate::conformance::Corpus;
    use crate::model::SearchQuery;
    use crate::tantivy_index::{Location, TantivyBackend, TantivyConfig};

    #[derive(Default)]
    struct Files {
        docs: Mutex<HashMap<DocId, IndexDocument>>,
        loads: AtomicUsize,
    }

    #[async_trait]
    impl DocumentSource for Files {
        async fn load(&self, doc: DocId) -> Result<Option<IndexDocument>> {
            let _ = self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(self.docs.lock().unwrap().get(&doc).cloned())
        }

        async fn list(&self) -> Result<Vec<DocId>> {
            Ok(self.docs.lock().unwrap().keys().copied().collect())
        }
    }

    fn service(corpus: &Corpus) -> SearchService {
        let (backend, _) = TantivyBackend::open(&TantivyConfig {
            location: Location::Memory,
            key: None,
            model: None,
        })
        .unwrap();
        SearchService::new(
            Arc::new(backend),
            Arc::new(corpus.access()),
            Arc::new(corpus.stars()),
        )
    }

    fn event(corpus: &Corpus, kind: EventKind) -> Event {
        Event::new(corpus.eng, Actor::System, kind)
    }

    async fn hits(service: &SearchService, corpus: &Corpus, text: &str) -> usize {
        service
            .search(&Principal::User(corpus.ada), &SearchQuery::new(text, 50))
            .await
            .unwrap()
            .hits
            .len()
    }

    #[tokio::test]
    async fn saves_are_indexed_coalesced_and_purges_removed() {
        let corpus = Corpus::default();
        let files = Arc::new(Files::default());
        for doc in corpus.documents() {
            let _ = files.docs.lock().unwrap().insert(doc.doc, doc);
        }
        let service = service(&corpus);
        let (indexer, task) =
            Indexer::spawn(service.clone(), files.clone(), IndexerConfig::default());

        for _ in 0..10 {
            indexer
                .on_event(&event(
                    &corpus,
                    EventKind::doc_updated(corpus.runbook, VersionId::generate()),
                ))
                .await
                .unwrap();
        }
        indexer.flush().await.unwrap();
        assert!(
            files.loads.load(Ordering::SeqCst) <= 2,
            "burst was not coalesced"
        );
        assert!(hits(&service, &corpus, "canary").await > 0);

        let _ = files.docs.lock().unwrap().remove(&corpus.runbook);
        indexer
            .on_event(&event(&corpus, EventKind::doc_purged(corpus.runbook)))
            .await
            .unwrap();
        indexer.flush().await.unwrap();
        assert_eq!(hits(&service, &corpus, "canary").await, 0);

        drop(indexer);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn conflict_copies_index_both_documents_and_rebuild_restores_all() {
        let corpus = Corpus::default();
        let copy = event(
            &corpus,
            EventKind::conflict_copy_created(corpus.runbook, corpus.issue),
        );
        assert_eq!(touched(&copy), [corpus.runbook, corpus.issue]);

        let files = Files::default();
        for doc in corpus.documents() {
            let _ = files.docs.lock().unwrap().insert(doc.doc, doc);
        }
        let service = service(&corpus);
        let report = rebuild(&service, &files, 2).await.unwrap();
        assert_eq!(report.documents, 5);
        assert!(hits(&service, &corpus, "gateway").await >= 4);
    }

    struct Broken;

    #[async_trait]
    impl DocumentSource for Broken {
        async fn load(&self, _: DocId) -> Result<Option<IndexDocument>> {
            Err(SearchError::Source("disk on fire".into()))
        }

        async fn list(&self) -> Result<Vec<DocId>> {
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn flush_reports_failures() {
        let corpus = Corpus::default();
        let (indexer, _task) =
            Indexer::spawn(service(&corpus), Arc::new(Broken), IndexerConfig::default());
        indexer
            .on_event(&event(&corpus, EventKind::doc_created(corpus.runbook)))
            .await
            .unwrap();
        assert!(indexer.flush().await.is_err());
    }
}
