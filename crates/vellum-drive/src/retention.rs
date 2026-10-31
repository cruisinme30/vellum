// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Version retention: which versions a policy lets go.

use std::collections::HashSet;

use vellum_kernel::VersionId;

use crate::model::{RetentionPolicy, VersionRecord};

const DAY_MS: u64 = 86_400_000;

/// The versions `policy` no longer keeps at `now_ms`.
///
/// Versions younger than `keep_all_days` all stay. Older ones fall into
/// day buckets, then ISO-week buckets (weeks start on Monday), and only the
/// newest version in each bucket stays. Versions past the weekly window go,
/// except that a document's newest version always stays.
#[must_use]
pub fn prunable(
    versions: &[VersionRecord],
    policy: &RetentionPolicy,
    now_ms: u64,
) -> Vec<VersionId> {
    let mut sorted: Vec<&VersionRecord> = versions.iter().collect();
    sorted.sort_by_key(|v| std::cmp::Reverse(v.id));
    let today = now_ms / DAY_MS;
    let all_until = u64::from(policy.keep_all_days);
    let daily_until = all_until + u64::from(policy.daily_days);
    let weekly_until = policy
        .weekly_weeks
        .map(|weeks| daily_until + u64::from(weeks) * 7);
    let mut seen_days = HashSet::new();
    let mut seen_weeks = HashSet::new();
    let mut out = Vec::new();
    for (i, version) in sorted.into_iter().enumerate() {
        let day = version.written_ms() / DAY_MS;
        let age = today.saturating_sub(day);
        let keep = if age < all_until {
            true
        } else if age < daily_until {
            seen_days.insert(day)
        } else if weekly_until.is_none_or(|until| age < until) {
            // 1970-01-01 was a Thursday; shift so weeks start on Monday.
            seen_weeks.insert((day + 3) / 7)
        } else {
            false
        };
        if !keep && i > 0 {
            out.push(version.id);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use vellum_kernel::{Actor, ObjectId, SpaceId};

    use super::*;

    fn version(ms: u64, salt: u128) -> VersionRecord {
        VersionRecord {
            id: VersionId::from_u128((u128::from(ms) << 80) | salt),
            doc: ObjectId::from_u128(1),
            space: SpaceId::from_u128(2),
            size: 10,
            author: Actor::System,
        }
    }

    #[test]
    fn keeps_recent_then_thins_out() {
        let now = 1_000 * DAY_MS;
        let policy = RetentionPolicy {
            keep_all_days: 2,
            daily_days: 3,
            weekly_weeks: Some(2),
        };
        let versions = vec![
            version(now - 1, 1),
            version(now - DAY_MS / 2, 2),
            version(now - DAY_MS / 2 - 5, 3),
            version(now - 3 * DAY_MS + 100, 4),
            version(now - 3 * DAY_MS + 50, 5),
            version(now - 8 * DAY_MS + 100, 6),
            version(now - 8 * DAY_MS + 50, 7),
            version(now - 400 * DAY_MS, 8),
        ];
        let mut pruned = prunable(&versions, &policy, now);
        pruned.sort();
        let mut expected: Vec<VersionId> = [4, 6, 7]
            .iter()
            .filter_map(|i: &usize| versions.get(*i))
            .map(|v| v.id)
            .collect();
        expected.sort();
        assert_eq!(pruned, expected);
    }

    #[test]
    fn newest_version_always_stays() {
        let now = 1_000 * DAY_MS;
        let versions = vec![version(1, 1)];
        let policy = RetentionPolicy {
            keep_all_days: 0,
            daily_days: 0,
            weekly_weeks: Some(0),
        };
        assert!(prunable(&versions, &policy, now).is_empty());
    }

    #[test]
    fn forever_weekly_keeps_one_per_week() {
        let now = 10_000 * DAY_MS;
        let versions: Vec<VersionRecord> = (0..70)
            .map(|d| version(now - (200 + d) * DAY_MS, u128::from(d)))
            .collect();
        let kept = versions.len()
            - prunable(&versions, &RetentionPolicy::default(), now).len();
        assert!((10..=11).contains(&kept), "kept {kept}");
    }
}
