// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The SQL index keeps the shared index tables in step with the drive:
//! `page_ancestors` (the tree authorization reads), `folders` and
//! `documents` match the drive after creates, moves within and across
//! spaces, trash, restore and purge; and `vellum reindex`, which rebuilds
//! the shared tables from files, loses no drive state.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_db::sqlx::types::Uuid;
use vellum_db::{Index, IndexConfig, IndexPool, PlainFiles, sqlx};
use vellum_drive::memory::MemoryAccess;
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, DriveIndex, Node, NodeKind, SpaceKind};
use vellum_kernel::{Actor, ObjectId, UserId};
use vellum_store::LocalStore;

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// (space, ancestor, descendant, depth)
type Closure = BTreeSet<(Uuid, Uuid, Uuid, i32)>;
/// folder → (space, parent, path)
type Folders = BTreeMap<Uuid, (Uuid, Option<Uuid>, String)>;
/// document → (space, folder, path, trashed)
type Documents = BTreeMap<Uuid, (Uuid, Option<Uuid>, String, bool)>;

const fn id(value: ObjectId) -> Uuid {
    Uuid::from_u128(value.as_u128())
}

/// What the shared tables should hold for these nodes.
fn expected(spaces: &[Uuid], nodes: &[Node]) -> (Closure, Folders, Documents) {
    let by_id: HashMap<ObjectId, &Node> = nodes.iter().map(|n| (n.id, n)).collect();
    let tree_parent = |n: &Node| {
        n.trash
            .as_ref()
            .map_or(n.parent, |t| t.original_parent)
            .filter(|p| by_id.contains_key(p))
    };
    // The space is the root, above every top-level node.
    let mut closure: Closure = spaces.iter().map(|s| (*s, *s, *s, 0)).collect();
    for node in nodes {
        let space = id(node.space.into());
        let mut depth = 0;
        let mut current = Some(node.id);
        while let Some(at) = current {
            let _ = closure.insert((space, id(at), id(node.id), depth));
            depth += 1;
            current = by_id.get(&at).and_then(|n| tree_parent(n));
        }
        let _ = closure.insert((space, space, id(node.id), depth));
    }
    let live_folder = |p: Option<ObjectId>| {
        p.filter(|p| {
            by_id
                .get(p)
                .is_some_and(|n| n.kind == NodeKind::Folder && n.is_live())
        })
        .map(id)
    };
    let mut folders = Folders::new();
    let mut documents = Documents::new();
    for node in nodes {
        let row = (id(node.space.into()), live_folder(node.parent));
        if node.kind == NodeKind::Folder {
            if node.is_live() {
                let _ = folders.insert(id(node.id), (row.0, row.1, node.path.as_str().to_owned()));
            }
        } else {
            let _ = documents.insert(
                id(node.id),
                (
                    row.0,
                    row.1,
                    node.path.as_str().to_owned(),
                    node.trash.is_some(),
                ),
            );
        }
    }
    (closure, folders, documents)
}

macro_rules! read_shared {
    ($pool:expr) => {{
        let pool = $pool;
        let closure: Vec<(Uuid, Uuid, Uuid, i32)> = sqlx::query_as(
            "SELECT space_id, ancestor_id, descendant_id, depth FROM page_ancestors",
        )
        .fetch_all(pool)
        .await?;
        let folders: Vec<(Uuid, Uuid, Option<Uuid>, String)> =
            sqlx::query_as("SELECT folder_id, space_id, parent_id, path FROM folders")
                .fetch_all(pool)
                .await?;
        let documents: Vec<(Uuid, Uuid, Option<Uuid>, String, Option<i64>)> =
            sqlx::query_as("SELECT doc_id, space_id, folder_id, path, trashed_at FROM documents")
                .fetch_all(pool)
                .await?;
        (
            closure.into_iter().collect::<Closure>(),
            folders
                .into_iter()
                .map(|(f, s, p, path)| (f, (s, p, path)))
                .collect::<Folders>(),
            documents
                .into_iter()
                .map(|(d, s, f, path, t)| (d, (s, f, path, t.is_some())))
                .collect::<Documents>(),
        )
    }};
}

