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
    AddSection, At, BookKey, CreateSnapshot, DiffInput, ListBooks, MoveSection, PlaceInput,
    SectionKey, SetSectionAccess, TocLine, UpdateSection,
};
use vellum_books_service::{AuthzAccess, BooksService};
use vellum_contracts::{CredentialKind, Principal, RequestContext, Scope, ScopeSet, Service as _};
use vellum_kernel::{BookId, ErrorKind, ObjectId, RequestId, Timestamp, UserId};
use vellum_store::{BlobPath, LocalStore};

const DIR: &str = "drives/shared/Northstar/Handbook.book";

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
    assert_eq!(ops.len(), 11);
    let _ = ListBooks::default();
}

impl World {
    const fn book_node(&self) -> Id {
        Id(self.book.as_u128())
    }
}
