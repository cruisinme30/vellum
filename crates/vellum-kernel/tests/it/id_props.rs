// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests for ID encoding.

use proptest::prelude::*;
use vellum_kernel::{BlockId, DocId, ObjectId, SpaceId};

proptest! {
    #[test]
    fn ulid_text_round_trips(raw in any::<u128>()) {
        let id = DocId::from_u128(raw);
        let text = id.to_string();
        prop_assert_eq!(text.len(), 26);
        prop_assert_eq!(text.parse::<DocId>(), Ok(id));
        prop_assert_eq!(text.to_ascii_lowercase().parse::<DocId>(), Ok(id));
    }

    #[test]
    fn text_order_matches_value_order(a in any::<u128>(), b in any::<u128>()) {
        let (x, y) = (SpaceId::from_u128(a), SpaceId::from_u128(b));
        prop_assert_eq!(x.cmp(&y), x.to_string().cmp(&y.to_string()));
    }

    #[test]
    fn uuid_and_object_id_conversions_are_lossless(raw in any::<u128>()) {
        let id = DocId::from_u128(raw);
        prop_assert_eq!(DocId::from_uuid(id.as_uuid()), id);
        prop_assert_eq!(DocId::from(ObjectId::from(id)), id);
    }

    #[test]
    fn serde_round_trips(raw in any::<u128>(), block in 0..=BlockId::MAX) {
        let id = DocId::from_u128(raw);
        let json = serde_json::to_string(&id).unwrap_or_default();
        prop_assert_eq!(serde_json::from_str::<DocId>(&json).ok(), Some(id));
        let block = BlockId::from_u64(block);
        let json = serde_json::to_string(&block).unwrap_or_default();
        prop_assert_eq!(
            serde_json::from_str::<Option<BlockId>>(&json).ok(),
            Some(block)
        );
    }

    #[test]
    fn block_id_text_round_trips(raw in 0..=BlockId::MAX) {
        let id = BlockId::from_u64(raw);
        prop_assert!(id.is_some());
        let text = id.map(|b| b.to_string()).unwrap_or_default();
        prop_assert!(text.starts_with("blk-"));
        prop_assert_eq!(text.len(), 12);
        prop_assert_eq!(&text.to_ascii_lowercase(), &text);
        prop_assert_eq!(text.parse::<BlockId>().ok(), id);
    }

    #[test]
    fn parsing_arbitrary_text_never_panics(text in ".{0,40}") {
        let _ = text.parse::<DocId>();
        let _ = text.parse::<BlockId>();
        let _ = format!("blk-{text}").parse::<BlockId>();
    }
}

#[test]
fn generated_ids_are_unique_and_ordered() {
    let ids: Vec<DocId> = (0..1000).map(|_| DocId::generate()).collect();
    assert!(ids.windows(2).all(|w| matches!(w, [a, b] if a < b)));
}

#[test]
fn json_schemas_describe_the_text_forms() {
    let doc =
        serde_json::to_value(schemars::schema_for!(DocId)).unwrap_or_default();
    assert_eq!(
        doc.pointer("/format").and_then(|v| v.as_str()),
        Some("ulid")
    );
    let pattern = doc
        .pointer("/pattern")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    assert!(pattern.starts_with("^[0-7]"));
    let block = serde_json::to_value(schemars::schema_for!(BlockId))
        .unwrap_or_default();
    assert_eq!(
        block.pointer("/type").and_then(|v| v.as_str()),
        Some("string")
    );
}
