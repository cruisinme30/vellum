// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Reciprocal rank fusion (D62).
//!
//! Each ranked list contributes `weight / (k + rank)` to an item's score,
//! with ranks counted from 1. Scores from BM25 and cosine similarity live on
//! different scales, so fusing ranks rather than scores needs no tuning per
//! corpus; `k = 60` is the value from the original RRF paper.

use std::collections::{HashMap, HashSet};
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
/// Ties break by the item's best rank in any list, then by first
/// appearance (earlier lists first, then rank), so the order is
/// deterministic and the first list wins exact ties.
#[must_use]
pub fn fuse<T: Clone + Eq + Hash>(lists: &[Ranked<T>]) -> Vec<(T, f64)> {
    // Items in order of first appearance, with (score, best rank).
    let mut order: Vec<(T, f64, usize)> = Vec::new();
    let mut index: HashMap<T, usize> = HashMap::new();
    for list in lists {
        let mut seen = HashSet::new();
        for (rank0, item) in list.items.iter().enumerate() {
            if !seen.insert(item) {
                continue;
            }
            let rank = rank0 + 1;
            #[allow(clippy::cast_precision_loss)] // ranks are far below 2^52
            let contribution = list.weight / (RRF_K + rank as f64);
            let slot = *index.entry(item.clone()).or_insert_with(|| {
                order.push((item.clone(), 0.0, usize::MAX));
                order.len() - 1
            });
            if let Some(entry) = order.get_mut(slot) {
                entry.1 += contribution;
                entry.2 = entry.2.min(rank);
            }
        }
    }
    // A stable sort keeps first appearance as the final tie-break.
    order.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.2.cmp(&b.2)));
    order
        .into_iter()
        .map(|(item, score, _)| (item, score))
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
        let fused =
            fuse(&[ranked(&["a", "b", "c"], 1.0), ranked(&["c", "d"], 1.0)]);
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
    fn exact_ties_go_to_the_first_list() {
        let fused = fuse(&[ranked(&["a", "b"], 1.0), ranked(&["b", "a"], 1.0)]);
        let order: Vec<_> = fused.iter().map(|(item, _)| *item).collect();
        assert_eq!(order, ["a", "b"]);
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(fuse::<u8>(&[]).is_empty());
    }
}
