// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The collections service over a real store on disk: Northstar's
//! requirements linked two-way to releases, with per-collection access.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use serde_json::json;
use vellum_collections::service::{
    Access, CollectionsService, CreateRowInput, Need, QueryInput, RowKey, SaveViewInput,
    UpdateRowInput, ViewKey,
};
use vellum_collections::{DateValue, Value, ViewKind};
use vellum_contracts::{
    CredentialKind, Principal, Readiness, RequestContext, Scope, ScopeSet, Service,
};
use vellum_kernel::{CollectionId, DocId, Error, ErrorKind, RequestId, Timestamp, UserId};
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition};

const REQS: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C30";
const RELS: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C31";
const SECRET: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C32";
const R1: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C41";
const R2: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C42";
const V10: &str = "01JBQ8Y4ZK6V3W2N5R7T9X1C51";

const REQS_DIR: &str = "drives/shared/Northstar/Requirements.collection";
const RELS_DIR: &str = "drives/shared/Northstar/Releases.collection";
const SECRET_DIR: &str = "drives/shared/Board/Salaries.collection";

/// Grants by (user, collection).
#[derive(Default)]
struct Grants(Mutex<BTreeMap<(UserId, CollectionId), Need>>);

#[cfg(test)]
impl Grants {
    fn grant(&self, user: UserId, collection: &str, need: Need) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .insert((user, collection.parse().unwrap()), need);
    }
}

#[async_trait]
impl Access for Grants {
    async fn allowed(
        &self,
        principal: &Principal,
        collection: CollectionId,
        _path: &BlobPath,
        need: Need,
    ) -> Result<bool, Error> {
        let Some(user) = principal.user_id() else {
            return Ok(false);
        };
        let held = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&(user, collection))
            .copied();
        Ok(matches!(
            (held, need),
            (Some(Need::Edit), _) | (Some(Need::Read), Need::Read)
        ))
    }
}

#[cfg(test)]
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

#[cfg(test)]
fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[cfg(test)]
fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

#[cfg(test)]
fn seed(root: &Path) {
    write(
        root,
        &format!("{REQS_DIR}/schema.yaml"),
        &format!(
            "id: {REQS}\ntitle: Requirements\nproperties:\n  - {{key: t, name: Requirement, type: title}}\n  - {{key: est, name: Estimate, type: number}}\n  - {{key: dn, name: Done, type: checkbox}}\n  - {{key: rel, name: Release, type: relation, to: {RELS}, back: rq}}\n  - {{key: left, name: Remaining, type: formula, f: 'if(Done, 0, Estimate)'}}\nviews:\n  - {{name: All, type: table, sort: est desc}}\n  - {{name: Open, type: list, filter: Done is false}}\n"
        ),
    );
    write(
        root,
        &format!("{RELS_DIR}/schema.yaml"),
        &format!(
            "id: {RELS}\ntitle: Releases\nproperties:\n  - {{key: t, name: Release, type: title}}\n  - {{key: rq, name: Requirements, type: relation, to: {REQS}, back: rel}}\n  - {{key: pts, name: Points, type: rollup, via: rq, prop: est, agg: sum}}\n"
        ),
    );
    write(
        root,
        &format!("{REQS_DIR}/rows/replay.md"),
        &format!(
            "---\nid: {R1}\nt: Replay 7 days\nest: 8\ndn: true\nrel: [{V10}]\n---\nReplay notes.\n"
        ),
    );
    write(
        root,
        &format!("{REQS_DIR}/rows/webhooks.md"),
        &format!("---\nid: {R2}\nt: Webhooks\nest: 5\n---\n"),
    );
    write(
        root,
        &format!("{RELS_DIR}/rows/v1-0.md"),
        &format!("---\nid: {V10}\nt: v1.0\nrq: [{R1}]\n---\n"),
    );
    write(
        root,
        &format!("{SECRET_DIR}/schema.yaml"),
        &format!(
            "id: {SECRET}\ntitle: Salaries\nproperties:\n  - {{key: t, name: Name, type: title}}\n"
        ),
    );
    // Not collections: ignored.
    write(root, "drives/shared/Northstar/Plan.md", "# Plan\n");
    write(
        root,
        "drives/shared/Northstar/Broken.collection/schema.yaml",
        "id: nope\n",
    );
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    store: Arc<LocalStore>,
    grants: Arc<Grants>,
    service: Arc<CollectionsService>,
}

