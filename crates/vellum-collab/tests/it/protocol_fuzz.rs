// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Protocol fuzzing: arbitrary frames and arbitrary update sequences never
//! take a room down; the sender is dropped at worst.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use proptest::prelude::*;
use support::{Editor, add_paragraph, fixture, room};
use vellum_collab::protocol::{self, MSG_DONE, MSG_TRACE};
use vellum_collab::{Access, CollabConfig};
use yrs::sync::{Message, SyncMessage};

fn message() -> impl Strategy<Value = Message> {
    let bytes = prop::collection::vec(any::<u8>(), 0..64);
    prop_oneof![
        bytes
            .clone()
            .prop_map(|b| Message::Sync(SyncMessage::Update(b))),
        bytes
            .clone()
            .prop_map(|b| Message::Sync(SyncMessage::SyncStep2(b))),
        Just(Message::AwarenessQuery),
        bytes.clone().prop_map(|b| Message::Custom(MSG_TRACE, b)),
        Just(Message::Custom(MSG_DONE, Vec::new())),
        (4..255_u8, bytes).prop_map(|(tag, b)| Message::Custom(tag, b)),
    ]
}

fn frame() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..256),
        prop::collection::vec(message(), 1..6)
            .prop_map(|m| protocol::encode(&m)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn rooms_survive_arbitrary_frames(
        frames in prop::collection::vec(frame(), 1..12)
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let f = fixture(CollabConfig::default());
            let key = room();
            let fuzzer = Editor::join(&f.hub, key, Access::Write).await;
            for frame in frames {
                if fuzzer.connection.send(frame).await.is_err() {
                    break;
                }
            }
            let writer = Editor::join(&f.hub, key, Access::Write).await;
            writer.edit(|doc| add_paragraph(doc, "alive")).await;
            let mut reader = Editor::join(&f.hub, key, Access::Read).await;
            assert!(
                reader.pump_until(|e| e.markdown().contains("alive")).await
            );
        });
    }
}
