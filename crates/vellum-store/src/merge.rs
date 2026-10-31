// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Three-way merge of a blob's two descendants against their common base.
//!
//! Vellum never resolves concurrent edits by last-write-wins (D12). Prose
//! is merged at character level from diff-match-patch diffs, so two people
//! editing different sentences of one paragraph both keep their work.
//! Manifests (`book.yaml`, `SUMMARY.md`, collection schemas, folder files)
//! are merged line by line with diff3, because a character-level merge of
//! two structural edits can yield a well-formed but wrong manifest. When
//! both sides change the same region the merge reports a conflict and the
//! caller writes a conflict copy instead of choosing a winner.

use diff_match_patch_rs::{Compat, DiffMatchPatch, Ops};

use crate::path::BlobPath;

/// How a blob's content is merged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    /// Prose: character-level merge from diff-match-patch diffs.
    Text,
    /// Structural files: line-based diff3.
    Manifest,
}

impl MergeKind {
    /// Chooses the strategy from the file name: YAML files and book
    /// `SUMMARY.md` tables of contents are manifests, everything else is
    /// text.
    #[must_use]
    pub fn for_path(path: &BlobPath) -> Self {
        let ext = |name: &str| {
            name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())
        };
        let yaml =
            matches!(ext(path.file_name()).as_deref(), Some("yaml" | "yml"));
        let in_book = path.parent().and_then(|p| ext(p.file_name())).as_deref()
            == Some("book");
        let book_toc = in_book && path.file_name() == "SUMMARY.md";
        if yaml || book_toc {
            Self::Manifest
        } else {
            Self::Text
        }
    }
}

/// Result of a three-way merge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Merged {
    /// Both sides' changes combined.
    Clean(String),
    /// Both sides changed the same region differently.
    Conflict,
}

/// Merges `ours` and `theirs`, both descended from `base`.
#[must_use]
#[tracing::instrument(
    skip_all,
    fields(kind = ?kind, base = base.len(), conflict),
    level = "debug"
)]
pub fn merge3(kind: MergeKind, base: &str, ours: &str, theirs: &str) -> Merged {
    let merged = if ours == theirs || base == theirs {
        Merged::Clean(ours.to_owned())
    } else if base == ours {
        Merged::Clean(theirs.to_owned())
    } else {
        match kind {
            MergeKind::Manifest => diffy::merge(base, ours, theirs)
                .map_or(Merged::Conflict, Merged::Clean),
            MergeKind::Text => merge_text(base, ours, theirs),
        }
    };
    let _ =
        tracing::Span::current().record("conflict", merged == Merged::Conflict);
    merged
}

/// One side's replacement of `base[start..end]` (char indices) by `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hunk {
    start: usize,
    end: usize,
    text: Vec<char>,
}

impl Hunk {
    /// True when applying both hunks would be ambiguous: their ranges
    /// overlap, or an insertion lands on or inside the other's range.
    const fn clashes(&self, other: &Self) -> bool {
        if self.start == self.end || other.start == other.end {
            self.start <= other.end && other.start <= self.end
        } else {
            self.start < other.end && other.start < self.end
        }
    }
}

fn merge_text(base: &str, ours: &str, theirs: &str) -> Merged {
    let (Some(a), Some(b)) = (hunks(base, ours), hunks(base, theirs)) else {
        return Merged::Conflict;
    };
    let mut all: Vec<Hunk> = Vec::with_capacity(a.len() + b.len());
    let (mut ia, mut ib) = (a.into_iter().peekable(), b.into_iter().peekable());
    loop {
        let next = match (ia.peek(), ib.peek()) {
            (Some(x), Some(y)) if x == y => {
                let _ = ib.next(); // Both sides made the same change.
                ia.next()
            }
            (Some(x), Some(y)) if x.clashes(y) => return Merged::Conflict,
            (Some(x), Some(y)) => {
                if x.start <= y.start {
                    ia.next()
                } else {
                    ib.next()
                }
            }
            (Some(_), None) => ia.next(),
            (None, Some(_)) => ib.next(),
            (None, None) => break,
        };
        all.extend(next);
    }
    let base: Vec<char> = base.chars().collect();
    let mut out = String::with_capacity(base.len());
    let mut at = 0;
    for hunk in all {
        out.extend(base.get(at..hunk.start).unwrap_or_default());
        out.extend(hunk.text);
        at = hunk.end;
    }
    out.extend(base.get(at..).unwrap_or_default());
    Merged::Clean(out)
}

