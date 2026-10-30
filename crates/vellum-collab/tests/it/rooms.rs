// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Room behaviour through the hub: sync, batching, versions, access,
//! awareness, re-homing and draining.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use crate::support;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use support::{Editor, TestCodec, add_paragraph, fixture, room, type_into};
use vellum_collab::protocol::{self, MSG_DONE};
use vellum_collab::{
    Access, CloseReason, CollabConfig, MarkdownCodec as _, Outbound, VersionReason,
};
use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry};
use yrs::sync::{Message, SyncMessage};
use yrs::{ClientID, ReadTxn as _, Transact as _};

fn quick() -> CollabConfig {
    CollabConfig {
        batch_window: Duration::from_millis(10),
        ..CollabConfig::default()
    }
}

#[tokio::test]
async fn editors_start_from_the_file_and_converge() {
    let f = fixture(quick());
    let key = room();
    f.files.put(key, "# Plan\n\nHello\n");

    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    let mut bob = Editor::join(&f.hub, key, Access::Write).await;
    assert!(
        alice
            .pump_until(|e| e.markdown() == "# Plan\n\nHello\n")
            .await
    );
    assert!(
        bob.pump_until(|e| e.markdown() == "# Plan\n\nHello\n")
            .await
    );

    alice.edit(|doc| type_into(doc, 1, " world")).await;
    bob.edit(|doc| add_paragraph(doc, "From Bob")).await;
    let expected_a = "# Plan\n\nHello world\n\nFrom Bob\n";
    assert!(
        alice.pump_until(|e| e.markdown() == expected_a).await,
        "{}",
        alice.markdown()
    );
    assert!(
        bob.pump_until(|e| e.markdown() == expected_a).await,
        "{}",
        bob.markdown()
    );
    assert_eq!(f.hub.open_rooms(), 1);
}

