// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The drive service end to end, on a real folder (the local-folder
//! adapter), once per index: in memory, SQLite and Postgres (in a
//! throwaway container). Access is the in-memory role adapter.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use bytes::Bytes;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner as _;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt as _};
use vellum_db::{Index, IndexConfig};
use vellum_drive::layout;
use vellum_drive::memory::{MemoryAccess, MemoryIndex, Role};
use vellum_drive::sql::SqlIndex;
use vellum_drive::{
    ActivityKind, Clock, Drive, DriveError, DriveIndex, Node, NodeKind, Saved, SpaceKind,
    SpaceSettings,
};
use vellum_kernel::{Actor, ObjectId, Timestamp, UserId};
use vellum_store::{Base, BlobPath, BlobStore, LocalStore, Precondition};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug)]
struct ManualClock(AtomicI64);

impl ManualClock {
    fn advance_days(&self, days: i64) {
        let _ = self.0.fetch_add(days * DAY_MS, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_ms(self.0.load(Ordering::SeqCst)).unwrap_or_else(|_| Timestamp::now())
    }
}

/// Which index a test runs on.
#[derive(Debug, Clone, Copy)]
enum Kind {
    Memory,
    Sqlite,
    Postgres,
}

/// Where SQL indexes for one test live: a folder for SQLite files, or a
/// Postgres container with one database per index.
struct Databases {
    kind: Kind,
    dir: tempfile::TempDir,
    postgres: Option<(ContainerAsync<Postgres>, String)>,
    opened: AtomicI64,
}

impl Databases {
    async fn new(kind: Kind) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let postgres = if matches!(kind, Kind::Postgres) {
            let container = Postgres::default().with_tag("17-alpine").start().await?;
            let port = container.get_host_port_ipv4(5432).await?;
            Some((
                container,
                format!("postgres://postgres:postgres@127.0.0.1:{port}"),
            ))
        } else {
            None
        };
        Ok(Self {
            kind,
            dir: tempfile::tempdir()?,
            postgres,
            opened: AtomicI64::new(0),
        })
    }

    /// A new, empty index.
    async fn index(&self) -> Result<Arc<dyn DriveIndex>, Box<dyn std::error::Error + Send + Sync>> {
        let n = self.opened.fetch_add(1, Ordering::SeqCst);
        Ok(match (self.kind, &self.postgres) {
            (Kind::Memory, _) => Arc::new(MemoryIndex::new()),
            (Kind::Sqlite, _) => {
                let path = self.dir.path().join(format!("index-{n}.sqlite"));
                Arc::new(SqlIndex::new(
                    Index::open(&IndexConfig::sqlite(path, None)).await?,
                ))
            }
            (Kind::Postgres, Some((_, base))) => {
                let admin = Index::open(&IndexConfig::postgres(format!("{base}/postgres"))).await?;
                if let vellum_db::IndexPool::Postgres(pool) = admin.pool() {
                    let _ = vellum_db::sqlx::query(vellum_db::sqlx::AssertSqlSafe(format!(
                        "CREATE DATABASE drive_{n}"
                    )))
                    .execute(pool)
                    .await?;
                }
                admin.close().await;
                Arc::new(SqlIndex::new(
                    Index::open(&IndexConfig::postgres(format!("{base}/drive_{n}"))).await?,
                ))
            }
            (Kind::Postgres, None) => return Err("no Postgres container".into()),
        })
    }
}

struct World {
    dir: tempfile::TempDir,
    databases: Databases,
    store: Arc<LocalStore>,
    index: Arc<dyn DriveIndex>,
    access: Arc<MemoryAccess>,
    clock: Arc<ManualClock>,
    drive: Drive,
    alice: Actor,
    alice_id: UserId,
    bob: Actor,
    bob_id: UserId,
}

async fn world(kind: Kind) -> Result<World, Box<dyn std::error::Error + Send + Sync>> {
    let dir = tempfile::tempdir()?;
    let databases = Databases::new(kind).await?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index = databases.index().await?;
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let clock = Arc::new(ManualClock(AtomicI64::new(1_790_000_000_000)));
    let drive = Drive::with_clock(store.clone(), index.clone(), access.clone(), clock.clone());
    let (alice_id, bob_id) = (UserId::generate(), UserId::generate());
    Ok(World {
        dir,
        databases,
        store,
        index,
        access,
        clock,
        drive,
        alice: Actor::User { user: alice_id },
        alice_id,
        bob: Actor::User { user: bob_id },
        bob_id,
    })
}

