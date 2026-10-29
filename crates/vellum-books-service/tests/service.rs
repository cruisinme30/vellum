// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The books service end to end: a real book folder on disk, the real
//! authorization engine (in memory) and the operations as the gateway
//! calls them.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]
#![allow(clippy::too_many_lines, reason = "the fixture spells out a whole book")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use vellum_authz::{
    Actor, Authz, Change, Id, MemoryStore, Object, Permission, Relation, Subject, Tuple,
};
use vellum_books_service::api::{
    AddSection, At, BookKey, CopySection, CreateSnapshot, DiffInput, HistoryEntry, HistoryInput,
    ListBooks, MergeSection, MoveSection, PlaceInput, Publish, PublishFormat, RestoreOutline,
    SectionKey, SetSectionAccess, SplitSection, TocLine, TransferSection, UpdateSection,
};
use vellum_books_service::{AuthzAccess, BooksService};
use vellum_contracts::{CredentialKind, Principal, RequestContext, Scope, ScopeSet, Service as _};
use vellum_kernel::{BookId, ErrorKind, ObjectId, RequestId, Timestamp, UserId};
use vellum_store::{BlobPath, LocalStore};

const DIR: &str = "drives/shared/Northstar/Handbook.book";
const GUIDE: &str = "drives/shared/Northstar/Guide.book";

struct World {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    authz: Arc<Authz<MemoryStore>>,
    service: Arc<BooksService>,
    space: Id,
    book: BookId,
    intro: String,
    ops: String,
    runbook: String,
    glossary: String,
    sre: Id,
    ana: UserId,
    raj: UserId,
    mia: UserId,
    guide: BookId,
}