async fn shared(
    db: &Index,
) -> Result<(Closure, Folders, Documents), Box<dyn std::error::Error + Send + Sync>> {
    Ok(match db.pool() {
        IndexPool::Sqlite(pool) => read_shared!(pool),
        IndexPool::Postgres(pool) => read_shared!(pool),
        _ => return Err("unknown backend".into()),
    })
}

async fn all_nodes(
    index: &SqlIndex,
) -> Result<Vec<Node>, Box<dyn std::error::Error + Send + Sync>> {
    let mut out = Vec::new();
    for space in index.spaces().await? {
        out.extend(index.nodes(space.id).await?);
    }
    Ok(out)
}

async fn assert_in_step(index: &SqlIndex, step: &str) -> TestResult {
    let nodes = all_nodes(index).await?;
    let spaces: Vec<Uuid> = index
        .spaces()
        .await?
        .iter()
        .map(|s| id(s.id.into()))
        .collect();
    let (closure, folders, documents) = shared(index.db()).await?;
    let (want_closure, want_folders, want_documents) = expected(&spaces, &nodes);
    assert_eq!(closure, want_closure, "page_ancestors after {step}");
    assert_eq!(folders, want_folders, "folders after {step}");
    assert_eq!(documents, want_documents, "documents after {step}");
    Ok(())
}

async fn shared_tables_follow_the_drive(db: Index) -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index = Arc::new(SqlIndex::new(db));
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let drive = Drive::new(store.clone(), index.clone(), access.clone());
    let alice_id = UserId::generate();
    let alice = Actor::User { user: alice_id };

    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let ops = drive
        .create_space(&alice, SpaceKind::SharedDrive, "ops", "Operations", None)
        .await?;
    let a = drive.create_folder(&alice, eng.id, None, "A").await?;
    let b = drive.create_folder(&alice, eng.id, Some(a.id), "B").await?;
    let top = drive.create_page(&alice, eng.id, None, "Top", "x").await?;
    let deep = drive
        .create_page(&alice, eng.id, Some(b.id), "Deep", "[[Top]]")
        .await?;
    assert_in_step(&index, "creates").await?;
    assert_eq!(index.backlinks(top.id).await?, vec![deep.id]);

    let _ = drive.move_node(&alice, b.id, eng.id, None).await?;
    assert_in_step(&index, "a move to the top level").await?;
    let _ = drive.move_node(&alice, b.id, eng.id, Some(a.id)).await?;
    let _ = drive.rename(&alice, a.id, "Renamed").await?;
    assert_in_step(&index, "a move back and a rename").await?;

    let _ = drive.move_node(&alice, a.id, ops.id, None).await?;
    assert_in_step(&index, "a move to another space").await?;

    let _ = drive.trash(&alice, a.id).await?;
    assert_in_step(&index, "trashing a folder").await?;
    let _ = drive.restore(&alice, a.id).await?;
    assert_in_step(&index, "restoring it").await?;

    drive.star(&alice, deep.id, true).await?;
    let _ = drive.read(&alice, top.id).await?;
    let _ = drive.trash(&alice, b.id).await?;
    drive.purge(&alice, b.id).await?;
    assert_in_step(&index, "purging a folder").await?;
    assert!(index.node(deep.id).await?.is_none());
    assert!(drive.starred(&alice).await?.is_empty());

    // A reindex rebuilds the shared tables from files and keeps what
    // only the database holds: stars, recents and activity.
    drive.star(&alice, top.id, true).await?;
    assert_eq!(
        index.starred(alice_id).await?,
        vec![top.id],
        "before reindex"
    );
    let activity = index.activity(top.id, 10).await?.len();
    let recent = index.recent(alice_id, 10).await?;
    let _ = index.db().reindex(dir.path(), PlainFiles).await?;
    assert_eq!(index.starred(alice_id).await?, vec![top.id]);
    assert_eq!(index.recent(alice_id, 10).await?, recent);
    assert_eq!(index.activity(top.id, 10).await?.len(), activity);
    assert_eq!(index.spaces().await?.len(), 2);
    // The next drive write puts the drive's view back.
    let _ = drive.scan().await?;
    let _ = drive.create_page(&alice, eng.id, None, "After", "").await?;
    for node in all_nodes(&index).await? {
        index.put_node(&node).await?;
    }
    assert_in_step(&index, "a reindex and a rescan").await?;
    Ok(())
}

