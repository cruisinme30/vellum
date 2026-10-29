// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Checks the generated block model is self-consistent.

use vellum_contracts::blocks::{CalloutKind, DataFenceLang, MarkType, NodeType, SCHEMA_VERSION};

#[test]
fn every_name_round_trips() {
    for node in NodeType::ALL {
        assert_eq!(NodeType::from_name(node.name()), Some(node));
    }
    for mark in MarkType::ALL {
        assert_eq!(MarkType::from_name(mark.name()), Some(mark));
        assert!(!mark.markdown().is_empty());
    }
    assert_eq!(NodeType::from_name("nope"), None);
    const { assert!(SCHEMA_VERSION >= 1) };
}

#[test]
fn block_ids_live_on_blocks_not_inlines() {
    for node in NodeType::ALL {
        assert!(!(node.has_block_id() && node.is_inline()), "{node:?}");
    }
    assert!(NodeType::Paragraph.has_block_id());
    assert!(NodeType::Mention.is_inline());
    assert_eq!(NodeType::Callout.markdown(), "container:callout");
    assert_eq!(NodeType::Columns.content(), "column{2,4}");
}

#[test]
fn enum_attributes_have_defaults() {
    assert_eq!(CalloutKind::DEFAULT, CalloutKind::Info);
    assert_eq!(CalloutKind::from_name("crit"), Some(CalloutKind::Crit));
    assert_eq!(
        DataFenceLang::from_name("risk-matrix"),
        Some(DataFenceLang::RiskMatrix)
    );
    assert_eq!(DataFenceLang::ALL.len(), 16);
}