impl World {
    async fn text(&self, node: &Node) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        Ok(String::from_utf8(
            self.store.get(&node.path).await?.bytes.to_vec(),
        )?)
    }

    async fn node(&self, id: ObjectId) -> Result<Node, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.index.node(id).await?.ok_or("missing node")?)
    }

    async fn edit(
        &self,
        actor: &Actor,
        node: &Node,
        f: impl FnOnce(String) -> String,
    ) -> Result<vellum_drive::SaveOutcome, Box<dyn std::error::Error + Send + Sync>> {
        let blob = self.store.get(&node.path).await?;
        let text = String::from_utf8(blob.bytes.to_vec())?;
        let base = Base {
            bytes: blob.bytes,
            etag: Some(blob.meta.etag),
        };
        Ok(self
            .drive
            .save(actor, node.id, base, Bytes::from(f(text)))
            .await?)
    }
}

async fn pages_live_in_real_folders(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    assert_eq!(eng.root.as_str(), "drives/shared/eng");
    let specs = w
        .drive
        .create_folder(&w.alice, eng.id, None, "Specs")
        .await?;
    let plan = w
        .drive
        .create_page(
            &w.alice,
            eng.id,
            Some(specs.id),
            "Roadmap 2027",
            "# Roadmap\n",
        )
        .await?;
    assert_eq!(
        plan.path.as_str(),
        "drives/shared/eng/Specs/Roadmap 2027.md"
    );
    let text = w.text(&plan).await?;
    assert!(text.starts_with(&format!(
        "---\nid: {}\ntitle: Roadmap 2027\nvellum: 1\n---\n",
        plan.id
    )));

    // A second page with the same title gets a suffixed file name.
    let twin = w
        .drive
        .create_page(&w.alice, eng.id, Some(specs.id), "Roadmap 2027", "")
        .await?;
    assert_eq!(twin.path.file_name(), "Roadmap 2027 (2).md");

    let listed = w.drive.list(&w.alice, eng.id, Some(specs.id)).await?;
    assert_eq!(listed.len(), 2);
    let root = w.drive.list(&w.alice, eng.id, None).await?;
    assert_eq!(
        root.iter().map(|n| n.kind).collect::<Vec<_>>(),
        vec![NodeKind::Folder]
    );

    // Deny by default.
    assert!(matches!(
        w.drive.list(&w.bob, eng.id, None).await,
        Err(DriveError::PermissionDenied)
    ));
    assert!(w.drive.shared_drives(w.bob_id).await?.is_empty());
    assert_eq!(w.drive.shared_drives(w.alice_id).await?.len(), 1);

    // Reading lands in recents; starring in starred.
    let _ = w.drive.read(&w.alice, plan.id).await?;
    w.clock.advance_days(1);
    let _ = w.drive.read(&w.alice, twin.id).await?;
    let recent = w.drive.recent(&w.alice, 10).await?;
    assert_eq!(recent.first().map(|(n, _)| n.id), Some(twin.id));
    w.drive.star(&w.alice, plan.id, true).await?;
    assert_eq!(
        w.drive.starred(&w.alice).await?.first().map(|n| n.id),
        Some(plan.id)
    );
    let filter = vellum_drive::Filter {
        starred: true,
        ..vellum_drive::Filter::default()
    };
    assert_eq!(w.drive.find(&w.alice, &filter, 10).await?.len(), 1);
    let filter = vellum_drive::Filter {
        title: Some("roadmap".into()),
        kinds: vec![NodeKind::Page],
        location: Some(specs.id),
        owner: Some(w.alice_id),
        ..vellum_drive::Filter::default()
    };
    assert_eq!(w.drive.find(&w.alice, &filter, 10).await?.len(), 2);
    assert!(w.drive.find(&w.bob, &filter, 10).await?.is_empty());
    w.drive.star(&w.alice, plan.id, false).await?;
    assert!(w.drive.starred(&w.alice).await?.is_empty());
    Ok(())
}