#[tokio::test]
async fn done_writes_a_version_and_acknowledges() {
    let f = fixture(quick());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    alice.edit(|doc| add_paragraph(doc, "Ship it")).await;
    alice.send(&[Message::Custom(MSG_DONE, Vec::new())]).await;
    assert!(alice.pump_until(|e| e.dones == 1).await);

    let versions = f.files.versions(key);
    assert_eq!(versions.len(), 1);
    let version = versions.first().unwrap();
    assert_eq!(version.markdown, "Ship it\n");
    assert_eq!(version.reason, VersionReason::Done);
    assert_eq!(version.editors, vec![alice.user]);

    // Done again with nothing new acknowledges without a second version.
    alice.send(&[Message::Custom(MSG_DONE, Vec::new())]).await;
    assert!(alice.pump_until(|e| e.dones == 2).await);
    assert_eq!(f.files.versions(key).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn idle_writes_a_version_after_thirty_seconds() {
    let f = fixture(CollabConfig::default());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    alice.edit(|doc| add_paragraph(doc, "Draft")).await;
    alice.pump().await;

    tokio::time::sleep(Duration::from_secs(29)).await;
    assert!(f.files.versions(key).is_empty());
    tokio::time::sleep(Duration::from_secs(2)).await;
    alice.pump().await;
    let versions = f.files.versions(key);
    assert_eq!(versions.len(), 1, "{versions:?}");
    assert_eq!(versions.first().unwrap().reason, VersionReason::Idle);
}

#[tokio::test(start_paused = true)]
async fn updates_are_appended_in_batches_within_fifty_ms() {
    let f = fixture(CollabConfig::default());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    for word in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        alice.edit(|doc| type_into(doc, 0, word)).await;
    }
    alice.pump().await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    assert_eq!(
        f.store.record_count(key),
        0,
        "nothing written before the window"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    alice.pump().await;
    assert_eq!(f.store.record_count(key), 1, "one merged batch");
}

#[tokio::test]
async fn readers_see_edits_but_cannot_make_them() {
    let f = fixture(quick());
    let key = room();
    let mut writer = Editor::join(&f.hub, key, Access::Write).await;
    let mut reader = Editor::join(&f.hub, key, Access::Read).await;
    reader.edit(|doc| add_paragraph(doc, "sneaky")).await;
    writer.edit(|doc| add_paragraph(doc, "real")).await;
    assert!(reader.pump_until(|e| e.markdown().contains("real")).await);
    writer.pump().await;
    assert_eq!(writer.markdown(), "real\n");
}

#[tokio::test]
async fn revoking_access_disconnects_at_once() {
    let f = fixture(quick());
    let key = room();
    let mut bob = Editor::join(&f.hub, key, Access::Write).await;
    bob.pump().await;
    f.hub.set_access(key, bob.user, None);
    assert!(bob.pump_until(|e| e.closed.is_some()).await);
    assert_eq!(bob.closed, Some(Outbound::Close(CloseReason::Revoked)));
}

#[tokio::test]
async fn downgrading_to_read_stops_edits() {
    let f = fixture(quick());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    let mut bob = Editor::join(&f.hub, key, Access::Write).await;
    bob.pump().await;
    f.hub.set_access(key, bob.user, Some(Access::Read));
    bob.edit(|doc| add_paragraph(doc, "after downgrade")).await;
    alice.edit(|doc| add_paragraph(doc, "alice")).await;
    assert!(bob.pump_until(|e| e.markdown().contains("alice")).await);
    alice.pump().await;
    assert!(!alice.markdown().contains("after downgrade"));
}

fn awareness(client: u64, json: &str) -> AwarenessUpdate {
    let mut clients = HashMap::new();
    let _previous = clients.insert(
        ClientID::new(client),
        AwarenessUpdateEntry {
            clock: 1,
            json: Arc::from(json),
        },
    );
    AwarenessUpdate { clients }
}

#[tokio::test]
async fn awareness_is_shared_and_cannot_be_spoofed() {
    let f = fixture(quick());
    let key = room();
    let alice = Editor::join(&f.hub, key, Access::Write).await;
    let bob = Editor::join(&f.hub, key, Access::Read).await;
    let mut carol = Editor::join(&f.hub, key, Access::Write).await;

    alice
        .send(&[Message::Awareness(awareness(11, r#"{"cursor":1}"#))])
        .await;
    assert!(
        carol
            .pump_until(|e| e.peers.iter().any(|p| p.client_id == 11))
            .await
    );
    let entry = carol.peers.iter().find(|p| p.client_id == 11).unwrap();
    assert_eq!(entry.user, alice.user);
    assert!(entry.can_write);

    // Bob tries to move Alice's cursor: refused, so nobody hears about it.
    bob.send(&[Message::Awareness(awareness(11, r#"{"cursor":99}"#))])
        .await;
    bob.send(&[Message::Awareness(awareness(22, r#"{"cursor":5}"#))])
        .await;
    assert!(
        carol
            .pump_until(|e| e.peers.iter().any(|p| p.client_id == 22))
            .await
    );
    let spoofed = carol
        .awareness
        .iter()
        .flat_map(|u| u.clients.iter())
        .any(|(id, e)| id.get() == 11 && e.json.contains("99"));
    assert!(!spoofed);
    let bob_entry = carol.peers.iter().find(|p| p.client_id == 22).unwrap();
    assert_eq!(bob_entry.user, bob.user);
    assert!(!bob_entry.can_write);

    // Leaving clears the cursor for everyone.
    drop(alice);
    assert!(
        carol
            .pump_until(|e| e.peers.iter().all(|p| p.client_id != 11))
            .await
    );
}

#[tokio::test]
async fn a_new_node_recreates_the_room_from_the_log() {
    let f = fixture(quick());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    for n in 0..200 {
        alice
            .edit(|doc| add_paragraph(doc, &format!("line {n}")))
            .await;
    }
    alice.pump().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    let expected = alice.markdown();

    // Another node takes the room over (the first one is gone).
    let node2 = f.second_node(quick());
    let started = std::time::Instant::now();
    let mut bob = Editor::join(&node2, key, Access::Write).await;
    assert!(bob.pump_until(|e| e.markdown() == expected).await);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "re-home took {:?}",
        started.elapsed()
    );
}

/// Collab re-home under load (#191, ADR 0061 D268): three editors keep
/// typing while their node drains. They carry on offline, reconnect to
/// another node with their documents, and no line from anyone is lost.
#[tokio::test]
async fn editors_typing_through_a_drain_lose_nothing_on_the_new_node() {
    let f = fixture(quick());
    let key = room();
    let mut editors = Vec::new();
    for _ in 0..3 {
        editors.push(Editor::join(&f.hub, key, Access::Write).await);
    }
    let line = |who: usize, n: usize| format!("editor {who} line {n}");
    for n in 0..10 {
        for (who, editor) in editors.iter().enumerate() {
            editor.edit(|doc| add_paragraph(doc, &line(who, n))).await;
        }
    }

    // The node drains while they type; edits after this are offline.
    let drain = {
        let hub = f.hub.clone();
        tokio::spawn(async move { hub.shutdown().await })
    };
    for n in 10..20 {
        for (who, editor) in editors.iter().enumerate() {
            // Typing goes on: sent while the room is open, kept locally after.
            let before = editor.doc.transact().state_vector();
            add_paragraph(&editor.doc, &line(who, n));
            let update = editor.doc.transact().encode_state_as_update_v1(&before);
            let frame = protocol::encode(&[Message::Sync(SyncMessage::Update(update))]);
            let _ = editor.connection.send(frame).await;
        }
    }
    drain.await.unwrap();
    // Every connection is told to go elsewhere; nothing more is sent to it.
    for editor in &mut editors {
        let closed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match editor.connection.recv().await {
                    Some(Outbound::Frame(_)) => {}
                    other => return other,
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(closed, Some(Outbound::Close(CloseReason::Shutdown)));
    }

    // Each reconnects to the new owner with the document it has.
    let node2 = f.second_node(quick());
    let mut rejoined = Vec::new();
    for editor in editors {
        let doc = editor.doc.clone();
        rejoined.push(Editor::join_with(&node2, key, Access::Write, editor.user, doc).await);
    }
    let complete = |e: &Editor| {
        let text = e.markdown();
        (0..3).all(|who| (0..20).all(|n| text.contains(&line(who, n))))
    };
    let mut reader = Editor::join(&node2, key, Access::Read).await;
    for editor in &mut rejoined {
        editor.pump().await;
    }
    assert!(reader.pump_until(complete).await, "{}", reader.markdown());
    for editor in &mut rejoined {
        assert!(editor.pump_until(complete).await, "{}", editor.markdown());
    }
}

#[tokio::test]
async fn edits_not_yet_written_replay_from_the_editor_after_a_crash() {
    // A window so long that nothing reaches the log before the "crash".
    let slow = CollabConfig {
        batch_window: Duration::from_mins(10),
        ..CollabConfig::default()
    };
    let f = fixture(slow);
    let key = room();
    f.files.put(key, "Seed\n");
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    assert!(alice.pump_until(|e| e.markdown() == "Seed\n").await);
    alice.edit(|doc| add_paragraph(doc, "unsaved")).await;
    alice.pump().await;
    assert_eq!(f.store.record_count(key), 1, "only the seed is in the log");

    // The node dies; Alice reconnects to another one with her document.
    let node2 = f.second_node(quick());
    let doc = alice.doc.clone();
    let mut alice2 = Editor::join_with(&node2, key, Access::Write, alice.user, doc).await;
    alice2.pump().await;
    let mut bob = Editor::join(&node2, key, Access::Write).await;
    assert!(
        bob.pump_until(|e| e.markdown() == "Seed\n\nunsaved\n")
            .await,
        "{}",
        bob.markdown()
    );
}

#[tokio::test]
async fn external_markdown_edits_merge_with_live_typing() {
    let f = fixture(quick());
    let key = room();
    f.files.put(key, "One\n\nTwo\n\nThree\n");
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    assert!(
        alice
            .pump_until(|e| e.markdown() == "One\n\nTwo\n\nThree\n")
            .await
    );

    // Alice types in block 0 while the file changes block 2 on disk.
    alice.edit(|doc| type_into(doc, 0, " (typed)")).await;
    let applied = f
        .hub
        .apply_markdown(key, "One\n\nTwo\n\nThree **changed**\n".to_owned())
        .await
        .unwrap();
    assert!(applied);
    let expected = "One (typed)\n\nTwo\n\nThree **changed**\n";
    assert!(
        alice.pump_until(|e| e.markdown() == expected).await,
        "{}",
        alice.markdown()
    );

    // No open room: the caller writes the file itself.
    assert!(
        !f.hub
            .apply_markdown(room(), "x\n".to_owned())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn text_written_while_a_room_is_closed_wins_when_merged() {
    let config = CollabConfig {
        close_after_empty: Duration::from_millis(50),
        ..quick()
    };
    let f = fixture(config);
    let key = room();
    f.files.put(key, "Old\n");
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    assert!(alice.pump_until(|e| e.markdown() == "Old\n").await);
    alice.edit(|doc| add_paragraph(doc, "New")).await;
    alice.send(&[Message::Custom(MSG_DONE, Vec::new())]).await;
    assert!(alice.pump_until(|e| e.dones == 1).await);
    drop(alice);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(f.hub.open_rooms(), 0);

    // An old version is restored through the API while nobody edits.
    f.files.put(key, "Old\n");
    f.hub.merge_markdown(key, "Old\n".to_owned()).await.unwrap();

    // The room's log no longer brings the replaced text back.
    let mut bob = Editor::join(&f.hub, key, Access::Write).await;
    assert!(
        bob.pump_until(|e| e.markdown() == "Old\n").await,
        "{}",
        bob.markdown()
    );
    drop(bob);
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The file already held the restored text: no second copy is written.
    assert_eq!(f.files.versions(key).len(), 1);
}

#[tokio::test]
async fn merging_into_a_page_never_edited_live_opens_no_room() {
    let f = fixture(quick());
    let key = room();
    f.files.put(key, "Hello\n");
    f.hub
        .merge_markdown(key, "Hello again\n".to_owned())
        .await
        .unwrap();
    assert_eq!(f.hub.open_rooms(), 0);
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    assert!(alice.pump_until(|e| e.markdown() == "Hello\n").await);
}

#[tokio::test(start_paused = true)]
async fn a_storage_outage_delays_writes_but_loses_nothing() {
    let f = fixture(CollabConfig::default());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    f.store.set_failing(true);
    alice.edit(|doc| add_paragraph(doc, "during outage")).await;
    alice.pump().await;
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert_eq!(f.store.record_count(key), 0);

    f.store.set_failing(false);
    alice.edit(|doc| add_paragraph(doc, "after")).await;
    alice.pump().await;
    tokio::time::sleep(Duration::from_secs(1)).await;

    let node2 = f.second_node(CollabConfig::default());
    let mut bob = Editor::join(&node2, key, Access::Write).await;
    assert!(
        bob.pump_until(|e| e.markdown() == "during outage\n\nafter\n")
            .await,
        "{}",
        bob.markdown()
    );
}

#[tokio::test]
async fn shutdown_writes_versions_and_closes_connections() {
    let f = fixture(quick());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    alice.edit(|doc| add_paragraph(doc, "before drain")).await;
    alice.pump().await;
    f.hub.shutdown().await;
    assert!(alice.pump_until(|e| e.closed.is_some()).await);
    assert_eq!(alice.closed, Some(Outbound::Close(CloseReason::Shutdown)));
    let versions = f.files.versions(key);
    assert_eq!(versions.len(), 1);
    assert_eq!(versions.first().unwrap().reason, VersionReason::Shutdown);
    assert_eq!(f.hub.open_rooms(), 0);
}

#[tokio::test(start_paused = true)]
async fn an_empty_room_closes_after_writing_its_version() {
    let f = fixture(CollabConfig::default());
    let key = room();
    let alice = Editor::join(&f.hub, key, Access::Write).await;
    alice.edit(|doc| add_paragraph(doc, "bye")).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(alice);
    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(f.hub.open_rooms(), 0);
    let versions = f.files.versions(key);
    assert_eq!(versions.len(), 1);
    assert_eq!(versions.first().unwrap().reason, VersionReason::Closed);
    assert_eq!(versions.first().unwrap().markdown, "bye\n");
    // The log was compacted to one record.
    assert_eq!(f.store.record_count(key), 1);

    // Reopening works and starts from the log.
    let mut bob = Editor::join(&f.hub, key, Access::Write).await;
    assert!(bob.pump_until(|e| e.markdown() == "bye\n").await);
}

#[tokio::test]
async fn malformed_and_oversized_frames_drop_only_the_sender() {
    let f = fixture(CollabConfig {
        max_message_bytes: 1024,
        ..quick()
    });
    let key = room();
    let mut good = Editor::join(&f.hub, key, Access::Write).await;
    let mut bad = Editor::join(&f.hub, key, Access::Write).await;
    bad.connection.send(vec![0, 2, 200, 1, 2, 3]).await.unwrap();
    assert!(bad.pump_until(|e| e.closed.is_some()).await);
    assert_eq!(bad.closed, Some(Outbound::Close(CloseReason::Protocol)));

    let mut big = Editor::join(&f.hub, key, Access::Write).await;
    big.connection.send(vec![0; 2048]).await.unwrap();
    assert!(big.pump_until(|e| e.closed.is_some()).await);
    assert_eq!(big.closed, Some(Outbound::Close(CloseReason::TooLarge)));

    good.edit(|doc| add_paragraph(doc, "still here")).await;
    let mut other = Editor::join(&f.hub, key, Access::Write).await;
    assert!(other.pump_until(|e| e.markdown() == "still here\n").await);
    good.pump().await;
}

#[tokio::test]
async fn the_hub_done_call_writes_a_version() {
    let f = fixture(quick());
    let key = room();
    let mut alice = Editor::join(&f.hub, key, Access::Write).await;
    alice.edit(|doc| add_paragraph(doc, "via api")).await;
    alice.pump().await;
    f.hub.done(key).await.unwrap();
    assert_eq!(
        f.files.versions(key).first().map(|v| v.markdown.clone()),
        Some("via api\n".to_owned())
    );
    // Rooms that are not open need nothing.
    f.hub.done(room()).await.unwrap();
}

#[test]
fn the_test_codec_is_canonical() {
    let md = "---\ntitle: Plan\n---\n\n# Plan\n\nSome **bold** text\n";
    let tree = TestCodec.parse_markdown(md).unwrap();
    assert_eq!(TestCodec.to_markdown(&tree).unwrap(), md);
    let state = yrs::Doc::new().transact().state_vector();
    let frame = protocol::encode(&[Message::Sync(SyncMessage::SyncStep1(state))]);
    assert!(!frame.is_empty());
}
