// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Admin → Settings → Search (R-AI-7, R-SR-5, D300).
//!
//! - The `search.hybrid` setting turns the semantic half of queries on or
//!   off. It is read every few seconds and whenever the status is asked
//!   for, so a change applies without a restart. Pages keep getting
//!   vectors while it is off, so turning it on again needs no backfill.
//! - `admin_search.status` (`GET /v1/admin/search`) tells the admin what
//!   the server has: the engine, the model and whether it is in memory,
//!   and how far a rebuild or an embedding backfill has got.
//!
//! The model itself stays a folder the operator places and names with
//! `VELLUM_EMBEDDINGS`: a model is hundreds of megabytes, which an upload
//! form is the wrong place for.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use schemars::JsonSchema;
use serde::Serialize;
use vellum_contracts::{Operation, RequestContext, Scope, Service};
use vellum_kernel::Error;
use vellum_search::{OnnxEmbedder, Progress, SearchService};

use crate::secrets::Admin;

/// How often the admin setting is read when nobody looks at the status.
const FOLLOW_EVERY: Duration = Duration::from_secs(5);

/// What search is doing in the background.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchJob {
    /// Nothing has run since the server started.
    #[default]
    Idle,
    /// Building the index from every page and row.
    Indexing,
    /// Adding vectors to documents indexed before the model was set.
    Embedding,
    /// The last job finished.
    Done,
    /// The last job stopped; search indexes page by page meanwhile.
    Failed,
}

/// Search as Admin → Settings → Search shows it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SearchStatusView {
    /// `tantivy` (the data folder) or `postgres`.
    pub engine: String,
    /// Whether the server has a local embedding model (`VELLUM_EMBEDDINGS`).
    pub model_configured: bool,
    /// Whether queries are ranked with it now: a model is set and the
    /// `search.hybrid` setting is on.
    pub hybrid: bool,
    /// The model's id, such as `all-MiniLM-L6-v2`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Its vector length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
    /// Whether the model is in memory now; it loads on first use and is
    /// dropped after `idle_minutes` unused.
    pub loaded: bool,
    /// How long it stays in memory unused.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idle_minutes: Option<u64>,
    /// The background job.
    pub job: SearchJob,
    /// Documents the job has done.
    pub done: usize,
    /// Documents the job has to do.
    pub total: usize,
    /// Why the last job stopped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The search service's admin side, shared by the background jobs that
/// report into it and the admin route that reads it.
pub struct SearchStatus {
    service: SearchService,
    embedder: Option<OnnxEmbedder>,
    idle: Option<Duration>,
    progress: Progress,
    job: Mutex<(SearchJob, Option<String>)>,
    admin: OnceLock<Arc<Admin>>,
}

impl std::fmt::Debug for SearchStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchStatus")
            .field("engine", &self.service.engine())
            .field("model", &self.service.model_id())
            .finish_non_exhaustive()
    }
}

impl SearchStatus {
    /// The status of `service`, whose model (if any) is `embedder`.
    #[must_use]
    pub fn new(
        service: SearchService,
        embedder: Option<OnnxEmbedder>,
        idle: Option<Duration>,
    ) -> Arc<Self> {
        Arc::new(Self {
            service,
            embedder,
            idle,
            progress: Progress::default(),
            job: Mutex::new((SearchJob::Idle, None)),
            admin: OnceLock::new(),
        })
    }

    /// Where a job counts its documents.
    #[must_use]
    pub const fn progress(&self) -> &Progress {
        &self.progress
    }

    /// Records the job now running, or how the last one ended.
    pub fn set_job(&self, job: SearchJob, error: Option<String>) {
        if let Ok(mut current) = self.job.lock() {
            *current = (job, error);
        }
    }

    /// Starts following the admin settings: `search.hybrid` turns hybrid
    /// ranking on or off within a few seconds.
    pub fn follow(self: &Arc<Self>, admin: Arc<Admin>) {
        if self.admin.set(admin).is_err() || self.embedder.is_none() {
            return;
        }
        let status = Arc::clone(self);
        drop(tokio::spawn(async move {
            let mut tick = tokio::time::interval(FOLLOW_EVERY);
            tick.set_missed_tick_behavior(
                tokio::time::MissedTickBehavior::Delay,
            );
            loop {
                let _ = tick.tick().await;
                status.apply_setting().await;
            }
        }));
    }