async fn my_drive_and_shared_with_me(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    assert!(matches!(
        w.drive
            .create_space(
                &w.bob,
                SpaceKind::UserDrive,
                "alice",
                "Alice",
                Some(w.alice_id)
            )
            .await,
        Err(DriveError::PermissionDenied)
    ));
    let mine = w
        .drive
        .create_space(
            &w.alice,
            SpaceKind::UserDrive,
            "alice",
            "Alice",
            Some(w.alice_id),
        )
        .await?;
    assert_eq!(
        w.drive.my_drive(w.alice_id).await?.map(|s| s.id),
        Some(mine.id)
    );
    let folder = w
        .drive
        .create_folder(&w.alice, mine.id, None, "Shared stuff")
        .await?;
    let inside = w
        .drive
        .create_page(&w.alice, mine.id, Some(folder.id), "Notes", "hi")
        .await?;
    let _private = w
        .drive
        .create_page(&w.alice, mine.id, None, "Diary", "secret")
        .await?;
    w.access.grant(w.bob_id, folder.id, Role::Viewer);
    let shared = w.drive.shared_with_me(w.bob_id).await?;
    assert_eq!(
        shared.iter().map(|n| n.id).collect::<Vec<_>>(),
        vec![folder.id]
    );
    // Inherited access reaches the page inside.
    assert!(w.drive.read(&w.bob, inside.id).await.is_ok());
    assert!(w.drive.shared_with_me(w.alice_id).await?.is_empty());
    Ok(())
}

async fn renaming_rewrites_links_and_backlinks_follow(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let specs = w
        .drive
        .create_folder(&w.alice, eng.id, None, "Specs")
        .await?;
    let target = w
        .drive
        .create_page(&w.alice, eng.id, Some(specs.id), "Plan", "# Plan\n")
        .await?;
    let body = format!(
        "See [[Plan]], [the plan](Specs/Plan.md#goals) and [by id](vellum://doc/{}).\n",
        target.id
    );
    let source = w
        .drive
        .create_page(&w.alice, eng.id, None, "Index", &body)
        .await?;
    let back = w.drive.backlinks(&w.alice, target.id).await?;
    assert_eq!(
        back.iter().map(|n| n.id).collect::<Vec<_>>(),
        vec![source.id]
    );

    let renamed = w.drive.rename(&w.alice, target.id, "Plan v2").await?;
    assert_eq!(renamed.path.as_str(), "drives/shared/eng/Specs/Plan v2.md");
    assert!(w.text(&renamed).await?.contains("title: Plan v2"));
    let source = w.node(source.id).await?;
    let text = w.text(&source).await?;
    assert!(text.contains("[[Plan v2]]"), "{text}");
    assert!(text.contains("](Specs/Plan%20v2.md#goals)"), "{text}");
    assert!(
        text.contains(&format!("vellum://doc/{}", target.id)),
        "{text}"
    );

    // Saving a new title in front matter renames the file too.
    let out = w
        .edit(&w.alice, &renamed, |t| {
            t.replace("title: Plan v2", "title: Plan v3")
        })
        .await?;
    assert_eq!(out.node.path.file_name(), "Plan v3.md");
    assert!(
        w.text(&w.node(source.id).await?)
            .await?
            .contains("[[Plan v3]]")
    );

    let feed = w.drive.activity(&w.alice, specs.id, 50).await?;
    assert!(
        feed.iter()
            .any(|a| matches!(&a.kind, ActivityKind::Renamed { to, .. } if to == "Plan v3"))
    );
    Ok(())
}