fn ctx(user: UserId) -> RequestContext {
    RequestContext {
        request_id: RequestId::generate(),
        principal: Principal::user(
            user,
            CredentialKind::Session,
            ScopeSet::from_scopes([Scope::WriteDirect]),
            Timestamp::now(),
        ),
        deadline: Instant::now() + Duration::from_secs(5),
        traceparent: None,
    }
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn chapter(id: &str, title: &str, body: &str) -> String {
    format!("---\nid: {id}\ntitle: {title}\nvellum: 1\n---\n\n{body}\n")
}

fn uid(id: &str) -> Id {
    Id(id.parse::<ObjectId>().unwrap().as_u128())
}

async fn world() -> World {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let fresh = || ObjectId::generate().to_string();
    let (book, intro, ops, runbook, glossary) = (fresh(), fresh(), fresh(), fresh(), fresh());
    let (guide, start) = (fresh(), fresh());
    write(
        &root,
        &format!("{GUIDE}/book.yaml"),
        &format!("vellum: 1\nid: {guide}\ntitle: Guide\nnodes:\n  {start}:\n    path: start.md\n"),
    );
    write(
        &root,
        &format!("{GUIDE}/SUMMARY.md"),
        "# Summary\n\n- [Start](start.md)\n",
    );
    write(
        &root,
        &format!("{GUIDE}/start.md"),
        &chapter(&start, "Start", "Begin here."),
    );
    let sre = ObjectId::generate();
    write(
        &root,
        &format!("{DIR}/book.yaml"),
        &format!(
            "vellum: 1\nid: {book}\ntitle: Northstar handbook\nnodes:\n  \
             {intro}:\n    path: README.md\n  \
             {ops}:\n    path: operations/README.md\n    restrict_to: [\"group:{sre}\"]\n  \
             {runbook}:\n    path: operations/runbook.md\n  \
             {glossary}:\n    path: glossary.md\n"
        ),
    );
    write(
        &root,
        &format!("{DIR}/SUMMARY.md"),
        "# Summary\n\n[Introduction](README.md)\n\n- [Operations](operations/README.md)\n  \
         - [Runbook](operations/runbook.md)\n- [Glossary](glossary.md)\n",
    );
    write(
        &root,
        &format!("{DIR}/README.md"),
        &chapter(
            &intro,
            "Introduction",
            &format!("See [§](vellum://doc/{glossary})."),
        ),
    );
    write(
        &root,
        &format!("{DIR}/operations/README.md"),
        &chapter(&ops, "Operations", "Pager rotation."),
    );
    write(
        &root,
        &format!("{DIR}/operations/runbook.md"),
        &chapter(&runbook, "Runbook", "Restart the queue."),
    );
    write(
        &root,
        &format!("{DIR}/glossary.md"),
        &chapter(&glossary, "Glossary", "Terms."),
    );

    // The engine: the book node with its sections under it, as the index
    // registers them. The runbook is not indexed yet.
    let space = Id(1);
    let store = MemoryStore::new();
    store
        .set_tree(
            space,
            BTreeMap::from([
                (uid(&book), None),
                (uid(&intro), Some(uid(&book))),
                (uid(&ops), Some(uid(&book))),
                (uid(&glossary), Some(uid(&book))),
                (uid(&guide), None),
                (uid(&start), Some(uid(&guide))),
            ]),
        )
        .unwrap();
    let authz = Arc::new(Authz::new(store));
    let (ana, raj, mia) = (UserId::generate(), UserId::generate(), UserId::generate());
    let user = |u: UserId| Subject::User(Id(u.as_u128()));
    let node = Object::Node(uid(&book));
    let _ = authz
        .write(
            space,
            Actor::System,
            &[
                Change::Grant(Tuple::role(node, Relation::Viewer, user(ana))),
                Change::Grant(Tuple::role(node, Relation::Editor, user(raj))),
                Change::Grant(Tuple::role(node, Relation::Manager, user(mia))),
                Change::Grant(Tuple::role(
                    Object::Node(uid(&guide)),
                    Relation::Editor,
                    user(raj),
                )),
            ],
        )
        .await
        .unwrap();
    authz
        .update_group(Id(sre.as_u128()), &[user(raj)], &[])
        .await
        .unwrap();

    let blobs = Arc::new(LocalStore::open(&root).await.unwrap());
    let access = Arc::new(AuthzAccess::new(
        Arc::clone(&authz),
        Arc::new(move |p: &BlobPath| p.as_str().starts_with("drives/shared/").then_some(space)),
    ));
    let service = Arc::new(BooksService::new(blobs, access));
    World {
        _tmp: tmp,
        root,
        authz,
        service,
        space,
        book: book.parse().unwrap(),
        intro,
        ops,
        runbook,
        glossary,
        sre: Id(sre.as_u128()),
        ana,
        raj,
        mia,
        guide: guide.parse().unwrap(),
    }
}

impl World {
    fn key(&self, section: &str) -> SectionKey {
        SectionKey {
            book_id: self.book,
            section_id: section.to_owned(),
        }
    }

    async fn sees(&self, user: UserId, section: &str) -> bool {
        self.authz
            .check(
                self.space,
                vellum_authz::Principal::User(Id(user.as_u128())),
                uid(section),
                Permission::Read,
            )
            .await
            .unwrap()
            .allowed
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.root.join(DIR).join(rel)).unwrap()
    }
}