#[cfg(test)]
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    seed(&root);
    let store = Arc::new(LocalStore::open(&root).await.unwrap());
    let grants = Arc::new(Grants::default());
    let service = Arc::new(
        CollectionsService::new(store.clone(), grants.clone())
            .with_clock(Arc::new(|| DateValue::ymd(2026, 10, 29).unwrap())),
    );
    Fixture {
        _dir: dir,
        root,
        store,
        grants,
        service,
    }
}

#[cfg(test)]
fn kind(r: Result<impl std::fmt::Debug, Error>) -> ErrorKind {
    r.unwrap_err().kind()
}

#[tokio::test]
async fn lists_and_describes_only_what_the_caller_can_read() {
    let f = fixture().await;
    let ana = UserId::generate();
    let c = ctx(ana);
    assert!(f.service.list(&c).await.unwrap().collections.is_empty());

    f.grants.grant(ana, REQS, Need::Read);
    f.grants.grant(ana, RELS, Need::Read);
    let list = f.service.list(&c).await.unwrap();
    let titles: Vec<&str> = list.collections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Releases", "Requirements"]);
    assert_eq!(list.collections[1].rows, 2);
    assert_eq!(list.collections[1].views, ["All", "Open"]);
    assert_eq!(list.collections[1].path, REQS_DIR);

    let info = f.service.get(&c, REQS.parse().unwrap()).await.unwrap();
    let keys: Vec<&str> = info.properties.iter().map(|p| p.key.as_str()).collect();
    assert_eq!(keys, ["t", "est", "dn", "rel", "left"]);
    assert!(info.properties[4].computed);
    assert_eq!(
        info.properties[4].formula.as_deref(),
        Some("if(Done, 0, Estimate)")
    );
    assert_eq!(
        info.properties[3]
            .relation
            .as_ref()
            .unwrap()
            .back
            .as_deref(),
        Some("rq")
    );
    assert_eq!(info.views[1].filters, ["dn is false"]);
    let rels = f.service.get(&c, RELS.parse().unwrap()).await.unwrap();
    assert_eq!(rels.properties[2].rollup.as_ref().unwrap().via, "rq");

    // Hidden and unknown collections look the same.
    assert_eq!(
        kind(f.service.get(&c, SECRET.parse().unwrap()).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(f.service.get(&c, CollectionId::generate()).await),
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn reading_needs_access_to_linked_collections() {
    let f = fixture().await;
    let bo = UserId::generate();
    f.grants.grant(bo, REQS, Need::Read);
    let err = f
        .service
        .get(&ctx(bo), REQS.parse().unwrap())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
}

#[tokio::test]
async fn queries_views_with_overrides() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Read);
    f.grants.grant(ana, RELS, Need::Read);
    let c = ctx(ana);
    let q = |view: Option<&str>| QueryInput {
        collection_id: REQS.parse().unwrap(),
        view: view.map(str::to_owned),
        kind: None,
        filter: Vec::new(),
        sort: Vec::new(),
        group: None,
        hide: Vec::new(),
        limit: None,
    };
    let all = f.service.query(&c, q(None)).await.unwrap();
    let titles: Vec<&str> = all.groups[0]
        .rows
        .iter()
        .map(|r| r.title.as_str())
        .collect();
    assert_eq!(titles, ["Replay 7 days", "Webhooks"]);

    let open = f.service.query(&c, q(Some("open"))).await.unwrap();
    assert_eq!(open.kind, ViewKind::List);
    assert_eq!(open.groups[0].rows.len(), 1);

    let mut custom = q(None);
    custom.sort = vec!["Estimate asc".into()];
    custom.filter = vec!["Remaining > 0".into()];
    custom.kind = Some(ViewKind::Board);
    custom.limit = Some(5);
    let r = f.service.query(&c, custom).await.unwrap();
    assert_eq!(r.kind, ViewKind::Board);
    assert_eq!(r.groups[0].rows[0].title, "Webhooks");

    let releases = QueryInput {
        collection_id: RELS.parse().unwrap(),
        ..q(None)
    };
    let r = f.service.query(&c, releases).await.unwrap();
    let points = r.columns.iter().position(|col| col.key == "pts").unwrap();
    assert_eq!(r.groups[0].rows[0].cells[points].display, "8");

    assert_eq!(
        kind(f.service.query(&c, q(Some("Nope"))).await),
        ErrorKind::NotFound
    );
    let mut bad = q(None);
    bad.filter = vec!["Nope = 1".into()];
    assert_eq!(
        kind(f.service.query(&c, bad).await),
        ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn writes_rows_mirrors_links_and_trashes() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let reqs: CollectionId = REQS.parse().unwrap();

    let created = f
        .service
        .create_row(
            &c,
            CreateRowInput {
                collection_id: reqs,
                values: BTreeMap::from([
                    ("Requirement".to_owned(), Value::Text("Audit log".into())),
                    ("est".to_owned(), Value::Number(3.0)),
                    (
                        "Release".to_owned(),
                        Value::List(vec![Value::Text(V10.into())]),
                    ),
                ]),
                body: "Why it matters.\n".into(),
            },
        )
        .await
        .unwrap();
    let row = created.row.unwrap();
    assert_eq!(row.title, "Audit log");
    assert_eq!(row.path, format!("{REQS_DIR}/rows/audit-log.md"));
    let text = read(&f.root, &row.path);
    assert!(text.contains("t: Audit log"), "{text}");
    assert!(text.ends_with("Why it matters.\n"), "{text}");
    assert!(
        !text.contains("left:"),
        "computed values stay out of files: {text}"
    );
    // The release row on the other side gained the link.
    let release = read(&f.root, &format!("{RELS_DIR}/rows/v1-0.md"));
    assert!(release.contains(&row.row.id.to_string()), "{release}");
    assert_eq!(created.changed.written.len(), 2);

    // Same title again gets its own file.
    let twin = f
        .service
        .create_row(
            &c,
            CreateRowInput {
                collection_id: reqs,
                values: BTreeMap::from([("t".to_owned(), Value::Text("Audit log".into()))]),
                body: String::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        twin.row.unwrap().path,
        format!("{REQS_DIR}/rows/audit-log-2.md")
    );

    let updated = f
        .service
        .update_row(
            &c,
            UpdateRowInput {
                collection_id: reqs,
                row_id: R2.parse().unwrap(),
                values: BTreeMap::from([("dn".to_owned(), Value::Bool(true))]),
                body: Some("Shipped.\n".into()),
            },
        )
        .await
        .unwrap();
    let detail = updated.row.unwrap();
    assert_eq!(detail.path, format!("{REQS_DIR}/rows/webhooks.md"));
    assert_eq!(detail.body, "Shipped.\n");
    assert!(read(&f.root, &detail.path).contains("dn: true"));
    let got = f
        .service
        .get_row(
            &c,
            RowKey {
                collection_id: reqs,
                row_id: R2.parse().unwrap(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&got.row).unwrap()["cells"]["left"],
        json!({"value": {"type": "number", "value": 0.0}})
    );
}

#[tokio::test]
async fn deleting_a_row_trashes_it_and_unlinks_it() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let reqs: CollectionId = REQS.parse().unwrap();
    let v10: DocId = V10.parse().unwrap();
    let deleted = f
        .service
        .delete_row(
            &c,
            RowKey {
                collection_id: reqs,
                row_id: R1.parse().unwrap(),
            },
        )
        .await
        .unwrap();
    assert!(deleted.row.is_none());
    assert!(!f.root.join(format!("{REQS_DIR}/rows/replay.md")).exists());
    assert!(
        f.root
            .join(format!(".vellum/trash/collections/{REQS}/{R1}.md"))
            .exists()
    );
    let release = read(&f.root, &format!("{RELS_DIR}/rows/v1-0.md"));
    assert!(!release.contains(R1), "{release}");
    assert!(deleted.changed.written.iter().any(|r| r.row == v10));
}

#[tokio::test]
async fn bad_edits_are_rejected_before_anything_is_written() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let reqs: CollectionId = REQS.parse().unwrap();
    let before = read(&f.root, &format!("{REQS_DIR}/rows/webhooks.md"));
    let bad = f
        .service
        .update_row(
            &c,
            UpdateRowInput {
                collection_id: reqs,
                row_id: R2.parse().unwrap(),
                values: BTreeMap::from([("left".to_owned(), Value::Number(1.0))]),
                body: None,
            },
        )
        .await;
    assert_eq!(kind(bad), ErrorKind::InvalidInput);
    let missing = f
        .service
        .delete_row(
            &c,
            RowKey {
                collection_id: reqs,
                row_id: DocId::generate(),
            },
        )
        .await;
    assert_eq!(kind(missing), ErrorKind::NotFound);
    assert_eq!(
        read(&f.root, &format!("{REQS_DIR}/rows/webhooks.md")),
        before
    );
}

#[tokio::test]
async fn edits_need_edit_access_on_both_sides_of_a_link() {
    let f = fixture().await;
    let bo = UserId::generate();
    f.grants.grant(bo, REQS, Need::Read);
    f.grants.grant(bo, RELS, Need::Read);
    let reqs: CollectionId = REQS.parse().unwrap();
    let input = || UpdateRowInput {
        collection_id: reqs,
        row_id: R2.parse().unwrap(),
        values: BTreeMap::from([("est".to_owned(), Value::Number(1.0))]),
        body: None,
    };
    let err = f.service.update_row(&ctx(bo), input()).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::PermissionDenied);
    assert!(
        err.to_string().contains("cannot edit this collection"),
        "{err}"
    );

    f.grants.grant(bo, REQS, Need::Edit);
    let err = f.service.update_row(&ctx(bo), input()).await.unwrap_err();
    assert!(err.to_string().contains("linked collection"), "{err}");

    f.grants.grant(bo, RELS, Need::Edit);
    assert!(f.service.update_row(&ctx(bo), input()).await.is_ok());
}

#[tokio::test]
async fn a_file_changed_on_disk_is_not_overwritten() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let reqs: CollectionId = REQS.parse().unwrap();
    // Load, then someone edits the file in their editor.
    let _ = f.service.list(&c).await.unwrap();
    let path = BlobPath::new(format!("{REQS_DIR}/rows/webhooks.md")).unwrap();
    let edited = format!("---\nid: {R2}\nt: Webhooks v2\nest: 13\n---\n");
    let _ = f
        .store
        .put(&path, Bytes::from(edited.clone()), Precondition::None)
        .await
        .unwrap();
    let input = || UpdateRowInput {
        collection_id: reqs,
        row_id: R2.parse().unwrap(),
        values: BTreeMap::from([("dn".to_owned(), Value::Bool(true))]),
        body: None,
    };
    assert_eq!(
        kind(f.service.update_row(&c, input()).await),
        ErrorKind::Conflict
    );
    assert_eq!(read(&f.root, path.as_str()), edited);
    // The failed write dropped the cache, so a retry sees the new file.
    let row = f
        .service
        .update_row(&c, input())
        .await
        .unwrap()
        .row
        .unwrap();
    assert_eq!(row.title, "Webhooks v2");
    assert!(read(&f.root, path.as_str()).contains("est: 13"));
}

