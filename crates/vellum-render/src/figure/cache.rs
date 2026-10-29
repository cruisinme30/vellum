// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The last-good-SVG cache behind the Kroki fallback.
//!
//! Entries are found two ways: by content key (the same source renders the
//! same picture), and by slot (where the diagram sits, such as
//! `<doc id>#<n>`), which serves the previous picture of a diagram whose
//! source changed while Kroki was down.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Storage for rendered diagram SVGs. The in-memory [`MemoryCache`] is the
/// default; a blob-store adapter can persist entries across restarts.
pub trait SvgCache: Send + Sync + std::fmt::Debug {
    /// The SVG rendered for exactly this content key.
    fn get(&self, key: &str) -> Option<String>;
    /// The most recent SVG stored for a slot, whatever its source.
    fn last(&self, slot: &str) -> Option<String>;
    /// Stores an SVG under its key and, when given, its slot.
    fn put(&self, key: &str, slot: Option<&str>, svg: &str);
}

#[derive(Debug, Default)]
struct Entries {
    by_key: HashMap<String, String>,
    by_slot: HashMap<String, String>,
    order: VecDeque<String>,
}

/// A bounded in-memory cache that evicts the oldest entries first.
#[derive(Debug)]
pub struct MemoryCache {
    capacity: usize,
    entries: Mutex<Entries>,
}

impl MemoryCache {
    /// A cache holding at most `capacity` diagrams.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: Mutex::new(Entries::default()),
        }
    }
}

impl SvgCache for MemoryCache {
    fn get(&self, key: &str) -> Option<String> {
        self.entries.lock().ok()?.by_key.get(key).cloned()
    }

    fn last(&self, slot: &str) -> Option<String> {
        let entries = self.entries.lock().ok()?;
        let key = entries.by_slot.get(slot)?;
        entries.by_key.get(key).cloned()
    }

    fn put(&self, key: &str, slot: Option<&str>, svg: &str) {
        let Ok(mut e) = self.entries.lock() else {
            return;
        };
        if e.by_key.insert(key.to_owned(), svg.to_owned()).is_none() {
            e.order.push_back(key.to_owned());
        }
        if let Some(slot) = slot {
            let _ = e.by_slot.insert(slot.to_owned(), key.to_owned());
        }
        while e.order.len() > self.capacity {
            if let Some(old) = e.order.pop_front() {
                let _ = e.by_key.remove(&old);
                e.by_slot.retain(|_, k| *k != old);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_by_key_and_slot_and_evicts_oldest() {
        let c = MemoryCache::new(2);
        c.put("k1", Some("doc#0"), "<svg>1</svg>");
        c.put("k2", Some("doc#0"), "<svg>2</svg>");
        assert_eq!(c.get("k1").as_deref(), Some("<svg>1</svg>"));
        assert_eq!(c.last("doc#0").as_deref(), Some("<svg>2</svg>"));
        c.put("k3", None, "<svg>3</svg>");
        assert_eq!(c.get("k1"), None);
        assert_eq!(c.get("k3").as_deref(), Some("<svg>3</svg>"));
        assert_eq!(c.last("doc#0").as_deref(), Some("<svg>2</svg>"));
        c.put("k4", None, "<svg>4</svg>");
        assert_eq!(c.last("doc#0"), None);
    }
}