async fn moving_keeps_explicit_grants_and_fixes_relative_links(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let a = w.drive.create_folder(&w.alice, eng.id, None, "A").await?;
    let b = w.drive.create_folder(&w.alice, eng.id, None, "B").await?;
    let sibling = w
        .drive
        .create_page(&w.alice, eng.id, Some(a.id), "Sibling", "")
        .await?;
    let page = w
        .drive
        .create_page(&w.alice, eng.id, Some(a.id), "Moving", "[s](Sibling.md)\n")
        .await?;
    let linker = w
        .drive
        .create_page(&w.alice, eng.id, None, "Linker", "[m](A/Moving.md)\n")
        .await?;

    // Bob can see A (inherited by the page) and has an explicit grant on the page.
    w.access.grant(w.bob_id, a.id, Role::Viewer);
    w.access.grant(w.bob_id, page.id, Role::Editor);

    let moved = w
        .drive
        .move_node(&w.alice, page.id, eng.id, Some(b.id))
        .await?;
    assert_eq!(moved.path.as_str(), "drives/shared/eng/B/Moving.md");
    assert_eq!(moved.parent, Some(b.id));
    assert!(w.text(&moved).await?.contains("[s](../A/Sibling.md)"));
    assert!(
        w.text(&w.node(linker.id).await?)
            .await?
            .contains("[m](B/Moving.md)")
    );
    // Explicit grant kept, inherited view of A gone for A's other content.
    assert!(w.drive.read(&w.bob, moved.id).await.is_ok());
    assert!(w.drive.read(&w.bob, sibling.id).await.is_ok());
    w.access.revoke(w.bob_id, a.id);
    assert!(w.drive.read(&w.bob, moved.id).await.is_ok());
    assert!(w.drive.read(&w.bob, sibling.id).await.is_err());

    // A folder cannot move into itself.
    assert!(matches!(
        w.drive.move_node(&w.alice, a.id, eng.id, Some(a.id)).await,
        Err(DriveError::Invalid(_))
    ));
    // Moving a folder carries its contents.
    let a_moved = w
        .drive
        .move_node(&w.alice, a.id, eng.id, Some(b.id))
        .await?;
    assert_eq!(
        w.node(sibling.id).await?.path.as_str(),
        "drives/shared/eng/B/A/Sibling.md"
    );
    assert_eq!(a_moved.path.as_str(), "drives/shared/eng/B/A");
    Ok(())
}

async fn copies_get_fresh_ids_and_internal_links(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let folder = w.drive.create_folder(&w.alice, eng.id, None, "Kit").await?;
    let one = w
        .drive
        .create_page(&w.alice, eng.id, Some(folder.id), "One", "")
        .await?;
    let body = format!("[one](vellum://doc/{})\n", one.id);
    let two = w
        .drive
        .create_page(&w.alice, eng.id, Some(folder.id), "Two", &body)
        .await?;

    let copy = w.drive.copy_node(&w.alice, folder.id, eng.id, None).await?;
    assert_ne!(copy.id, folder.id);
    assert_eq!(copy.path.as_str(), "drives/shared/eng/Kit (2)");
    let children = w.drive.list(&w.alice, eng.id, Some(copy.id)).await?;
    assert_eq!(children.len(), 2);
    let ids: Vec<ObjectId> = children.iter().map(|n| n.id).collect();
    assert!(!ids.contains(&one.id) && !ids.contains(&two.id));
    let two_copy = children
        .iter()
        .find(|n| n.title == "Two")
        .ok_or("no copy of Two")?;
    let one_copy = children
        .iter()
        .find(|n| n.title == "One")
        .ok_or("no copy of One")?;
    let text = w.text(two_copy).await?;
    assert!(
        text.contains(&format!("vellum://doc/{}", one_copy.id)),
        "{text}"
    );
    assert!(text.contains(&format!("id: {}", two_copy.id)), "{text}");
    // The original is untouched.
    assert!(
        w.text(&two)
            .await?
            .contains(&format!("vellum://doc/{}", one.id))
    );
    let feed = w.drive.activity(&w.alice, copy.id, 10).await?;
    assert!(
        feed.iter()
            .any(|a| a.kind == ActivityKind::Copied { from: folder.id })
    );
    Ok(())
}

