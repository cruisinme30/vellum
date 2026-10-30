// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Per-connection guards against slow and idle clients (red team round 4).
//!
//! hyper bounds how long a client may take over its request *headers*,
//! but not its body, and an HTTP/2 client that answers pings may hold a
//! connection without ever sending a request. [`Deadline`] bounds the body
//! and [`Activity`] lets the accept loop close a connection that has had
//! nothing in flight for too long.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use hyper::body::{Body, Frame, SizeHint};
use tokio::time::Sleep;

type BoxError = Box<dyn std::error::Error + Send + Sync>;
/// The gap timer and the whole-body timer.
type Timers = (Pin<Box<Sleep>>, Pin<Box<Sleep>>);

/// A request body that must keep arriving.
///
/// No gap between chunks may be longer than `idle`, and all of it must
/// arrive within `total` of the first read. A client
/// that trickles a large body a byte at a time is cut off instead of
/// holding its connection and handler for hours.
pub struct Deadline<B> {
    inner: B,
    idle: Duration,
    total: Duration,
    timers: Option<Timers>,
}

impl<B> Deadline<B> {
    /// Wraps `inner`.
    pub const fn new(inner: B, idle: Duration, total: Duration) -> Self {
        Self {
            inner,
            idle,
            total,
            timers: None,
        }
    }
}

/// Why a body was cut off.
#[derive(Debug)]
pub struct TooSlow;

impl std::fmt::Display for TooSlow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the request body arrived too slowly")
    }
}

impl std::error::Error for TooSlow {}

impl<B> Body for Deadline<B>
where
    B: Body + Unpin,
    B::Error: Into<BoxError>,
{
    type Data = B::Data;
    type Error = BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let (idle, total) = (this.idle, this.total);
        let (gap, whole) = this.timers.get_or_insert_with(|| {
            (
                Box::pin(tokio::time::sleep(idle)),
                Box::pin(tokio::time::sleep(total)),
            )
        });
        match Pin::new(&mut this.inner).poll_frame(cx) {
            Poll::Ready(Some(Ok(frame))) => {
                gap.as_mut().reset(tokio::time::Instant::now() + idle);
                Poll::Ready(Some(Ok(frame)))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e.into()))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => {
                if whole.as_mut().poll(cx).is_ready() || gap.as_mut().poll(cx).is_ready() {
                    tracing::info!(
                        event = "gateway.slow_body",
                        "cut off a request body that arrived too slowly"
                    );
                    return Poll::Ready(Some(Err(Box::new(TooSlow))));
                }
                Poll::Pending
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

/// What one connection is doing: requests in flight, and when it last
/// had one.
#[derive(Debug)]
pub struct Activity {
    in_flight: AtomicUsize,
    last: Mutex<Instant>,
}

impl Activity {
    /// A connection accepted now.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            in_flight: AtomicUsize::new(0),
            last: Mutex::new(Instant::now()),
        })
    }

    /// Marks a request in flight until the returned guard drops (when its
    /// response body has been sent).
    #[must_use]
    pub fn begin(self: &Arc<Self>) -> Busy {
        let _ = self.in_flight.fetch_add(1, Ordering::SeqCst);
        Busy(Arc::clone(self))
    }

    /// How long the connection has had nothing in flight.
    #[must_use]
    pub fn idle_for(&self) -> Duration {
        if self.in_flight.load(Ordering::SeqCst) > 0 {
            return Duration::ZERO;
        }
        self.last
            .lock()
            .map_or(Duration::ZERO, |last| last.elapsed())
    }
}

/// A request in flight; see [`Activity::begin`].
#[derive(Debug)]
pub struct Busy(Arc<Activity>);

impl Drop for Busy {
    fn drop(&mut self) {
        if let Ok(mut last) = self.0.last.lock() {
            *last = Instant::now();
        }
        let _ = self.0.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A response body that keeps its request counted as in flight until the
/// last byte is sent, so a long download is never mistaken for idling.
pub struct Tracked<B> {
    inner: B,
    _busy: Busy,
}

impl<B> Tracked<B> {
    /// Wraps `inner`, holding `busy` until it is done.
    pub const fn new(inner: B, busy: Busy) -> Self {
        Self { inner, _busy: busy }
    }
}

impl<B: Body + Unpin> Body for Tracked<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.get_mut().inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_only_counts_without_requests_in_flight() {
        let activity = Activity::new();
        let busy = activity.begin();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(activity.idle_for(), Duration::ZERO);
        drop(busy);
        std::thread::sleep(Duration::from_millis(20));
        assert!(activity.idle_for() >= Duration::from_millis(20));
    }
}
