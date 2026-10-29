// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Real usage counts for Admin → Usage and the `vellum.usage.*` metrics
//! (ADR 0046, D194–D197).
//!
//! The services that own the numbers (identity for people, the drive index
//! for pages, storage and activity) answer a [`CountsSource`]. A
//! [`UsageSampler`] asks it every few minutes and keeps the answer, so the
//! OpenTelemetry gauges and the admin screen read a cached copy and a
//! metrics export never reaches the database (D195). Exact counts stay on
//! the server: only admins see them, and anonymous telemetry still sends
//! size buckets ([`crate::usage::bucket`]).

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use async_trait::async_trait;
use opentelemetry::KeyValue;
use opentelemetry::metrics::ObservableGauge;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{Result, Timestamp};

/// The workspace's size and activity at one moment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageCounts {
    /// When the counts were taken (Unix milliseconds); 0 before the first
    /// sample.
    pub sampled_at_ms: i64,
    /// Accounts, enabled or not.
    pub users: u64,
    /// Accounts that may sign in.
    pub enabled_users: u64,
    /// Enabled administrators.
    pub admins: u64,
    /// People seen in the last 24 hours.
    pub active_users_1d: u64,
    /// People seen in the last 7 days.
    pub active_users_7d: u64,
    /// People seen in the last 30 days.
    pub active_users_30d: u64,
    /// Browser sessions that have not expired.
    pub sessions: u64,
    /// Personal API tokens.
    pub api_tokens: u64,
    /// Spaces (shared and personal drives).
    pub spaces: u64,
    /// Live folders.
    pub folders: u64,
    /// Live pages.
    pub pages: u64,
    /// Live books.
    pub books: u64,
    /// Live collections.
    pub collections: u64,
    /// Items in the trash.
    pub trashed: u64,
    /// Saved page versions.
    pub versions: u64,
    /// Bytes of current files, live and trashed.
    pub file_bytes: u64,
    /// Bytes of saved versions.
    pub version_bytes: u64,
    /// Drive activity entries (edits, moves, shares…) in the last 24 hours.
    pub events_1d: u64,
    /// Drive activity entries in the last 7 days.
    pub events_7d: u64,
}

impl UsageCounts {
    /// Bytes stored, files and versions together.
    #[must_use]
    pub const fn stored_bytes(&self) -> u64 {
        self.file_bytes.saturating_add(self.version_bytes)
    }

    /// Pages, books and collections: what anonymous telemetry calls
    /// documents.
    #[must_use]
    pub const fn documents(&self) -> u64 {
        self.pages
            .saturating_add(self.books)
            .saturating_add(self.collections)
    }
}

/// Counts the owning services now. Implemented by the composition root.
#[async_trait]
pub trait CountsSource: Send + Sync {
    /// Fresh counts as of `now`.
    async fn count(&self, now: Timestamp) -> Result<UsageCounts>;
}

/// Keeps the latest counts and publishes them as gauges.
pub struct UsageSampler {
    source: Arc<dyn CountsSource>,
    latest: RwLock<UsageCounts>,
    gauges: OnceLock<Vec<ObservableGauge<u64>>>,
}

impl std::fmt::Debug for UsageSampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsageSampler")
            .field("latest", &self.latest())
            .finish_non_exhaustive()
    }
}

impl UsageSampler {
    /// A sampler over `source`, with nothing sampled yet.
    #[must_use]
    pub fn new(source: Arc<dyn CountsSource>) -> Self {
        Self {
            source,
            latest: RwLock::new(UsageCounts::default()),
            gauges: OnceLock::new(),
        }
    }