async fn trash_cascades_and_restores_exactly(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let folder = w.drive.create_folder(&w.alice, eng.id, None, "Old").await?;
    let early = w
        .drive
        .create_page(&w.alice, eng.id, Some(folder.id), "Early", "")
        .await?;
    let late = w
        .drive
        .create_page(&w.alice, eng.id, Some(folder.id), "Late", "")
        .await?;

    // Early is trashed on its own, then the whole folder.
    let _ = w.drive.trash(&w.alice, early.id).await?;
    let trashed = w.drive.trash(&w.alice, folder.id).await?;
    assert!(trashed.path.as_str().starts_with(".vellum/trash/"));
    let late_now = w.node(late.id).await?;
    let state = late_now.trash.as_ref().ok_or("late not trashed")?;
    assert!(!state.explicitly_trashed);
    assert_eq!(state.entry, folder.id);
    assert!(w.drive.list(&w.alice, eng.id, None).await?.is_empty());
    let entries = w.drive.trash_list(&w.alice, eng.id).await?;
    assert_eq!(entries.len(), 2);
    // A member that went with its folder is restored through the folder.
    assert!(matches!(
        w.drive.restore(&w.alice, late.id).await,
        Err(DriveError::Invalid(_))
    ));

    let back = w.drive.restore(&w.alice, folder.id).await?;
    assert_eq!(back.path.as_str(), "drives/shared/eng/Old");
    let inside = w.drive.list(&w.alice, eng.id, Some(folder.id)).await?;
    assert_eq!(
        inside.iter().map(|n| n.id).collect::<Vec<_>>(),
        vec![late.id]
    );
    assert!(w.node(early.id).await?.trash.is_some());

    // Restoring the page brings it back into its folder.
    let early_back = w.drive.restore(&w.alice, early.id).await?;
    assert_eq!(early_back.parent, Some(folder.id));

    // Purging is for managers; after 30 days the trash empties itself.
    let _ = w.drive.trash(&w.alice, late.id).await?;
    w.access.grant(w.bob_id, eng.id, Role::Editor);
    assert!(matches!(
        w.drive.purge(&w.bob, late.id).await,
        Err(DriveError::PermissionDenied)
    ));
    let model = Actor::Model {
        on_behalf_of: w.alice_id,
    };
    assert!(matches!(
        w.drive.purge(&model, late.id).await,
        Err(DriveError::PermissionDenied)
    ));
    w.clock.advance_days(29);
    assert_eq!(w.drive.purge_expired().await?, 0);
    w.clock.advance_days(1);
    assert_eq!(w.drive.purge_expired().await?, 1);
    assert!(w.index.node(late.id).await?.is_none());
    assert!(w.drive.trash_list(&w.alice, eng.id).await?.is_empty());
    Ok(())
}

async fn shortcuts_never_grant_access(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let secret = w
        .drive
        .create_page(&w.alice, eng.id, None, "Secret", "")
        .await?;
    let open = w
        .drive
        .create_folder(&w.alice, eng.id, None, "Open")
        .await?;
    let shortcut = w
        .drive
        .create_shortcut(&w.alice, secret.id, eng.id, Some(open.id))
        .await?;
    assert_eq!(
        shortcut.path.as_str(),
        "drives/shared/eng/Open/Secret.shortcut.yaml"
    );
    assert_eq!(
        w.drive.resolve_shortcut(&w.alice, shortcut.id).await?.id,
        secret.id
    );

    w.access.grant(w.bob_id, open.id, Role::Viewer);
    assert!(w.drive.get(&w.bob, shortcut.id).await.is_ok());
    assert!(matches!(
        w.drive.resolve_shortcut(&w.bob, shortcut.id).await,
        Err(DriveError::PermissionDenied)
    ));
    Ok(())
}

