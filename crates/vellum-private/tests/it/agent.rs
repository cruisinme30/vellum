// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The private agent over a real folder: Northstar's board keeps an
//! end-to-end encrypted space, and only enrolled devices can search it.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "test helpers fail loudly"
)]

use std::sync::Arc;

use bytes::Bytes;
use vellum_crypto::recipients::{RecipientPublic, RecipientSecret, seal_for};
use vellum_private::{AgentError, DeviceIdentity, PrivateAgent, PrivateSpace, Report};
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition};

const SPACE_ID: &str = "01JDB0ARD00000000000000000";
const ROOT: &str = "drives/shared/board";

fn page(id: &str, title: &str, body: &str) -> Vec<u8> {
    format!("---\nid: {id}\ntitle: {title}\nvellum: 1\n---\n\n{body}\n").into_bytes()
}

async fn put(store: &LocalStore, path: &str, bytes: Vec<u8>) {
    let _ = store
        .put(
            &BlobPath::new(path).unwrap(),
            Bytes::from(bytes),
            Precondition::None,
        )
        .await
        .unwrap();
}

struct World {
    dir: tempfile::TempDir,
    store: Arc<LocalStore>,
    me: RecipientSecret,
    colleague: RecipientPublic,
}

/// A space with five files: two pages sealed to this device (and a
/// colleague), one sealed to the colleague only, one tampered with, and one
/// left in plain text.
async fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let me = RecipientSecret::generate().unwrap();
    let colleague = RecipientSecret::generate().unwrap().public();
    let both = [me.public(), colleague];
    put(
        &store,
        &format!("{ROOT}/.vellum-folder.yaml"),
        format!("id: {SPACE_ID}\n").into_bytes(),
    )
    .await;
    let plan = page(
        "01JDB0ARDP1AN0000000000001",
        "Fundraising plan",
        "# Q4\n\nRaise the seed round in December with Northstar Ventures.\n\n## Risks\n\nRunway is nine months.",
    );
    put(
        &store,
        &format!("{ROOT}/plan.md"),
        seal_for(&both, &plan).unwrap(),
    )
    .await;
    let hiring = page(
        "01JDB0ARDH1R1NG00000000002",
        "Hiring",
        "Two engineers in January.",
    );
    put(
        &store,
        &format!("{ROOT}/people/hiring.md"),
        seal_for(&both, &hiring).unwrap(),
    )
    .await;
    let theirs = page(
        "01JDB0ARDTHE1R500000000003",
        "Salaries",
        "Not for this device.",
    );
    put(
        &store,
        &format!("{ROOT}/salaries.md"),
        seal_for(&[colleague], &theirs).unwrap(),
    )
    .await;
    let mut tampered = seal_for(&both, &page("01JDB0ARDTAMPER00000000004", "T", "x")).unwrap();
    let last = tampered.len() - 1;
    tampered[last] ^= 1;
    put(&store, &format!("{ROOT}/tampered.md"), tampered).await;
    put(
        &store,
        &format!("{ROOT}/leaked.md"),
        page("01JDB0ARD1EAKED00000000005", "Leak", "plain"),
    )
    .await;
    World {
        dir,
        store,
        me,
        colleague,
    }
}

async fn agent(world: &World) -> PrivateAgent {
    let me = DeviceIdentity::from_secret(world.me.clone());
    let space = PrivateSpace::open(
        world.store.as_ref(),
        &me,
        "board",
        BlobPath::new(ROOT).unwrap(),
    )
    .await
    .unwrap();
    PrivateAgent::new(me, world.store.clone(), vec![space]).unwrap()
}

#[tokio::test]
async fn it_opens_only_what_is_sealed_to_this_device() {
    let world = world().await;
    let agent = agent(&world).await;
    assert_eq!(agent.spaces()[0].id.to_string(), SPACE_ID);
    let report = agent.refresh().await.unwrap();
    assert_eq!(
        report,
        Report {
            opened: 2,
            not_for_this_device: 1,
            unreadable: 1,
            not_sealed: 1,
        }
    );
    let docs = agent.documents("board").await;
    let titles: Vec<_> = docs.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(titles, ["Hiring", "Fundraising plan"]);
    assert!(agent.documents("nope").await.is_empty());
    let _ = world.colleague;
}

#[tokio::test]
async fn searches_find_blocks_with_their_section() {
    let world = world().await;
    let agent = agent(&world).await;
    let _ = agent.refresh().await.unwrap();

    let hits = agent.search("runway", None, 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Fundraising plan");
    assert_eq!(hits[0].heading_path, vec!["Q4", "Risks"]);
    let doc = agent.document(hits[0].doc).await.unwrap();
    assert!(doc.blocks.iter().any(|b| b.id == hits[0].block));

    assert_eq!(
        agent
            .search("seed round", Some("board"), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        agent
            .search("seed round", Some("elsewhere"), 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(agent.search("salaries", None, 10).await.unwrap().is_empty());
    assert!(agent.search("plain", None, 10).await.unwrap().is_empty());

    let by_path = agent
        .document_by_path(&format!("{ROOT}/people/hiring.md"))
        .await
        .unwrap();
    assert_eq!(by_path.title, "Hiring");
}

#[tokio::test]
async fn a_refresh_drops_pages_this_device_lost() {
    let world = world().await;
    let agent = agent(&world).await;
    let _ = agent.refresh().await.unwrap();
    let hiring = page(
        "01JDB0ARDH1R1NG00000000002",
        "Hiring",
        "Two engineers in January.",
    );
    put(
        &world.store,
        &format!("{ROOT}/people/hiring.md"),
        seal_for(&[world.colleague], &hiring).unwrap(),
    )
    .await;
    let report = agent.refresh().await.unwrap();
    assert_eq!(report.opened, 1);
    assert_eq!(report.not_for_this_device, 2);
    assert!(
        agent
            .search("engineers", None, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn nothing_decrypted_reaches_the_disk() {
    let world = world().await;
    let agent = agent(&world).await;
    let _ = agent.refresh().await.unwrap();
    let _ = agent.search("runway", None, 10).await.unwrap();
    for entry in walk(world.dir.path()) {
        let bytes = std::fs::read(&entry).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("Runway is nine months"),
            "{} holds plaintext",
            entry.display()
        );
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[tokio::test]
async fn bad_spaces_are_refused() {
    let world = world().await;
    let me = DeviceIdentity::from_secret(world.me.clone());
    let missing = PrivateSpace::open(
        world.store.as_ref(),
        &me,
        "x",
        BlobPath::new("drives/none").unwrap(),
    )
    .await;
    assert!(missing.is_err());
    put(
        &world.store,
        "drives/bad/.vellum-folder.yaml",
        b"id: nope\n".to_vec(),
    )
    .await;
    let bad = PrivateSpace::open(
        world.store.as_ref(),
        &me,
        "x",
        BlobPath::new("drives/bad").unwrap(),
    )
    .await;
    assert!(matches!(bad, Err(AgentError::NotASpace(_))));

    let space = PrivateSpace::open(
        world.store.as_ref(),
        &me,
        "board",
        BlobPath::new(ROOT).unwrap(),
    )
    .await
    .unwrap();
    let dup = PrivateAgent::new(me, world.store.clone(), vec![space.clone(), space]);
    assert!(matches!(dup, Err(AgentError::DuplicateSlug(_))));
}