/// The edits turning `base` into `side`, in base order. `None` if the
/// diff could not be computed (treated as a conflict, never as a win).
fn hunks(base: &str, side: &str) -> Option<Vec<Hunk>> {
    let diffs = DiffMatchPatch::new().diff_main::<Compat>(base, side).ok()?;
    let mut out: Vec<Hunk> = Vec::new();
    let mut at = 0;
    for diff in diffs {
        let len = diff.data().len();
        if diff.op() == Ops::Equal {
            at += len;
            continue;
        }
        let hunk = match out.last_mut() {
            Some(h) if h.end == at => h,
            _ => {
                out.push(Hunk {
                    start: at,
                    end: at,
                    text: Vec::new(),
                });
                out.last_mut()?
            }
        };
        if diff.op() == Ops::Delete {
            hunk.end += len;
            at += len;
        } else {
            hunk.text.extend_from_slice(diff.data());
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const BASE: &str =
        "# Plan\n\nShip the editor in May.\nHire two engineers.\n";

    #[test]
    fn disjoint_text_edits_both_survive() {
        let ours = "# Plan\n\nShip the editor in June.\nHire two engineers.\n";
        let theirs =
            "# Plan\n\nShip the editor in May.\nHire three engineers.\n";
        let want =
            "# Plan\n\nShip the editor in June.\nHire three engineers.\n";
        assert_eq!(
            merge3(MergeKind::Text, BASE, ours, theirs),
            Merged::Clean(want.into())
        );
    }

    #[test]
    fn same_word_edited_twice_conflicts() {
        let ours = BASE.replace("May", "June");
        let theirs = BASE.replace("May", "July");
        assert_eq!(
            merge3(MergeKind::Text, BASE, &ours, &theirs),
            Merged::Conflict
        );
    }

    #[test]
    fn identical_edits_merge_once() {
        let ours = BASE.replace("two", "2");
        assert_eq!(
            merge3(MergeKind::Text, BASE, &ours, &ours),
            Merged::Clean(ours.clone())
        );
        let both = format!("{}Done.\n", BASE.replace("two", "2"));
        let theirs = format!("{BASE}Done.\n");
        assert_eq!(
            merge3(MergeKind::Text, BASE, &ours, &theirs),
            Merged::Clean(both)
        );
    }

    #[test]
    fn insertions_at_the_same_point_conflict() {
        let ours = format!("{BASE}A\n");
        let theirs = format!("{BASE}B\n");
        assert_eq!(
            merge3(MergeKind::Text, BASE, &ours, &theirs),
            Merged::Conflict
        );
    }

    #[test]
    fn manifests_merge_by_line() {
        let base =
            "- [Intro](intro.md)\n- [Setup](setup.md)\n- [Usage](usage.md)\n";
        let ours = "- [Introduction](intro.md)\n- [Setup](setup.md)\n\
            - [Usage](usage.md)\n";
        let theirs = "- [Intro](intro.md)\n- [Setup](setup.md)\n\
            - [Usage](usage.md)\n- [FAQ](faq.md)\n";
        let want = "- [Introduction](intro.md)\n- [Setup](setup.md)\n\
            - [Usage](usage.md)\n- [FAQ](faq.md)\n";
        assert_eq!(
            merge3(MergeKind::Manifest, base, ours, theirs),
            Merged::Clean(want.into())
        );
        let clash = base.replace("Setup]", "Install]");
        let other = base.replace("Setup]", "Getting started]");
        assert_eq!(
            merge3(MergeKind::Manifest, base, &clash, &other),
            Merged::Conflict
        );
    }

    #[test]
    fn picks_strategy_from_path() -> Result<(), crate::StoreError> {
        let kind = |p: &str| BlobPath::new(p).map(|p| MergeKind::for_path(&p));
        assert_eq!(
            kind("drives/shared/Ops/Guide.book/SUMMARY.md")?,
            MergeKind::Manifest
        );
        assert_eq!(
            kind("drives/shared/Ops/Guide.book/book.yaml")?,
            MergeKind::Manifest
        );
        assert_eq!(kind("drives/shared/Ops/SUMMARY.md")?, MergeKind::Text);
        assert_eq!(kind("teams/ENG/issues/ENG-142.md")?, MergeKind::Text);
        Ok(())
    }

    proptest! {
        #[test]
        fn one_sided_edits_always_apply(
            base in "[a-c \\n]{0,40}",
            side in "[a-c \\n]{0,40}"
        ) {
            prop_assert_eq!(
                merge3(MergeKind::Text, &base, &side, &base),
                Merged::Clean(side.clone())
            );
            prop_assert_eq!(
                merge3(MergeKind::Text, &base, &base, &side),
                Merged::Clean(side)
            );
        }

        #[test]
        fn edits_at_both_ends_keep_both_sides(base in "[a-e \\n]{1,30}") {
            let (ours, theirs) = (format!("X{base}"), format!("{base}Y"));
            let merged = merge3(MergeKind::Text, &base, &ours, &theirs);
            prop_assert_eq!(merged, Merged::Clean(format!("X{base}Y")));
        }
    }
}