async fn quotas_and_version_retention(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let page = w
        .drive
        .create_page(&w.alice, eng.id, None, "Log", "v1\n")
        .await?;
    let mut node = page.clone();
    for n in 2..=5 {
        w.clock.advance_days(10);
        node = w
            .edit(&w.alice, &node, |t| format!("{t}v{n}\n"))
            .await?
            .node;
    }
    let versions = w.drive.versions(&w.alice, page.id).await?;
    assert_eq!(versions.len(), 5);

    // Restore the first version as a new one.
    let first = versions.last().ok_or("no versions")?.id;
    let restored = w.drive.restore_version(&w.alice, page.id, first).await?;
    assert!(!w.text(&restored.node).await?.contains("v2"));
    assert_eq!(w.drive.versions(&w.alice, page.id).await?.len(), 6);

    // Keep everything for 15 days, then nothing older but the newest.
    let mut settings = SpaceSettings::default();
    settings.retention.keep_all_days = 15;
    settings.retention.daily_days = 0;
    settings.retention.weekly_weeks = Some(0);
    let _ = w
        .drive
        .update_space(&w.alice, eng.id, None, settings.clone())
        .await?;
    // Ages 0, 0, 10, 20, 30 and 40 days: the three under 15 days stay.
    assert_eq!(w.drive.prune_versions(&w.alice, eng.id).await?, 3);
    assert_eq!(w.drive.versions(&w.alice, page.id).await?.len(), 3);

    // A tight quota refuses growth.
    let used = w.drive.usage(&w.alice, eng.id).await?.used;
    settings.quota_bytes = Some(used + 10);
    let _ = w
        .drive
        .update_space(&w.alice, eng.id, None, settings)
        .await?;
    let err = w
        .drive
        .create_page(&w.alice, eng.id, None, "Big", &"x".repeat(1000))
        .await;
    assert!(
        matches!(err, Err(DriveError::QuotaExceeded { .. })),
        "{err:?}"
    );
    Ok(())
}

async fn concurrent_edits_merge_or_make_a_conflict_copy(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let page = w
        .drive
        .create_page(&w.alice, eng.id, None, "Shared", "one\ntwo\nthree\n")
        .await?;
    let base_blob = w.store.get(&page.path).await?;
    let base = || Base {
        bytes: base_blob.bytes.clone(),
        etag: Some(base_blob.meta.etag.clone()),
    };
    let original = String::from_utf8(base_blob.bytes.to_vec())?;

    // Someone edits the file outside Vellum.
    let theirs = original.replace("one\n", "ONE\n");
    let _ = w
        .store
        .put(&page.path, Bytes::from(theirs), Precondition::None)
        .await?;
    // Our edit to another line merges.
    let ours = original.replace("three\n", "THREE\n");
    let out = w
        .drive
        .save(&w.alice, page.id, base(), Bytes::from(ours))
        .await?;
    assert!(matches!(out.saved, Saved::Merged(_)));
    let merged = w.text(&out.node).await?;
    assert!(merged.contains("ONE") && merged.contains("THREE"));

    // An edit to the same line conflicts and lands in a copy.
    let clash = original.replace("one\n", "uno\n");
    let out = w
        .drive
        .save(&w.alice, page.id, base(), Bytes::from(clash))
        .await?;
    let Saved::Conflicted { copy } = out.saved else {
        return Err(format!("expected a conflict, got {:?}", out.saved).into());
    };
    assert!(copy.path.file_name().contains("conflict"));
    assert_ne!(copy.id, page.id);
    assert!(w.text(&copy).await?.contains("uno"));
    assert!(w.text(&w.node(page.id).await?).await?.contains("ONE"));
    Ok(())
}

async fn the_index_rebuilds_from_files(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let folder = w
        .drive
        .create_folder(&w.alice, eng.id, None, "Specs")
        .await?;
    let page = w
        .drive
        .create_page(&w.alice, eng.id, Some(folder.id), "Plan", "x")
        .await?;
    let linker = w
        .drive
        .create_page(&w.alice, eng.id, None, "Index", "[[Plan]]")
        .await?;
    let gone = w
        .drive
        .create_page(&w.alice, eng.id, None, "Gone", "")
        .await?;
    let _ = w.drive.trash(&w.alice, gone.id).await?;

    // Renamed outside Vellum: identity follows the ID.
    let outside = BlobPath::new("drives/shared/eng/Specs/Renamed in Finder.md")?;
    let _ = w.store.rename(&page.path, &outside).await?;
    // Copied outside Vellum: the duplicate ID is replaced.
    let dup = BlobPath::new("drives/shared/eng/Index copy.md")?;
    let bytes = w.store.get(&linker.path).await?.bytes;
    let _ = w.store.put(&dup, bytes, Precondition::IfAbsent).await?;

    let report = w.drive.scan().await?;
    assert_eq!(report.spaces, 1);
    assert_eq!(report.relocated, 1);
    assert_eq!(report.reidentified, 1);
    assert_eq!(w.node(page.id).await?.path, outside);

    // A fresh index rebuilt from the same files sees the same drive.
    let index = w.databases.index().await?;
    let access = Arc::new(MemoryAccess::new(index.clone()));
    let rebuilt = Drive::new(w.store.clone(), index.clone(), access);
    let report = rebuilt.scan().await?;
    assert_eq!(report.removed, 0);
    let node = index.node(page.id).await?.ok_or("page lost")?;
    assert_eq!(node.parent, Some(folder.id));
    assert_eq!(index.backlinks(page.id).await?.len(), 2);
    let trashed = index.node(gone.id).await?.ok_or("trash lost")?;
    assert!(trashed.trash.as_ref().is_some_and(|t| t.explicitly_trashed));
    assert_eq!(index.versions(page.id).await?.len(), 1);
    let space = index.space(eng.id).await?.ok_or("space lost")?;
    assert_eq!(space.name, "Engineering");
    assert!(layout::is_state(&trashed.path));
    Ok(())
}