fn titles(toc: &[TocLine]) -> Vec<String> {
    fn walk(node: &vellum_books_service::api::TocNode, out: &mut Vec<String>) {
        out.push(node.title.clone());
        for child in &node.children {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    for line in toc {
        if let TocLine::Chapter(node) = line {
            walk(node, &mut out);
        }
    }
    out
}

#[tokio::test]
async fn readers_see_only_the_sections_they_may() {
    let w = world().await;
    let ana = w.service.get(&ctx(w.ana), w.book).await.unwrap();
    assert_eq!(ana.role, "viewer");
    assert_eq!(titles(&ana.toc), ["Introduction", "Glossary"]);
    // Hidden and unknown sections look the same.
    for section in [&w.ops, &w.runbook] {
        let err = w.service.get_section(&ctx(w.ana), w.key(section)).await;
        assert_eq!(err.unwrap_err().kind(), ErrorKind::NotFound);
    }

    let raj = w.service.get(&ctx(w.raj), w.book).await.unwrap();
    assert_eq!(
        titles(&raj.toc),
        ["Introduction", "Operations", "Runbook", "Glossary"]
    );
    let doc = w
        .service
        .get_section(&ctx(w.raj), w.key(&w.runbook))
        .await
        .unwrap();
    assert!(doc.restricted);
    assert_eq!(doc.label, "§1.1 Runbook");
    assert!(doc.access.is_none());

    // Managers see everything, with the access settings.
    let doc = w
        .service
        .get_section(&ctx(w.mia), w.key(&w.ops))
        .await
        .unwrap();
    let access = doc.access.unwrap();
    assert_eq!(
        access.restrict_to.unwrap(),
        [format!("group:{}", ObjectId::from_u128(w.sre.0))]
    );

    // Cross-references are filled in.
    let intro = w
        .service
        .get_section(&ctx(w.ana), w.key(&w.intro))
        .await
        .unwrap();
    assert_eq!(
        intro.markdown.trim(),
        "See [§2 Glossary](vellum://doc/".to_owned() + &w.glossary + ")."
    );

    // Someone with no access to the book cannot find it.
    let stranger = UserId::generate();
    assert_eq!(
        w.service
            .get(&ctx(stranger), w.book)
            .await
            .unwrap_err()
            .kind(),
        ErrorKind::NotFound
    );
    let list = w.service.list(&ctx(stranger)).await.unwrap();
    assert!(list.books.is_empty());
    let list = w.service.list(&ctx(w.ana)).await.unwrap();
    assert_eq!(list.books.len(), 1);
    assert_eq!(list.books[0].path, DIR);
}

#[tokio::test]
async fn restrictions_are_mirrored_into_the_engine() {
    let w = world().await;
    // Before any edit the engine knows nothing of the restriction.
    assert!(w.sees(w.ana, &w.ops).await);
    w.service.sync_access(w.book).await.unwrap();
    assert!(!w.sees(w.ana, &w.ops).await);
    assert!(w.sees(w.raj, &w.ops).await);
    assert!(w.sees(w.mia, &w.ops).await);
    assert!(w.sees(w.ana, &w.glossary).await);

    // A manager restricts the glossary to Ana alone.
    let ana = format!("user:{}", w.ana);
    let info = w
        .service
        .set_section_access(
            &ctx(w.mia),
            SetSectionAccess {
                book_id: w.book,
                section_id: w.glossary.clone(),
                restrict_to: vec![ana.clone()],
                owners: vec![format!("user:{}", w.mia)],
                reviewers: vec![],
            },
        )
        .await
        .unwrap();
    assert_eq!(info.restrict_to.unwrap(), std::slice::from_ref(&ana));
    assert!(w.read("book.yaml").contains(&ana));
    assert!(w.sees(w.ana, &w.glossary).await);
    assert!(!w.sees(w.raj, &w.glossary).await);
    assert!(w.sees(w.mia, &w.glossary).await);
    // Ana is still only a viewer there: the grant is capped by the book.
    let decision = w
        .authz
        .check(
            w.space,
            vellum_authz::Principal::User(Id(w.ana.as_u128())),
            uid(&w.glossary),
            Permission::Edit,
        )
        .await
        .unwrap();
    assert!(!decision.allowed);

    // Lifting it restores inherited access.
    let _ = w
        .service
        .set_section_access(
            &ctx(w.mia),
            SetSectionAccess {
                book_id: w.book,
                section_id: w.glossary.clone(),
                restrict_to: vec![],
                owners: vec![],
                reviewers: vec![],
            },
        )
        .await
        .unwrap();
    assert!(w.sees(w.raj, &w.glossary).await);

    // Only managers may change access; editors may not, readers may not.
    let attempt = SetSectionAccess {
        book_id: w.book,
        section_id: w.intro.clone(),
        restrict_to: vec![ana],
        owners: vec![],
        reviewers: vec![],
    };
    let err = w
        .service
        .set_section_access(&ctx(w.raj), attempt.clone())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    // Widening an enclosing restriction is refused.
    let widen = SetSectionAccess {
        section_id: w.runbook.clone(),
        restrict_to: vec![format!("user:{}", w.ana)],
        ..attempt
    };
    let err = w
        .service
        .set_section_access(&ctx(w.mia), widen)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[tokio::test]
async fn editors_change_the_structure_on_disk() {
    let w = world().await;
    let place = |at, section: &str| PlaceInput {
        at,
        section: Some(section.to_owned()),
        region: None,
    };
    // Readers cannot edit.
    let add = AddSection {
        book_id: w.book,
        title: "Escalation".into(),
        body: "Call the lead.".into(),
        place: place(At::LastChildOf, &w.ops),
    };
    let err = w
        .service
        .add_section(&ctx(w.ana), add.clone())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);

    // Raj adds a section under Operations; it inherits the restriction.
    let doc = w.service.add_section(&ctx(w.raj), add).await.unwrap();
    assert_eq!(doc.label, "§1.2 Escalation");
    assert!(doc.restricted);
    assert_eq!(doc.path, "operations/escalation.md");
    assert!(
        w.read("SUMMARY.md")
            .contains("[Escalation](operations/escalation.md)")
    );
    assert!(
        w.read("operations/escalation.md")
            .contains("Call the lead.")
    );
    let err = w
        .service
        .get_section(&ctx(w.ana), w.key(&doc.id))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);

    // Ana cannot place things next to a section she cannot see.
    let err = w
        .service
        .move_section(
            &ctx(w.ana),
            MoveSection {
                book_id: w.book,
                section_id: w.glossary.clone(),
                place: place(At::After, &w.ops),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);

    // Rename and body edits keep the front matter and the ID.
    let doc = w
        .service
        .update_section(
            &ctx(w.raj),
            UpdateSection {
                book_id: w.book,
                section_id: w.glossary.clone(),
                title: Some("Terms".into()),
                body: Some("Words we use.".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(doc.path, "terms.md");
    let text = w.read("terms.md");
    assert!(text.contains(&format!("id: {}", w.glossary)));
    assert!(text.ends_with("Words we use.\n"));
    assert!(!w.root.join(DIR).join("glossary.md").exists());
    // The link from the introduction follows by ID; its label updates.
    let intro = w
        .service
        .get_section(&ctx(w.ana), w.key(&w.intro))
        .await
        .unwrap();
    assert!(intro.markdown.contains("[§2 Terms]"), "{}", intro.markdown);

    // Moving Operations after Terms renumbers it.
    let doc = w
        .service
        .move_section(
            &ctx(w.raj),
            MoveSection {
                book_id: w.book,
                section_id: w.ops.clone(),
                place: place(At::After, &w.glossary),
            },
        )
        .await
        .unwrap();
    assert_eq!(doc.label, "§2 Operations");

    // An outside edit on disk is seen at once.
    let mut runbook = w.read("operations/runbook.md");
    runbook.push_str("\nCheck the dashboards.\n");
    write(&w.root, &format!("{DIR}/operations/runbook.md"), &runbook);
    let doc = w
        .service
        .get_section(&ctx(w.raj), w.key(&w.runbook))
        .await
        .unwrap();
    assert!(doc.markdown.contains("Check the dashboards."));
}

#[tokio::test]
async fn removal_needs_sight_of_the_whole_subtree() {
    let w = world().await;
    // Narrow the runbook to Mia only; Raj can still see Operations.
    let _ = w
        .service
        .set_section_access(
            &ctx(w.mia),
            SetSectionAccess {
                book_id: w.book,
                section_id: w.runbook.clone(),
                restrict_to: vec![format!("group:{}", ObjectId::from_u128(w.sre.0))],
                owners: vec![],
                reviewers: vec![],
            },
        )
        .await
        .unwrap();
    let _ = w
        .service
        .set_section_access(
            &ctx(w.mia),
            SetSectionAccess {
                book_id: w.book,
                section_id: w.ops.clone(),
                restrict_to: vec![
                    format!("group:{}", ObjectId::from_u128(w.sre.0)),
                    format!("user:{}", w.ana),
                ],
                owners: vec![],
                reviewers: vec![],
            },
        )
        .await
        .unwrap();
    // Ana is promoted to editor; she sees Operations but not the runbook.
    let _ = w
        .authz
        .write(
            w.space,
            Actor::System,
            &[Change::Grant(Tuple::role(
                Object::Node(w.book_node()),
                Relation::Editor,
                Subject::User(Id(w.ana.as_u128())),
            ))],
        )
        .await
        .unwrap();
    let err = w
        .service
        .remove_section(&ctx(w.ana), w.key(&w.ops))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    let removed = w
        .service
        .remove_section(&ctx(w.raj), w.key(&w.ops))
        .await
        .unwrap();
    assert_eq!(removed.removed, [w.ops.clone(), w.runbook.clone()]);
    assert!(!w.root.join(DIR).join("operations/runbook.md").exists());
    assert!(!w.read("SUMMARY.md").contains("Operations"));
}

#[tokio::test]
async fn snapshots_and_diffs_respect_restrictions() {
    let w = world().await;
    let v1 = w
        .service
        .create_snapshot(
            &ctx(w.raj),
            CreateSnapshot {
                book_id: w.book,
                tag: "v1.0".into(),
                message: "First release".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(v1.created_by, format!("user:{}", w.raj));
    assert!(w.root.join(DIR).join(".snapshots/v1.0.json").exists());
    let again = w
        .service
        .create_snapshot(
            &ctx(w.raj),
            CreateSnapshot {
                book_id: w.book,
                tag: "v1.0".into(),
                message: String::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(again.kind(), ErrorKind::Conflict);
    let err = w
        .service
        .create_snapshot(
            &ctx(w.ana),
            CreateSnapshot {
                book_id: w.book,
                tag: "v2".into(),
                message: String::new(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);

    for (section, body) in [(&w.runbook, "Restart twice."), (&w.glossary, "More terms.")] {
        let _ = w
            .service
            .update_section(
                &ctx(w.raj),
                UpdateSection {
                    book_id: w.book,
                    section_id: section.clone(),
                    title: None,
                    body: Some(body.into()),
                },
            )
            .await
            .unwrap();
    }
    let input = DiffInput {
        book_id: w.book,
        from: "v1.0".into(),
        to: None,
    };
    let raj = w.service.diff(&ctx(w.raj), input.clone()).await.unwrap();
    assert_eq!(raj.changed.len(), 2);
    let ana = w.service.diff(&ctx(w.ana), input.clone()).await.unwrap();
    assert_eq!(ana.changed.len(), 1);
    assert_eq!(ana.changed[0].id, w.glossary);
    assert!(ana.changed.iter().all(|c| !c.patch.contains("Restart")));
    assert!(ana.summary_patch.is_empty());
    let mia = w.service.diff(&ctx(w.mia), input).await.unwrap();
    assert_eq!(mia.changed.len(), 2);

    let list = w
        .service
        .list_snapshots(&ctx(w.ana), BookKey { book_id: w.book })
        .await
        .unwrap();
    assert_eq!(list.snapshots.len(), 1);
    assert_eq!(list.snapshots[0].message, "First release");
}

#[tokio::test]
async fn registers_every_operation() {
    let w = world().await;
    assert_eq!(w.service.name(), "books");
    let ops = Arc::clone(&w.service).operations();
    assert_eq!(ops.len(), 19);
    let _ = ListBooks::default();
}

impl World {
    const fn book_node(&self) -> Id {
        Id(self.book.as_u128())
    }
}

fn place(at: At, section: &str) -> PlaceInput {
    PlaceInput {
        at,
        section: Some(section.to_owned()),
        region: None,
    }
}

#[tokio::test]
async fn reorders_are_versioned_and_can_be_restored() {
    let w = world().await;
    let raj = ctx(w.raj);
    let history = async |user: UserId| {
        w.service
            .history(
                &ctx(user),
                HistoryInput {
                    book_id: w.book,
                    limit: None,
                },
            )
            .await
    };
    assert!(history(w.raj).await.unwrap().entries.is_empty());

    // The glossary moves before Operations, then back to the end.
    let move_glossary = |at: At, section: &str| MoveSection {
        book_id: w.book,
        section_id: w.glossary.clone(),
        place: place(at, section),
    };
    let _ = w
        .service
        .move_section(&raj, move_glossary(At::Before, &w.ops))
        .await
        .unwrap();
    let _ = w
        .service
        .move_section(&raj, move_glossary(At::After, &w.ops))
        .await
        .unwrap();
    // A body edit changes no outline, so it leaves no record.
    let _ = w
        .service
        .update_section(
            &raj,
            UpdateSection {
                book_id: w.book,
                section_id: w.intro.clone(),
                title: None,
                body: Some("Hello.".to_owned()),
            },
        )
        .await
        .unwrap();
    let entries = history(w.raj).await.unwrap().entries;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].op, "move_section");
    assert_eq!(entries[0].by, format!("user:{}", w.raj));
    let order = |e: &HistoryEntry| {
        e.outline
            .iter()
            .map(|l| l.title.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        order(&entries[1]),
        ["Introduction", "Glossary", "Operations", "Runbook"]
    );
    assert_eq!(
        entries[1].outline[3].parent.as_deref(),
        Some(w.ops.as_str())
    );

    // Readers never see restricted sections in old outlines either.
    let ana = history(w.ana).await.unwrap().entries;
    assert_eq!(order(&ana[1]), ["Introduction", "Glossary"]);

    // Restoring the older outline puts the glossary back before Operations.
    let info = w
        .service
        .restore_outline(
            &raj,
            RestoreOutline {
                book_id: w.book,
                entry: entries[1].id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        titles(&info.toc),
        ["Introduction", "Glossary", "Operations", "Runbook"]
    );
    assert_eq!(
        history(w.raj).await.unwrap().entries[0].op,
        "restore_outline"
    );

    // Viewers cannot restore, and unknown entries are not found.
    let denied = w
        .service
        .restore_outline(
            &ctx(w.ana),
            RestoreOutline {
                book_id: w.book,
                entry: entries[0].id.clone(),
            },
        )
        .await;
    assert_eq!(denied.unwrap_err().kind(), ErrorKind::PermissionDenied);
    let missing = w
        .service
        .restore_outline(
            &raj,
            RestoreOutline {
                book_id: w.book,
                entry: "../../book".to_owned(),
            },
        )
        .await;
    assert_eq!(missing.unwrap_err().kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn headings_become_sections_and_back() {
    let w = world().await;
    let raj = ctx(w.raj);
    let _ = w
        .service
        .update_section(
            &raj,
            UpdateSection {
                book_id: w.book,
                section_id: w.glossary.clone(),
                title: None,
                body: Some("Terms.\n\n## Alpha\n\nA.\n\n## Beta\n\nB.\n".to_owned()),
            },
        )
        .await
        .unwrap();
    let split = w
        .service
        .split_section(
            &raj,
            SplitSection {
                book_id: w.book,
                section_id: w.glossary.clone(),
                level: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(split.created.len(), 2);
    let info = w.service.get(&raj, w.book).await.unwrap();
    assert_eq!(
        titles(&info.toc),
        [
            "Introduction",
            "Operations",
            "Runbook",
            "Glossary",
            "Alpha",
            "Beta"
        ]
    );

    let merged = w
        .service
        .merge_section(
            &raj,
            MergeSection {
                book_id: w.book,
                section_id: w.glossary.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(merged.removed.len(), 2);
    let glossary = w
        .service
        .get_section(&raj, w.key(&w.glossary))
        .await
        .unwrap();
    assert!(glossary.markdown.contains("## Alpha"));

    // Viewers cannot fold sections.
    let denied = w
        .service
        .merge_section(
            &ctx(w.ana),
            MergeSection {
                book_id: w.book,
                section_id: w.glossary.clone(),
            },
        )
        .await;
    assert_eq!(denied.unwrap_err().kind(), ErrorKind::PermissionDenied);
}

#[tokio::test]
async fn sections_copy_within_and_move_between_books() {
    let w = world().await;
    let raj = ctx(w.raj);
    let copy = w
        .service
        .copy_section(
            &raj,
            CopySection {
                book_id: w.book,
                section_id: w.ops.clone(),
                place: place(At::After, &w.glossary),
            },
        )
        .await
        .unwrap();
    assert_ne!(copy.section_id, w.ops);
    assert_eq!(copy.created.len(), 2);

    // Moving Operations (with its runbook) into the guide.
    let moved = w
        .service
        .transfer_section(
            &raj,
            TransferSection {
                book_id: w.book,
                section_id: w.ops.clone(),
                to_book: w.guide,
                place: PlaceInput {
                    at: At::End,
                    section: None,
                    region: None,
                },
                copy: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(moved.section_id, w.ops);
    let guide = w.service.get(&raj, w.guide).await.unwrap();
    assert_eq!(titles(&guide.toc), ["Start", "Operations", "Runbook"]);
    let handbook = w.service.get(&raj, w.book).await.unwrap();
    assert!(!titles(&handbook.toc)[..2].contains(&"Operations".to_owned()));
    assert!(
        std::fs::read_to_string(w.root.join(GUIDE).join("operations/README.md"))
            .unwrap()
            .contains("Pager rotation")
    );

    // The same book is refused, and so is a viewer of the source.
    let same = w
        .service
        .transfer_section(
            &raj,
            TransferSection {
                book_id: w.book,
                section_id: w.intro.clone(),
                to_book: w.book,
                place: place(At::After, &w.glossary),
                copy: true,
            },
        )
        .await;
    assert_eq!(same.unwrap_err().kind(), ErrorKind::InvalidInput);
    let viewer = w
        .service
        .transfer_section(
            &ctx(w.ana),
            TransferSection {
                book_id: w.book,
                section_id: w.intro.clone(),
                to_book: w.guide,
                place: PlaceInput {
                    at: At::End,
                    section: None,
                    region: None,
                },
                copy: true,
            },
        )
        .await;
    assert_eq!(viewer.unwrap_err().kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn managers_publish_and_only_public_sites_are_served() {
    let w = world().await;
    let publish = async |user: UserId, format: PublishFormat, audience: Vec<String>| {
        w.service
            .publish(
                &ctx(user),
                Publish {
                    book_id: w.book,
                    format,
                    audience,
                },
            )
            .await
    };
    let denied = publish(w.raj, PublishFormat::Site, vec![]).await;
    assert_eq!(denied.unwrap_err().kind(), ErrorKind::PermissionDenied);

    let site = publish(w.mia, PublishFormat::Site, vec![]).await.unwrap();
    assert!(site.public);
    assert_eq!(
        site.url.as_deref(),
        Some(format!("/published/books/{}/", w.book).as_str())
    );
    let index = w.service.public_file(w.book, "").await.unwrap().unwrap();
    let index = String::from_utf8_lossy(&index);
    assert!(index.contains("Glossary"));
    assert!(
        !index.contains("Operations"),
        "restricted sections stay out"
    );
    assert!(!w.read(".published/site/style.css").is_empty());
    for bad in ["../book.yaml", "../../Handbook.book/book.yaml", "nope.html"] {
        assert!(w.service.public_file(w.book, bad).await.unwrap().is_none());
    }

    // The mdBook tree for the SRE group keeps their section.
    let sre = format!("group:{}", ObjectId::from_u128(w.sre.0));
    let tree = publish(w.mia, PublishFormat::Mdbook, vec![sre.clone()])
        .await
        .unwrap();
    assert!(!tree.public && tree.url.is_none());
    assert!(
        w.read(".published/mdbook/src/SUMMARY.md")
            .contains("Operations")
    );
    let listed = w
        .service
        .publications(&ctx(w.ana), BookKey { book_id: w.book })
        .await
        .unwrap();
    assert_eq!(listed.publications.len(), 2);

    // A site for members only is not served publicly.
    let _ = publish(w.mia, PublishFormat::Site, vec![sre])
        .await
        .unwrap();
    assert!(w.service.public_file(w.book, "").await.unwrap().is_none());
    // Publications never show up as book sections.
    let info = w.service.get(&ctx(w.mia), w.book).await.unwrap();
    assert_eq!(titles(&info.toc).len(), 4);
}