    /// The last counts taken; all zero before the first [`Self::refresh`].
    #[must_use]
    pub fn latest(&self) -> UsageCounts {
        *self.latest.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Counts again and keeps the result.
    ///
    /// # Errors
    ///
    /// The source failed; the previous counts are kept.
    pub async fn refresh(&self) -> Result<UsageCounts> {
        let mut counts = self.source.count(Timestamp::now()).await?;
        if counts.sampled_at_ms == 0 {
            counts.sampled_at_ms = Timestamp::now().unix_ms();
        }
        *self.latest.write().unwrap_or_else(PoisonError::into_inner) = counts;
        Ok(counts)
    }

    /// The cached counts, counted first when none were taken yet or they
    /// are older than `max_age_ms`.
    ///
    /// # Errors
    ///
    /// The source failed and nothing was cached.
    pub async fn current(&self, max_age_ms: i64) -> Result<UsageCounts> {
        let latest = self.latest();
        let age = Timestamp::now()
            .unix_ms()
            .saturating_sub(latest.sampled_at_ms);
        if latest.sampled_at_ms > 0 && age <= max_age_ms {
            return Ok(latest);
        }
        match self.refresh().await {
            Ok(counts) => Ok(counts),
            Err(error) if latest.sampled_at_ms > 0 => {
                tracing::warn!(%error, "usage counts are stale: counting failed");
                Ok(latest)
            }
            Err(error) => Err(error),
        }
    }

    /// Registers the `vellum.usage.*` gauges on the process's meter
    /// provider. Each export reads [`Self::latest`]. Safe to call more
    /// than once.
    pub fn register_metrics(self: &Arc<Self>) {
        let _ = self.gauges.get_or_init(|| {
            let meter = vellum_kernel::telemetry::meter("vellum-admin");
            GAUGES
                .iter()
                .map(|spec| {
                    let me = Arc::downgrade(self);
                    let read = spec.read;
                    meter
                        .u64_observable_gauge(spec.name)
                        .with_unit(spec.unit)
                        .with_description(spec.description)
                        .with_callback(move |observer| {
                            let Some(me) = me.upgrade() else { return };
                            let counts = me.latest();
                            if counts.sampled_at_ms == 0 {
                                return;
                            }
                            for (label, value) in read(&counts) {
                                let attrs: Vec<KeyValue> = label
                                    .map(|(k, v)| KeyValue::new(k, v))
                                    .into_iter()
                                    .collect();
                                observer.observe(value, &attrs);
                            }
                        })
                        .build()
                })
                .collect()
        });
    }
}

/// One attribute and value a gauge reports.
type Point = (Option<(&'static str, &'static str)>, u64);

/// A gauge and how it reads the counts.
struct GaugeSpec {
    name: &'static str,
    unit: &'static str,
    description: &'static str,
    read: fn(&UsageCounts) -> Vec<Point>,
}

/// The usage gauges (D196). Prometheus sees them as `vellum_usage_*`.
const GAUGES: &[GaugeSpec] = &[
    GaugeSpec {
        name: "vellum.usage.users",
        unit: "{user}",
        description: "Accounts, by state",
        read: |c| {
            vec![
                (Some(("state", "all")), c.users),
                (Some(("state", "enabled")), c.enabled_users),
                (Some(("state", "admin")), c.admins),
            ]
        },
    },
    GaugeSpec {
        name: "vellum.usage.active_users",
        unit: "{user}",
        description: "People seen in the window",
        read: |c| {
            vec![
                (Some(("window", "1d")), c.active_users_1d),
                (Some(("window", "7d")), c.active_users_7d),
                (Some(("window", "30d")), c.active_users_30d),
            ]
        },
    },
    GaugeSpec {
        name: "vellum.usage.sessions",
        unit: "{session}",
        description: "Browser sessions that have not expired",
        read: |c| vec![(None, c.sessions)],
    },
    GaugeSpec {
        name: "vellum.usage.api_tokens",
        unit: "{token}",
        description: "Personal API tokens",
        read: |c| vec![(None, c.api_tokens)],
    },
    GaugeSpec {
        name: "vellum.usage.spaces",
        unit: "{space}",
        description: "Shared and personal drives",
        read: |c| vec![(None, c.spaces)],
    },
    GaugeSpec {
        name: "vellum.usage.items",
        unit: "{item}",
        description: "Drive items, by kind; trashed items count once under trashed",
        read: |c| {
            vec![
                (Some(("kind", "page")), c.pages),
                (Some(("kind", "book")), c.books),
                (Some(("kind", "collection")), c.collections),
                (Some(("kind", "folder")), c.folders),
                (Some(("kind", "trashed")), c.trashed),
                (Some(("kind", "version")), c.versions),
            ]
        },
    },
    GaugeSpec {
        name: "vellum.usage.storage",
        unit: "By",
        description: "Bytes stored, by kind",
        read: |c| {
            vec![
                (Some(("kind", "files")), c.file_bytes),
                (Some(("kind", "versions")), c.version_bytes),
            ]
        },
    },
    GaugeSpec {
        name: "vellum.usage.activity",
        unit: "{event}",
        description: "Drive activity entries in the window",
        read: |c| {
            vec![
                (Some(("window", "1d")), c.events_1d),
                (Some(("window", "7d")), c.events_7d),
            ]
        },
    },
];

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use vellum_kernel::{Error, ErrorKind};

    use super::*;

    struct Counting {
        calls: AtomicU64,
        fail: bool,
    }

    #[async_trait]
    impl CountsSource for Counting {
        async fn count(&self, _now: Timestamp) -> Result<UsageCounts> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.fail {
                return Err(Error::new(ErrorKind::Unavailable, "index down"));
            }
            Ok(UsageCounts {
                users: n,
                pages: 3,
                books: 1,
                collections: 2,
                file_bytes: 10,
                version_bytes: 5,
                ..UsageCounts::default()
            })
        }
    }

    fn sampler(fail: bool) -> (Arc<Counting>, Arc<UsageSampler>) {
        let source = Arc::new(Counting {
            calls: AtomicU64::new(0),
            fail,
        });
        let sampler = Arc::new(UsageSampler::new(
            Arc::clone(&source) as Arc<dyn CountsSource>
        ));
        (source, sampler)
    }

    #[tokio::test]
    async fn caches_until_the_counts_are_too_old() -> Result<()> {
        let (source, sampler) = sampler(false);
        assert_eq!(sampler.latest(), UsageCounts::default());
        let first = sampler.current(60_000).await?;
        assert!(first.sampled_at_ms > 0);
        assert_eq!(first.users, 1);
        assert_eq!((first.stored_bytes(), first.documents()), (15, 6));
        let again = sampler.current(60_000).await?;
        assert_eq!(again, first);
        assert_eq!(source.calls.load(Ordering::SeqCst), 1);
        let fresh = sampler.current(-1).await?;
        assert_eq!(fresh.users, 2);
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_count_keeps_the_last_one_or_reports_the_error() {
        let (_, sampler) = sampler(true);
        assert!(sampler.current(60_000).await.is_err());
        assert_eq!(sampler.latest(), UsageCounts::default());
    }

    #[test]
    fn every_gauge_reads_the_counts() {
        let counts = UsageCounts {
            users: 4,
            enabled_users: 3,
            admins: 1,
            file_bytes: 99,
            ..UsageCounts::default()
        };
        let points: Vec<Point> = GAUGES.iter().flat_map(|g| (g.read)(&counts)).collect();
        assert!(points.contains(&(Some(("state", "all")), 4)));
        assert!(points.contains(&(Some(("kind", "files")), 99)));
        let names: Vec<&str> = GAUGES.iter().map(|g| g.name).collect();
        assert!(names.iter().all(|n| n.starts_with("vellum.usage.")));
        let (_, sampler) = sampler(false);
        sampler.register_metrics();
        sampler.register_metrics();
    }
}