#[tokio::test]
async fn invalidate_rereads_the_store() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Read);
    f.grants.grant(ana, RELS, Need::Read);
    let c = ctx(ana);
    let before = f.service.list(&c).await.unwrap();
    write(
        &f.root,
        &format!("{REQS_DIR}/rows/sso.md"),
        "---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C45\nt: SSO\n---\n",
    );
    assert_eq!(f.service.list(&c).await.unwrap(), before);
    f.service.invalidate().await;
    let after = f.service.list(&c).await.unwrap();
    assert_eq!(after.collections[1].rows, 3);
    assert_eq!(f.service.readiness().await, Readiness::Ready);
}

#[tokio::test]
async fn duplicate_ids_and_bad_files_are_reported() {
    let f = fixture().await;
    write(
        &f.root,
        &format!("{REQS_DIR}/rows/webhooks copy.md"),
        &format!("---\nid: {R2}\nt: Webhooks\nest: what\n---\n"),
    );
    // A second folder claiming the same collection id is skipped.
    write(
        &f.root,
        "drives/shared/Copy/Requirements.collection/schema.yaml",
        &read(&f.root, &format!("{REQS_DIR}/schema.yaml")),
    );
    std::fs::File::options()
        .write(true)
        .open(
            f.root
                .join("drives/shared/Copy/Requirements.collection/schema.yaml"),
        )
        .unwrap()
        .set_modified(std::time::SystemTime::now() + Duration::from_mins(1))
        .unwrap();
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Read);
    f.grants.grant(ana, RELS, Need::Read);
    let info = f
        .service
        .get(&ctx(ana), REQS.parse().unwrap())
        .await
        .unwrap();
    assert_eq!(info.path, REQS_DIR);
    let messages: BTreeSet<&str> = info
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert!(
        messages.iter().any(|m| m.contains("used twice")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.contains("repeats the collection's id")),
        "{messages:?}"
    );
    assert_eq!(
        f.service.list(&ctx(ana)).await.unwrap().collections[1].rows,
        3
    );
}

