// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The pull cursor (D117): where a client is in the log, and under which
//! log and permission epochs that position was valid.

use std::fmt;

/// A decoded cursor. Text form: `v1.<log epoch>.<access epoch>.<seq>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Changes when the log is rebuilt; old positions are then meaningless.
    pub log_epoch: u64,
    /// The reader's permission epoch; a change means records may have
    /// become visible or invisible without a log entry.
    pub access_epoch: u64,
    /// The last log position the client has seen.
    pub seq: u64,
}

impl Cursor {
    /// Reads a cursor. Empty or unreadable text yields `None`, which means
    /// "start from scratch", never an error: a client with a stale or
    /// corrupted cursor simply resynchronises.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        if parts.next() != Some("v1") {
            return None;
        }
        let log_epoch = parts.next()?.parse().ok()?;
        let access_epoch = parts.next()?.parse().ok()?;
        let seq = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Self {
            log_epoch,
            access_epoch,
            seq,
        })
    }
}

impl fmt::Display for Cursor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "v1.{}.{}.{}",
            self.log_epoch, self.access_epoch, self.seq
        )
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn rejects_foreign_shapes() {
        for text in [
            "",
            "v2.1.2.3",
            "v1.1.2",
            "v1.1.2.3.4",
            "v1.a.2.3",
            "v1.-1.2.3",
        ] {
            assert_eq!(Cursor::parse(text), None, "{text}");
        }
    }

    proptest! {
        #[test]
        fn round_trips(log_epoch: u64, access_epoch: u64, seq: u64) {
            let cursor = Cursor { log_epoch, access_epoch, seq };
            prop_assert_eq!(Cursor::parse(&cursor.to_string()), Some(cursor));
        }

        #[test]
        fn never_panics(text in ".{0,40}") {
            let _ = Cursor::parse(&text);
        }
    }
}
