// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reciprocal rank fusion (D62).
//!
//! Each ranked list contributes `weight / (k + rank)` to an item's score,
//! with ranks counted from 1. Scores from BM25 and cosine similarity live on
//! different scales, so fusing ranks rather than scores needs no tuning per
//! corpus; `k = 60` is the value from the original RRF paper.

use std::collections::HashMap;
use std::hash::Hash;

/// The RRF smoothing constant.
pub const RRF_K: f64 = 60.0;

/// One ranked input list with its weight.
#[derive(Debug, Clone)]
pub struct Ranked<T> {
    /// Items, best first. Duplicates after the first are ignored.
    pub items: Vec<T>,
    /// The list's weight; 1.0 for equal say.
    pub weight: f64,
}

/// Fuses ranked lists into one, best first, with each item's fused score.
///
/// Ties break by the item's best rank in any list, then by the first list
/// it appeared in, so the order is deterministic.
#[must_use]
pub fn fuse<T: Clone + Eq + Hash>(lists: &[Ranked<T>]) -> Vec<(T, f64)> {
    // item -> (score, best rank, first list)
    let mut scores: HashMap<T, (f64, usize, usize)> = HashMap::new();
    for (list_index, list) in lists.iter().enumerate() {
        let mut seen = std::collections::HashSet::new();
        for (rank0, item) in list.items.iter().enumerate() {
            if !seen.insert(item) {
                continue;
            }
            let rank = rank0 + 1;
            #[allow(clippy::cast_precision_loss)] // ranks are far below 2^52
            let contribution = list.weight / (RRF_K + rank as f64);
            let entry = scores
                .entry(item.clone())
                .or_insert((0.0, usize::MAX, list_index));
            entry.0 += contribution;
            entry.1 = entry.1.min(rank);
        }
    }
    let mut fused: Vec<(T, (f64, usize, usize))> = scores.into_iter().collect();
    fused.sort_by(|(_, a), (_, b)| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    // Equal (score, rank, list) tuples can only come from different items
    // at the same rank of the same list, which `seen` rules out, except
    // for items unique to different lists at equal ranks: order those by
    // list, which the comparator already does.
    fused
        .into_iter()
        .map(|(item, (score, _, _))| (item, score))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranked(items: &[&'static str], weight: f64) -> Ranked<&'static str> {
        Ranked {
            items: items.to_vec(),
            weight,
        }
    }

    #[test]
    fn an_item_in_both_lists_beats_one_in_a_single_list() {
        let fused = fuse(&[ranked(&["a", "b", "c"], 1.0), ranked(&["c", "d"], 1.0)]);
        let order: Vec<_> = fused.iter().map(|(item, _)| *item).collect();
        assert_eq!(order, ["c", "a", "b", "d"]);
    }

    #[test]
    fn scores_follow_the_formula() {
        let fused = fuse(&[ranked(&["a"], 1.0), ranked(&["a"], 0.5)]);
        let expected = 1.0 / 61.0 + 0.5 / 61.0;
        assert!((fused[0].1 - expected).abs() < 1e-12);
    }

    #[test]
    fn duplicates_in_one_list_count_once() {
        let fused = fuse(&[ranked(&["a", "a", "b"], 1.0)]);
        assert_eq!(fused.len(), 2);
        assert!((fused[0].1 - 1.0 / 61.0).abs() < 1e-12);
        assert!((fused[1].1 - 1.0 / 63.0).abs() < 1e-12);
    }

    #[test]
    fn ties_are_deterministic() {
        let fused = fuse(&[ranked(&["a"], 1.0), ranked(&["b"], 1.0)]);
        let order: Vec<_> = fused.iter().map(|(item, _)| *item).collect();
        assert_eq!(order, ["a", "b"]);
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(fuse::<u8>(&[]).is_empty());
    }
}