#[tokio::test]
async fn operations_are_valid_and_callable() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    assert_eq!(f.service.name(), "collections");
    let ops = f.service.clone().operations();
    let ids: Vec<&str> = ops.iter().map(vellum_contracts::Operation::id).collect();
    assert_eq!(
        ids,
        [
            "collections.list",
            "collections.get",
            "collections.query",
            "collections.get_row",
            "collections.create_row",
            "collections.update_row",
            "collections.delete_row",
            "collections.save_view",
            "collections.delete_view"
        ]
    );
    for op in &ops {
        op.validate("collections").unwrap();
    }
    let by_id = |id: &str| ops.iter().find(|o| o.id() == id).unwrap();
    let list = by_id("collections.list")
        .call(ctx(ana), json!({}))
        .await
        .unwrap();
    assert_eq!(list["collections"][1]["title"], "Requirements");
    let got = by_id("collections.get")
        .call(ctx(ana), json!({"collection_id": REQS}))
        .await
        .unwrap();
    assert_eq!(got["properties"][0]["type"], "title");
    let q = by_id("collections.query")
        .call(ctx(ana), json!({"collection_id": REQS, "view": "Open"}))
        .await
        .unwrap();
    assert_eq!(q["groups"][0]["rows"].as_array().unwrap().len(), 1);
    let row = by_id("collections.get_row")
        .call(ctx(ana), json!({"collection_id": REQS, "row_id": R1}))
        .await
        .unwrap();
    assert_eq!(row["title"], "Replay 7 days");
    let created = by_id("collections.create_row")
        .call(
            ctx(ana),
            json!({"collection_id": REQS, "values": {"Requirement": {"type": "text", "value": "Exports"}}}),
        )
        .await
        .unwrap();
    let new_id = created["row"]["row"]["id"].as_str().unwrap().to_owned();
    let _ = by_id("collections.update_row")
        .call(
            ctx(ana),
            json!({"collection_id": REQS, "row_id": new_id, "values": {"est": {"type": "number", "value": 2}}}),
        )
        .await
        .unwrap();
    let _ = by_id("collections.delete_row")
        .call(ctx(ana), json!({"collection_id": REQS, "row_id": new_id}))
        .await
        .unwrap();
    let err = by_id("collections.get")
        .call(ctx(ana), json!({"collection_id": REQS, "extra": 1}))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[cfg(test)]
