// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! RED metrics for any [`BlobStore`], applied as a decorator so every
//! adapter reports the same series without repeating the code.

use std::time::Instant;

use async_trait::async_trait;
use bytes::Bytes;
use futures::{StreamExt, stream::BoxStream};
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Histogram},
};

use crate::{
    error::{StoreError, StoreResult},
    path::BlobPath,
    store::{Blob, BlobEntry, BlobMeta, BlobStore, Precondition},
};

/// Counter of store operations, labelled `backend`, `op` and `outcome`
/// (`ok` or a [`crate::StoreError::kind`]).
pub const OPS_TOTAL: &str = "vellum_store_ops_total";
/// Histogram of operation latency in seconds, labelled `backend` and `op`.
pub const OP_SECONDS: &str = "vellum_store_op_duration_seconds";
/// Counter of bytes moved, labelled `backend` and `direction` (`read`/`write`).
pub const BYTES_TOTAL: &str = "vellum_store_bytes_total";

/// Wraps a store and records [`OPS_TOTAL`], [`OP_SECONDS`] and
/// [`BYTES_TOTAL`] through the global OpenTelemetry meter for every call.
/// Spans come from the inner adapter.
#[derive(Debug)]
pub struct Metered<S> {
    inner: S,
    ops: Counter<u64>,
    seconds: Histogram<f64>,
    bytes: Counter<u64>,
}

impl<S: BlobStore> Metered<S> {
    /// Wraps `inner`.
    pub fn new(inner: S) -> Self {
        let meter = opentelemetry::global::meter("vellum");
        Self {
            inner,
            ops: meter
                .u64_counter(OPS_TOTAL)
                .with_description("Blob store operations")
                .build(),
            seconds: meter
                .f64_histogram(OP_SECONDS)
                .with_unit("s")
                .with_description("Blob store operation latency")
                .build(),
            bytes: meter
                .u64_counter(BYTES_TOTAL)
                .with_unit("By")
                .with_description("Blob bytes moved")
                .build(),
        }
    }

    /// The wrapped store.
    pub const fn inner(&self) -> &S {
        &self.inner
    }

    fn record<T>(
        &self,
        op: &'static str,
        started: Instant,
        result: &StoreResult<T>,
    ) {
        let backend = KeyValue::new("backend", self.inner.backend());
        let op = KeyValue::new("op", op);
        let outcome = result.as_ref().map_or_else(StoreError::kind, |_| "ok");
        self.ops.add(
            1,
            &[
                backend.clone(),
                op.clone(),
                KeyValue::new("outcome", outcome),
            ],
        );
        self.seconds
            .record(started.elapsed().as_secs_f64(), &[backend, op]);
    }

    fn moved(&self, direction: &'static str, n: usize) {
        let attrs = [
            KeyValue::new("backend", self.inner.backend()),
            KeyValue::new("direction", direction),
        ];
        self.bytes.add(n as u64, &attrs);
    }
}

#[async_trait]
impl<S: BlobStore> BlobStore for Metered<S> {
    fn backend(&self) -> &'static str {
        self.inner.backend()
    }

    async fn get(&self, path: &BlobPath) -> StoreResult<Blob> {
        let started = Instant::now();
        let result = self.inner.get(path).await;
        self.record("get", started, &result);
        if let Ok(blob) = &result {
            self.moved("read", blob.bytes.len());
        }
        result
    }

    async fn head(&self, path: &BlobPath) -> StoreResult<BlobMeta> {
        let started = Instant::now();
        let result = self.inner.head(path).await;
        self.record("head", started, &result);
        result
    }

    async fn put(
        &self,
        path: &BlobPath,
        bytes: Bytes,
        condition: Precondition,
    ) -> StoreResult<BlobMeta> {
        let (started, len) = (Instant::now(), bytes.len());
        let result = self.inner.put(path, bytes, condition).await;
        self.record("put", started, &result);
        if result.is_ok() {
            self.moved("write", len);
        }
        result
    }

    async fn delete(
        &self,
        path: &BlobPath,
        condition: Precondition,
    ) -> StoreResult<()> {
        let started = Instant::now();
        let result = self.inner.delete(path, condition).await;
        self.record("delete", started, &result);
        result
    }

    async fn rename(
        &self,
        from: &BlobPath,
        to: &BlobPath,
    ) -> StoreResult<BlobMeta> {
        let started = Instant::now();
        let result = self.inner.rename(from, to).await;
        self.record("rename", started, &result);
        result
    }

    fn list(
        &self,
        dir: Option<&BlobPath>,
    ) -> BoxStream<'static, StoreResult<BlobEntry>> {
        let ops = self.ops.clone();
        let backend = KeyValue::new("backend", self.inner.backend());
        self.inner
            .list(dir)
            .inspect(move |item| {
                let outcome =
                    item.as_ref().map_or_else(StoreError::kind, |_| "ok");
                let attrs = [
                    backend.clone(),
                    KeyValue::new("op", "list_entry"),
                    KeyValue::new("outcome", outcome),
                ];
                ops.add(1, &attrs);
            })
            .boxed()
    }
}