async fn external_edits_and_renames_come_in_through_the_watcher(kind: Kind) -> TestResult {
    let w = world(kind).await?;
    let eng = w
        .drive
        .create_space(&w.alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let folder = w
        .drive
        .create_folder(&w.alice, eng.id, None, "Specs")
        .await?;
    let edited = w
        .drive
        .create_page(&w.alice, eng.id, None, "Edited", "one\n")
        .await?;
    let moved = w
        .drive
        .create_page(&w.alice, eng.id, None, "Moved", "")
        .await?;

    let mut known = Vec::new();
    for node in w.index.nodes(eng.id).await? {
        let file = layout::id_file(node.kind, &node.path)?;
        known.push((node.id, file.clone(), w.store.head(&file).await?.etag));
    }
    let watched = LocalStore::open(w.dir.path()).await?;
    let mut reconciler = vellum_store::watch::Reconciler::new(watched, known);

    // An editor saves in place; Finder moves a file into a folder.
    w.clock.advance_days(1);
    let text = w.text(&edited).await?;
    let _ = w
        .store
        .put(
            &edited.path,
            Bytes::from(format!("{text}two\n")),
            Precondition::None,
        )
        .await?;
    let target = BlobPath::new("drives/shared/eng/Specs/Moved.md")?;
    let _ = w.store.rename(&moved.path, &target).await?;

    let changes = reconciler.scan().await?;
    let report = w.drive.apply_changes(&changes).await?;
    assert_eq!(report.relocated, 1, "{changes:?}");
    let now = w.node(moved.id).await?;
    assert_eq!((now.path, now.parent), (target, Some(folder.id)));
    let versions = w.drive.versions(&w.alice, edited.id).await?;
    assert_eq!(versions.len(), 2);
    assert_eq!(versions.first().map(|v| v.author), Some(Actor::External));

    // Re-applying the same edit (our own write echoing back) adds nothing.
    let again = vec![vellum_store::watch::Change::Modified {
        id: edited.id,
        path: edited.path.clone(),
        etag: w.store.head(&edited.path).await?.etag,
    }];
    let _ = w.drive.apply_changes(&again).await?;
    assert_eq!(w.drive.versions(&w.alice, edited.id).await?.len(), 2);
    Ok(())
}

/// Runs each test on every index.
macro_rules! on_every_index {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            use super::{Kind, TestResult};

            #[tokio::test]
            async fn memory() -> TestResult {
                super::$name(Kind::Memory).await
            }

            #[tokio::test]
            async fn sqlite() -> TestResult {
                super::$name(Kind::Sqlite).await
            }

            #[tokio::test]
            async fn postgres() -> TestResult {
                super::$name(Kind::Postgres).await
            }
        }
    )*};
}

on_every_index!(
    pages_live_in_real_folders,
    my_drive_and_shared_with_me,
    renaming_rewrites_links_and_backlinks_follow,
    moving_keeps_explicit_grants_and_fixes_relative_links,
    copies_get_fresh_ids_and_internal_links,
    trash_cascades_and_restores_exactly,
    shortcuts_never_grant_access,
    quotas_and_version_retention,
    concurrent_edits_merge_or_make_a_conflict_copy,
    the_index_rebuilds_from_files,
    external_edits_and_renames_come_in_through_the_watcher,
);
