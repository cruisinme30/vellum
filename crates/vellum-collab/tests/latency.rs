// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Keystroke-to-peers latency through the hub: 10 rooms × 4 editors, every
//! editor typing. The brief's target is p95 < 200 ms over the internet;
//! the server's share of it must be a small fraction, so this gate is
//! p95 < 20 ms in-process.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::{Duration, Instant};

use support::{Editor, fixture, room, type_into};
use vellum_collab::{Access, CollabConfig, Outbound};

const ROOMS: usize = 10;
const EDITORS: usize = 4;
const KEYSTROKES: usize = 25;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keystrokes_reach_peers_fast() {
    let f = fixture(CollabConfig::default());
    let mut samples = Vec::new();
    for _ in 0..ROOMS {
        let key = room();
        let mut editors = Vec::new();
        for _ in 0..EDITORS {
            editors.push(Editor::join(&f.hub, key, Access::Write).await);
        }
        for editor in &mut editors {
            editor.pump().await;
        }
        for stroke in 0..KEYSTROKES {
            let typist = stroke % EDITORS;
            let started = Instant::now();
            editors[typist].edit(|doc| type_into(doc, 0, "x")).await;
            for (index, editor) in editors.iter_mut().enumerate() {
                if index == typist {
                    continue;
                }
                // Wait for the update frame, then apply it.
                loop {
                    let next = tokio::time::timeout(
                        Duration::from_secs(2),
                        editor.connection.recv(),
                    )
                    .await
                    .expect("a frame in time")
                    .expect("open");
                    // A sync (0) update (2) frame is the keystroke.
                    if let Outbound::Frame(frame) = next
                        && frame.starts_with(&[0, 2])
                    {
                        break;
                    }
                }
                samples.push(started.elapsed());
            }
        }
    }
    samples.sort();
    let p95 = samples[samples.len() * 95 / 100];
    let p50 = samples[samples.len() / 2];
    println!(
        "keystroke to peer: p50 {p50:?}, p95 {p95:?}, n = {}",
        samples.len()
    );
    assert!(p95 < Duration::from_millis(20), "p95 {p95:?}");
}