    /// Reads `search.hybrid` and applies it; an unreadable setting leaves
    /// things as they are.
    async fn apply_setting(&self) {
        let Some(admin) = self.admin.get() else {
            return;
        };
        match admin.settings().await {
            Ok(versioned) => {
                let on = versioned.settings.search.hybrid;
                if on != self.service.hybrid() && self.embedder.is_some() {
                    tracing::info!(
                        on,
                        "hybrid search turned {}",
                        if on { "on" } else { "off" }
                    );
                }
                self.service.set_hybrid(on);
            }
            Err(error) => {
                tracing::warn!(%error, "search: admin settings unreadable")
            }
        }
    }

    /// Search as the admin sees it now.
    pub async fn view(&self) -> SearchStatusView {
        self.apply_setting().await;
        let loaded = match &self.embedder {
            Some(e) => e.is_loaded().await,
            None => false,
        };
        let (job, error) = self
            .job
            .lock()
            .map_or((SearchJob::Idle, None), |j| j.clone());
        SearchStatusView {
            engine: self.service.engine().to_owned(),
            model_configured: self.embedder.is_some(),
            hybrid: self.service.hybrid(),
            model: self.service.model_id().map(str::to_owned),
            dimensions: self.service.dimensions(),
            loaded,
            idle_minutes: self.idle.map(|d| d.as_secs() / 60),
            job,
            done: self.progress.done(),
            total: self.progress.total(),
            error,
        }
    }
}

/// `admin_search.status`.
#[derive(Debug)]
pub struct SearchAdminService {
    status: Arc<SearchStatus>,
}

impl SearchAdminService {
    /// Serves `status`.
    #[must_use]
    pub const fn new(status: Arc<SearchStatus>) -> Self {
        Self { status }
    }
}

/// No input.
#[derive(Debug, serde::Deserialize, JsonSchema)]
pub struct NoInput {}

impl Service for SearchAdminService {
    fn name(&self) -> &'static str {
        "admin_search"
    }

    fn operations(self: Arc<Self>) -> Vec<Operation> {
        let s = Arc::clone(&self);
        vec![
            Operation::query(
                "admin_search.status",
                "/v1/admin/search",
                Scope::Admin,
                move |ctx: RequestContext, _input: NoInput| {
                    let s = Arc::clone(&s);
                    async move {
                        // The gateway checks the scope; this keeps share
                        // links out even if the route is misdeclared.
                        ctx.principal.require_scope(Scope::Admin)?;
                        Ok::<_, Error>(s.status.view().await)
                    }
                },
            )
            .summary("Search's engine, embedding model and background indexing")
            .description(
                "Whether hybrid ranking is on, which local embedding model the \
                 server has and whether it is in memory, and how far a rebuild \
                 or an embedding backfill has got.",
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(
        dir: Option<&std::path::Path>,
    ) -> (SearchService, Option<OnnxEmbedder>) {
        let status = crate::search::describe_status().unwrap();
        let mut service = status.service.clone();
        let embedder = dir.map(|dir| {
            OnnxEmbedder::new(vellum_search::OnnxConfig::from_dir(dir).unwrap())
        });
        if let Some(e) = &embedder {
            service = service.with_embedder(Arc::new(e.clone()));
        }
        (service, embedder)
    }

    #[tokio::test]
    async fn the_status_names_the_model_and_counts_the_job() {
        let (keywords, _) = service(None);
        let plain = SearchStatus::new(keywords, None, None).view().await;
        assert!(
            !plain.model_configured && !plain.hybrid && plain.model.is_none()
        );
        assert_eq!(plain.job, SearchJob::Idle);

        let dir = tempfile::tempdir().unwrap();
        let _ = vellum_search::onnx::testing::write_model(dir.path(), true);
        let (hybrid, embedder) = service(Some(dir.path()));
        let status = SearchStatus::new(
            hybrid.clone(),
            embedder,
            Some(Duration::from_mins(15)),
        );
        status.progress().start(40);
        status.set_job(SearchJob::Embedding, None);
        let view = status.view().await;
        assert!(view.model_configured && view.hybrid && !view.loaded);
        assert_eq!(view.idle_minutes, Some(15));
        assert_eq!(
            (view.job, view.done, view.total),
            (SearchJob::Embedding, 0, 40)
        );

        hybrid.set_hybrid(false);
        assert!(
            !status.view().await.hybrid,
            "no admin console: the switch stays put"
        );
    }
}