#[tokio::test]
async fn shared_tables_follow_the_drive_on_sqlite() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None)).await?;
    shared_tables_follow_the_drive(db).await
}

#[tokio::test]
async fn shared_tables_follow_the_drive_on_postgres() -> TestResult {
    let container: ContainerAsync<Postgres> =
        Postgres::default().with_tag("17-alpine").start().await?;
    let port = container.get_host_port_ipv4(5432).await?;
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    shared_tables_follow_the_drive(Index::open(&IndexConfig::postgres(url)).await?).await
}

/// Two shards (ADR 0040 D174): each space's rows live on its own shard,
/// each shard's shared tables stay in step, and moves between shards
/// leave nothing behind.
#[tokio::test]
async fn spaces_on_two_shards_keep_each_shard_in_step() -> TestResult {
    use vellum_db::{ShardRouter, ShardedIndex as Shards, ShardsConfig};
    use vellum_drive::sharded::ShardedIndex;

    let dir = tempfile::tempdir()?;
    let db = |name: &str| IndexConfig::sqlite(dir.path().join(format!("{name}.sqlite")), None);
    let config = ShardsConfig {
        main: db("main"),
        shards: vec![("eu".into(), db("eu")), ("us".into(), db("us"))],
    };
    let router = Arc::new(ShardRouter::open(Shards::open(&config).await?).await?);
    let files = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(files.path()).await?);
    let index = Arc::new(ShardedIndex::new(router));
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let drive = Drive::new(store, index.clone(), access);
    let alice_id = UserId::generate();
    let alice = Actor::User { user: alice_id };

    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let ops = drive
        .create_space(&alice, SpaceKind::SharedDrive, "ops", "Operations", None)
        .await?;
    let [eu, us] = index.shards() else {
        panic!("two shards")
    };
    assert_eq!(eu.spaces().await?.len(), 1, "one space per shard");
    assert_eq!(us.spaces().await?.len(), 1, "one space per shard");
    assert_eq!(index.spaces().await?.len(), 2);

    let folder = drive.create_folder(&alice, eng.id, None, "A").await?;
    let top = drive.create_page(&alice, ops.id, None, "Top", "x").await?;
    let page = drive
        .create_page(&alice, eng.id, Some(folder.id), "Deep", "[[Top]]")
        .await?;
    drive.star(&alice, top.id, true).await?;
    drive.star(&alice, page.id, true).await?;
    let _ = drive.read(&alice, top.id).await?;
    let _ = drive.read(&alice, page.id).await?;
    for shard in index.shards() {
        assert_in_step(shard, "creates on two shards").await?;
    }
    let starred: BTreeSet<ObjectId> = drive
        .starred(&alice)
        .await?
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(starred, BTreeSet::from([top.id, page.id]));
    let recent: BTreeSet<ObjectId> = index
        .recent(alice_id, 10)
        .await?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert!(recent.contains(&top.id) && recent.contains(&page.id));

    // Moving a folder to the other shard moves its rows.
    let _ = drive.move_node(&alice, folder.id, ops.id, None).await?;
    for shard in index.shards() {
        assert_in_step(shard, "a move between shards").await?;
    }
    let moved = index.node(page.id).await?.unwrap();
    assert_eq!(moved.space, ops.id);
    Ok(())
}