fn board(name: &str) -> SaveViewInput {
    SaveViewInput {
        collection_id: REQS.parse().unwrap(),
        view: name.into(),
        rename_to: None,
        kind: Some(ViewKind::Board),
        filter: vec!["Estimate > 1".into()],
        sort: vec!["Requirement asc".into()],
        group: Some("Done".into()),
        hide: vec!["Remaining".into()],
        date: None,
        end: None,
        limit: Some(20),
    }
}

#[tokio::test]
async fn saves_renames_and_deletes_views() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Read);
    let c = ctx(ana);
    let schema_path = format!("{REQS_DIR}/schema.yaml");

    // A new view is added at the end and written to schema.yaml by key.
    let info = f.service.save_view(&c, board("By status")).await.unwrap();
    let names: Vec<&str> = info.views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["All", "Open", "By status"]);
    let saved = &info.views[2];
    assert_eq!(saved.kind, ViewKind::Board);
    assert_eq!(saved.group.as_deref(), Some("dn"));
    assert_eq!(saved.hidden, ["left"]);
    let yaml = read(&f.root, &schema_path);
    assert!(yaml.contains("By status"), "{yaml}");
    assert!(yaml.contains("group: dn"), "{yaml}");

    // Saving the same name (any case) replaces it; renaming keeps its place.
    let mut again = board("by STATUS");
    again.kind = Some(ViewKind::Gallery);
    again.rename_to = Some("Cards".into());
    let info = f.service.save_view(&c, again).await.unwrap();
    let names: Vec<&str> = info.views.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["All", "Open", "Cards"]);
    assert_eq!(info.views[2].kind, ViewKind::Gallery);

    // Queries use the saved view.
    let q = QueryInput {
        collection_id: REQS.parse().unwrap(),
        view: Some("cards".into()),
        kind: None,
        filter: Vec::new(),
        sort: Vec::new(),
        group: None,
        hide: Vec::new(),
        limit: None,
    };
    assert_eq!(
        f.service.query(&c, q).await.unwrap().kind,
        ViewKind::Gallery
    );

    // Bad input and name clashes are rejected.
    let mut clash = board("Cards");
    clash.rename_to = Some("open".into());
    assert_eq!(
        kind(f.service.save_view(&c, clash).await),
        ErrorKind::Conflict
    );
    let mut bad = board("Broken");
    bad.filter = vec!["Nope = 1".into()];
    assert_eq!(
        kind(f.service.save_view(&c, bad).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.service.save_view(&c, board("  ")).await),
        ErrorKind::InvalidInput
    );

    let key = |view: &str| ViewKey {
        collection_id: REQS.parse().unwrap(),
        view: view.into(),
    };
    let info = f.service.delete_view(&c, key("CARDS")).await.unwrap();
    assert_eq!(info.views.len(), 2);
    assert!(!read(&f.root, &schema_path).contains("Cards"));
    assert_eq!(
        kind(f.service.delete_view(&c, key("Cards")).await),
        ErrorKind::NotFound
    );

    // Saving views needs edit access to this collection only.
    let bo = UserId::generate();
    f.grants.grant(bo, REQS, Need::Read);
    f.grants.grant(bo, RELS, Need::Read);
    assert_eq!(
        kind(f.service.save_view(&ctx(bo), board("Mine")).await),
        ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn a_schema_changed_on_disk_is_not_overwritten() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let _ = f.service.list(&c).await.unwrap();
    let schema_path = format!("{REQS_DIR}/schema.yaml");
    let edited = read(&f.root, &schema_path).replace("title: Requirements", "title: Reqs");
    write(&f.root, &schema_path, &edited);
    assert_eq!(
        kind(f.service.save_view(&c, board("Board")).await),
        ErrorKind::Conflict
    );
    assert_eq!(read(&f.root, &schema_path), edited);
    let info = f.service.save_view(&c, board("Board")).await.unwrap();
    assert_eq!(info.title, "Reqs");
}
