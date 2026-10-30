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
use vellum_collections::service::api::CollectionList;
use vellum_collections::service::{
    Access, AddPropertyInput, ChartInput, CollectionsService,
    CreateFromCsvInput, CreateRowInput, ExportCsvInput, ImportCsvInput,
    IndexFeed, Need, PropertyKey, QueryInput, RowKey, SaveViewInput,
    UpdatePropertyInput, UpdateRowInput, ViewKey,
};
use vellum_collections::{DateValue, Value, ViewKind, view};
use vellum_contracts::{
    CredentialKind, Principal, Readiness, RequestContext, Scope, ScopeSet,
    Service,
};
use vellum_kernel::{
    CollectionId, DocId, Error, ErrorKind, RequestId, Timestamp, UserId,
};
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
struct Grants(
    Mutex<BTreeMap<(UserId, CollectionId), Need>>,
    Mutex<BTreeSet<(UserId, String)>>,
);

#[cfg(test)]
impl Grants {
    fn grant_folder(&self, user: UserId, folder: &str) {
        let _ = self.1.lock().unwrap().insert((user, folder.to_owned()));
    }

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

    async fn allowed_in(
        &self,
        principal: &Principal,
        folder: &BlobPath,
        need: Need,
    ) -> Result<bool, Error> {
        let Some(user) = principal.user_id() else {
            return Ok(false);
        };
        Ok(need == Need::Edit
            && self
                .1
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains(&(user, folder.as_str().to_owned())))
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
            "id: {REQS}\ntitle: Requirements\n\
                properties:\n  - {{key: t, name: Requirement, type: title}}\n  \
                - {{key: est, name: Estimate, type: number}}\n  - {{key: dn, \
                name: Done, type: checkbox}}\n  - {{key: rel, name: Release, \
                type: relation, to: {RELS}, back: rq}}\n  - {{key: left, name: \
                Remaining, type: formula, f: 'if(Done, 0, Estimate)'}}\n\
                views:\n  - {{name: All, type: table, sort: est desc}}\n  - \
                {{name: Open, type: list, filter: Done is false}}\n"
        ),
    );
    write(
        root,
        &format!("{RELS_DIR}/schema.yaml"),
        &format!(
            "id: {RELS}\ntitle: Releases\n\
                properties:\n  - {{key: t, name: Release, type: title}}\n  - \
                {{key: rq, name: Requirements, type: relation, to: {REQS}, \
                back: rel}}\n  - {{key: pts, name: Points, type: rollup, via: \
                rq, prop: est, agg: sum}}\n"
        ),
    );
    write(
        root,
        &format!("{REQS_DIR}/rows/replay.md"),
        &format!(
            "---\nid: {R1}\nt: Replay 7 days\nest: 8\ndn: true\nrel: [{V10}]\n\
                ---\nReplay notes.\n"
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
            "id: {SECRET}\ntitle: Salaries\n\
                properties:\n  - {{key: t, name: Name, type: title}}\n"
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
    let titles: Vec<&str> =
        list.collections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Releases", "Requirements"]);
    assert_eq!(list.collections[1].rows, 2);
    assert_eq!(list.collections[1].views, ["All", "Open"]);
    assert_eq!(list.collections[1].path, REQS_DIR);

    let info = f.service.get(&c, REQS.parse().unwrap()).await.unwrap();
    let keys: Vec<&str> =
        info.properties.iter().map(|p| p.key.as_str()).collect();
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

    // RT-026 (#461): every filter runs against every row, so one request with
    // thousands of filters (or a megabyte needle) used to hold the
    // service for minutes. Both are refused up front now.
    let mut many = q(None);
    many.filter = vec!["Requirement contains zz".into(); view::MAX_FILTERS + 1];
    assert_eq!(
        kind(f.service.query(&c, many).await),
        ErrorKind::InvalidInput
    );
    let mut most = q(None);
    most.filter = vec!["Requirement not contains zz".into(); view::MAX_FILTERS];
    assert!(f.service.query(&c, most).await.is_ok());
    let mut long = q(None);
    long.filter = vec![format!(
        "Requirement contains {}",
        "z".repeat(view::MAX_CLAUSE_CHARS)
    )];
    assert_eq!(
        kind(f.service.query(&c, long).await),
        ErrorKind::InvalidInput
    );
    let mut sorts = q(None);
    sorts.sort = vec!["Estimate asc".into(); view::MAX_SORTS + 1];
    assert_eq!(
        kind(f.service.query(&c, sorts).await),
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
                values: BTreeMap::from([(
                    "t".to_owned(),
                    Value::Text("Audit log".into()),
                )]),
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
                values: BTreeMap::from([(
                    "left".to_owned(),
                    Value::Number(1.0),
                )]),
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
    let ids: Vec<&str> =
        ops.iter().map(vellum_contracts::Operation::id).collect();
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
            "collections.delete_view",
            "collections.add_property",
            "collections.update_property",
            "collections.delete_property",
            "collections.embed",
            "collections.chart",
            "collections.export_csv",
            "collections.import_csv",
            "collections.create_from_csv"
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
            json!({
                "collection_id": REQS,
                "values": {"Requirement": {"type": "text", "value": "Exports"}}
            }),
        )
        .await
        .unwrap();
    let new_id = created["row"]["row"]["id"].as_str().unwrap().to_owned();
    let _ = by_id("collections.update_row")
        .call(
            ctx(ana),
            json!({
                "collection_id": REQS,
                "row_id": new_id,
                "values": {"est": {"type": "number", "value": 2}}
            }),
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

#[tokio::test]
async fn resolves_embedded_views_live() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let fence =
        format!("id: {REQS}\nview: Open\ntype: board\nsort: Estimate desc\n");
    let embed = f.service.resolve_embed(&c.principal, &fence).await.unwrap();
    assert_eq!(embed.title, "Requirements");
    assert_eq!(embed.path, REQS_DIR);
    assert_eq!(embed.result.kind, ViewKind::Board);
    let titles = |r: &vellum_collections::ViewResult| -> Vec<String> {
        r.groups
            .iter()
            .flat_map(|g| &g.rows)
            .map(|row| row.title.clone())
            .collect()
    };
    assert_eq!(titles(&embed.result), ["Webhooks"]);

    // Live: a row edit shows up in the next resolve.
    let _ = f
        .service
        .create_row(
            &c,
            CreateRowInput {
                collection_id: REQS.parse().unwrap(),
                values: BTreeMap::from([
                    ("t".to_owned(), Value::Text("SSO".into())),
                    ("est".to_owned(), Value::Number(13.0)),
                ]),
                body: String::new(),
            },
        )
        .await
        .unwrap();
    let embed = f.service.resolve_embed(&c.principal, &fence).await.unwrap();
    assert_eq!(titles(&embed.result), ["SSO", "Webhooks"]);

    let bad = |body: &str| {
        let s = f.service.clone();
        let p = c.principal.clone();
        let body = body.to_owned();
        async move { s.resolve_embed(&p, &body).await.unwrap_err() }
    };
    let e = bad("id: nope\n").await;
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    let e = bad(&format!("id: {REQS}\nview: Missing\n")).await;
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    assert!(e.to_string().contains("no view named"), "{e}");
    let e = bad(&format!("id: {REQS}\ncolour: red\n")).await;
    assert_eq!(e.kind(), ErrorKind::InvalidInput);
    // A collection the reader cannot see is not found, never shown.
    let e = bad(&format!("id: {SECRET}\n")).await;
    assert_eq!(e.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn charts_read_collections() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Read);
    f.grants.grant(ana, RELS, Need::Read);
    let p = ctx(ana).principal;
    let chart = |y: Option<&str>, agg: Option<&str>| ChartInput {
        source: format!("collection:{REQS}"),
        view: None,
        x: "Done".into(),
        y: y.map(str::to_owned),
        series: None,
        agg: agg.map(str::to_owned),
    };
    let sums = f
        .service
        .resolve_chart(&p, chart(Some("Estimate"), None))
        .await
        .unwrap();
    assert_eq!(sums.categories.len(), 2);
    let total: f64 = sums.series.iter().flat_map(|s| &s.values).sum();
    assert!((total - 13.0).abs() < f64::EPSILON, "{sums:?}");
    let counts = f
        .service
        .resolve_chart(&p, chart(None, None))
        .await
        .unwrap();
    let rows: f64 = counts.series.iter().flat_map(|s| &s.values).sum();
    assert!((rows - 2.0).abs() < f64::EPSILON, "{counts:?}");

    assert_eq!(
        kind(f.service.resolve_chart(&p, chart(Some("Nope"), None)).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(
            f.service
                .resolve_chart(&p, chart(None, Some("median")))
                .await
        ),
        ErrorKind::InvalidInput
    );
    let mut csv = chart(None, None);
    csv.source = "csv:data.csv".into();
    assert_eq!(
        kind(f.service.resolve_chart(&p, csv).await),
        ErrorKind::InvalidInput
    );
    let mut hidden = chart(None, None);
    hidden.source = format!("collection:{SECRET}");
    assert_eq!(
        kind(f.service.resolve_chart(&p, hidden).await),
        ErrorKind::NotFound
    );
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

#[cfg(test)]
fn add(name: &str, kind: &str) -> AddPropertyInput {
    AddPropertyInput {
        collection_id: REQS.parse().unwrap(),
        name: name.into(),
        kind: kind.into(),
        options: Vec::new(),
        formula: None,
        via: None,
        prop: None,
        agg: None,
        format: None,
        to: None,
        back_name: None,
    }
}

#[cfg(test)]
fn change(key: &str) -> UpdatePropertyInput {
    UpdatePropertyInput {
        collection_id: REQS.parse().unwrap(),
        key: key.into(),
        name: None,
        kind: None,
        via: None,
        prop: None,
        agg: None,
        options: None,
        formula: None,
        format: None,
    }
}

#[tokio::test]
async fn adds_changes_and_removes_properties() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Read);
    let c = ctx(ana);
    let schema_path = format!("{REQS_DIR}/schema.yaml");

    // A select gets a key from its name and lands at the end.
    let mut status = add("Status", "select");
    status.options = serde_json::from_value(
        json!([{"name": "Draft"}, {"name": "Live", "color": "green"}]),
    )
    .unwrap();
    let info = f.service.add_property(&c, status).await.unwrap();
    let last = info.properties.last().unwrap();
    assert_eq!(
        (last.key.as_str(), last.kind.as_str()),
        ("status", "select")
    );
    assert_eq!(last.options.len(), 2);
    assert!(read(&f.root, &schema_path).contains("key: status"));

    // Names clash without regard to case; a second title, a relation with
    // no target and an unknown type are refused.
    assert_eq!(
        kind(f.service.add_property(&c, add("STATUS", "text")).await),
        ErrorKind::Conflict
    );
    assert_eq!(
        kind(f.service.add_property(&c, add("Owner", "relation")).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.service.add_property(&c, add("Other", "title")).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.service.add_property(&c, add("Odd", "nope")).await),
        ErrorKind::InvalidInput
    );
    let mut broken = add("Twice", "formula");
    broken.formula = Some("Nope * 2".into());
    assert_eq!(
        kind(f.service.add_property(&c, broken).await),
        ErrorKind::InvalidInput
    );

    // A formula and a rollup (its relation named, not keyed) are checked as on
    // load.
    let mut twice = add("Twice", "formula");
    twice.formula = Some("Estimate * 2".into());
    let info = f.service.add_property(&c, twice).await.unwrap();
    assert_eq!(
        info.properties.last().unwrap().formula.as_deref(),
        Some("Estimate * 2")
    );

    let mut rename = change("status");
    rename.name = Some("Stage".into());
    rename.options = Some(
        serde_json::from_value(json!([
            {"name": "Draft"},
            {"name": "Live"},
            {"name": "Live"},
            {"name": " "}
        ]))
        .unwrap(),
    );
    let info = f.service.update_property(&c, rename).await.unwrap();
    let stage = info.properties.iter().find(|p| p.key == "status").unwrap();
    assert_eq!(stage.name, "Stage");
    assert_eq!(stage.options.len(), 2);
    let mut wrong = change("dn");
    wrong.formula = Some("1".into());
    assert_eq!(
        kind(f.service.update_property(&c, wrong).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.service.update_property(&c, change("gone")).await),
        ErrorKind::NotFound
    );

    // Only editors change properties.
    let bo = UserId::generate();
    f.grants.grant(bo, REQS, Need::Read);
    f.grants.grant(bo, RELS, Need::Read);
    assert_eq!(
        kind(f.service.add_property(&ctx(bo), add("Mine", "text")).await),
        ErrorKind::PermissionDenied
    );
}

#[tokio::test]
async fn removing_a_property_cleans_up_views() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Read);
    let c = ctx(ana);
    let schema_path = format!("{REQS_DIR}/schema.yaml");
    let mut stage = add("Stage", "select");
    stage.options = serde_json::from_value(json!([{"name": "Live"}])).unwrap();
    let _ = f.service.add_property(&c, stage).await.unwrap();
    let mut twice = add("Twice", "formula");
    twice.formula = Some("Estimate * 2".into());
    let _ = f.service.add_property(&c, twice).await.unwrap();

    // Removing a property also drops the view settings that used it.
    let mut by_stage = board("By stage");
    by_stage.group = Some("Stage".into());
    by_stage.filter = vec!["Estimate > 1".into(), "Stage is Live".into()];
    by_stage.hide = Vec::new();
    let _ = f.service.save_view(&c, by_stage).await.unwrap();
    let key = |key: &str| PropertyKey {
        collection_id: REQS.parse().unwrap(),
        key: key.into(),
    };
    let info = f.service.delete_property(&c, key("stage")).await.unwrap();
    assert!(info.properties.iter().all(|p| p.key != "stage"));
    let view = info.views.iter().find(|v| v.name == "By stage").unwrap();
    assert_eq!(view.group, None);
    assert_eq!(view.filters, ["est > 1"]);
    assert!(!read(&f.root, &schema_path).contains("key: stage"));

    // The title stays, and so does a property a formula or rollup reads,
    // here or on the other side of a relation.
    assert_eq!(
        kind(f.service.delete_property(&c, key("t")).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.service.delete_property(&c, key("rel")).await),
        ErrorKind::Conflict
    );
    assert_eq!(
        kind(f.service.delete_property(&c, key("est")).await),
        ErrorKind::Conflict
    );
    let info = f.service.delete_property(&c, key("twice")).await.unwrap();
    assert!(info.properties.iter().all(|p| p.key != "twice"));
}

#[cfg(test)]
fn cell(
    row: &vellum_collections::service::RowDetail,
    key: &str,
) -> serde_json::Value {
    serde_json::to_value(row).unwrap()["row"]["cells"][key].clone()
}

#[tokio::test]
async fn relations_and_rollups_are_added_and_removed_on_both_sides() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let rels_schema = format!("{RELS_DIR}/schema.yaml");

    // A relation gets its pair in the target, named as asked.
    let mut milestone = add("Milestone", "relation");
    milestone.to = Some(RELS.parse().unwrap());
    milestone.back_name = Some("Planned work".into());
    let info = f.service.add_property(&c, milestone).await.unwrap();
    let forward = info.properties.last().unwrap();
    assert_eq!(forward.key, "milestone");
    let back = f.service.get(&c, RELS.parse().unwrap()).await.unwrap();
    let pair = back.properties.last().unwrap();
    assert_eq!(pair.name, "Planned work");
    assert!(read(&f.root, &rels_schema).contains("back: milestone"));

    // A rollup names its relation and the related property.
    let mut count = add("Milestones", "rollup");
    count.via = Some("Milestone".into());
    count.prop = Some("Release".into());
    count.agg = Some(vellum_collections::Aggregation::Count);
    let info = f.service.add_property(&c, count).await.unwrap();
    assert_eq!(info.properties.last().unwrap().kind, "rollup");

    // A relation to its own collection pairs inside it.
    let mut blocks = add("Blocks", "relation");
    blocks.to = Some(REQS.parse().unwrap());
    blocks.back_name = Some("Blocked by".into());
    let info = f.service.add_property(&c, blocks).await.unwrap();
    let names: Vec<&str> =
        info.properties.iter().map(|p| p.name.as_str()).collect();
    assert!(names.ends_with(&["Blocks", "Blocked by"]), "{names:?}");

    // Removing a relation waits for its rollup, then takes the pair along.
    let key = |key: &str| PropertyKey {
        collection_id: REQS.parse().unwrap(),
        key: key.into(),
    };
    assert_eq!(
        kind(f.service.delete_property(&c, key("milestone")).await),
        ErrorKind::Conflict
    );
    let _ = f
        .service
        .delete_property(&c, key("milestones"))
        .await
        .unwrap();
    let _ = f
        .service
        .delete_property(&c, key("milestone"))
        .await
        .unwrap();
    assert!(!read(&f.root, &rels_schema).contains("Planned work"));

    // Linking needs edit access to the other collection; nothing is written.
    let bo = UserId::generate();
    f.grants.grant(bo, REQS, Need::Edit);
    f.grants.grant(bo, RELS, Need::Read);
    let before = read(&f.root, &format!("{REQS_DIR}/schema.yaml"));
    let mut sneaky = add("Sneaky", "relation");
    sneaky.to = Some(RELS.parse().unwrap());
    assert_eq!(
        kind(f.service.add_property(&ctx(bo), sneaky).await),
        ErrorKind::PermissionDenied
    );
    assert_eq!(read(&f.root, &format!("{REQS_DIR}/schema.yaml")), before);
}

#[tokio::test]
async fn types_change_without_losing_values_and_renames_rewrite_formulas() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let row_key = || RowKey {
        collection_id: REQS.parse().unwrap(),
        row_id: R1.parse().unwrap(),
    };
    let row_path = format!("{REQS_DIR}/rows/replay.md");

    // Renaming a property rewrites the formulas that name it.
    let mut rename = change("est");
    rename.name = Some("Story points".into());
    let info = f.service.update_property(&c, rename).await.unwrap();
    let left = info.properties.iter().find(|p| p.key == "left").unwrap();
    assert_eq!(
        left.formula.as_deref(),
        Some(r#"if(Done, 0, prop("Story points"))"#)
    );
    let row = f.service.get_row(&c, row_key()).await.unwrap();
    assert_eq!(cell(&row, "left")["value"]["type"], "number");

    // A checkbox that becomes a number keeps its value in the file and
    // reports it; turning it back restores it.
    let mut to_number = change("dn");
    to_number.kind = Some("number".into());
    let info = f.service.update_property(&c, to_number).await.unwrap();
    assert!(
        info.diagnostics
            .iter()
            .any(|d| d.message.contains("kept as written")),
        "{:?}",
        info.diagnostics
    );
    assert!(read(&f.root, &row_path).contains("dn: true"));
    let mut back = change("dn");
    back.kind = Some("checkbox".into());
    let info = f.service.update_property(&c, back).await.unwrap();
    assert!(info.diagnostics.is_empty(), "{:?}", info.diagnostics);
    let row = f.service.get_row(&c, row_key()).await.unwrap();
    assert_eq!(
        cell(&row, "dn")["value"],
        json!({"type": "bool", "value": true})
    );

    // A number can become a select: its values read as the option text.
    let mut to_select = change("est");
    to_select.kind = Some("select".into());
    to_select.options =
        Some(serde_json::from_value(json!([{"name": "8"}])).unwrap());
    let _ = f.service.update_property(&c, to_select).await.unwrap();
    let row = f.service.get_row(&c, row_key()).await.unwrap();
    assert_eq!(
        cell(&row, "est")["value"],
        json!({"type": "text", "value": "8"})
    );
    let mut to_text = change("dn");
    to_text.kind = Some("text".into());
    let _ = f.service.update_property(&c, to_text).await.unwrap();
    let row = f.service.get_row(&c, row_key()).await.unwrap();
    assert_eq!(
        cell(&row, "dn")["value"],
        json!({"type": "text", "value": "true"})
    );

    // Titles and relations keep their type.
    for (key, to) in [("t", "text"), ("rel", "text"), ("est", "relation")] {
        let mut wrong = change(key);
        wrong.kind = Some(to.into());
        assert_eq!(
            kind(f.service.update_property(&c, wrong).await),
            ErrorKind::InvalidInput,
            "{key} to {to}"
        );
    }
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
    let edited = read(&f.root, &schema_path)
        .replace("title: Requirements", "title: Reqs");
    write(&f.root, &schema_path, &edited);
    assert_eq!(
        kind(f.service.save_view(&c, board("Board")).await),
        ErrorKind::Conflict
    );
    assert_eq!(read(&f.root, &schema_path), edited);
    let info = f.service.save_view(&c, board("Board")).await.unwrap();
    assert_eq!(info.title, "Reqs");
}

#[tokio::test]
async fn exports_views_as_csv() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Read);
    f.grants.grant(ana, RELS, Need::Read);
    let c = ctx(ana);
    let reqs: CollectionId = REQS.parse().unwrap();
    let all = f
        .service
        .export_csv(
            &c,
            ExportCsvInput {
                collection_id: reqs,
                view: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(all.file_name, "Requirements.csv");
    let mut lines = all.csv.lines();
    assert_eq!(
        lines.next(),
        Some("Requirement,Estimate,Done,Release,Remaining")
    );
    assert_eq!(lines.next(), Some("Replay 7 days,8,Yes,v1.0,0"));
    assert_eq!(lines.next(), Some("Webhooks,5,No,,5"));

    let open = f
        .service
        .export_csv(
            &c,
            ExportCsvInput {
                collection_id: reqs,
                view: Some("Open".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(open.file_name, "Requirements - Open.csv");
    assert_eq!(open.csv.lines().count(), 2);

    let missing = f
        .service
        .export_csv(
            &c,
            ExportCsvInput {
                collection_id: reqs,
                view: Some("Nope".into()),
            },
        )
        .await;
    assert_eq!(kind(missing), ErrorKind::NotFound);
    let hidden = f
        .service
        .export_csv(
            &ctx(UserId::generate()),
            ExportCsvInput {
                collection_id: reqs,
                view: None,
            },
        )
        .await;
    assert_eq!(kind(hidden), ErrorKind::NotFound);
}

#[tokio::test]
async fn imports_csv_rows_into_a_collection() {
    let f = fixture().await;
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let c = ctx(ana);
    let reqs: CollectionId = REQS.parse().unwrap();
    let text = "Requirement,Estimate,Release,Colour\nSSO,13,v1.0,red\n\
        Audit log,abc,,blue\n";
    let input = |dry_run| ImportCsvInput {
        collection_id: reqs,
        csv: text.into(),
        dry_run,
    };

    let preview = f.service.import_csv(&c, input(true)).await.unwrap();
    assert!(!preview.written);
    assert_eq!(preview.rows, 2);
    assert_eq!(preview.skipped_columns, ["Colour"]);
    assert!(!preview.diagnostics.is_empty(), "abc is not a number");
    assert!(preview.changed.written.is_empty());
    assert!(!f.root.join(format!("{REQS_DIR}/rows/sso.md")).exists());

    let done = f.service.import_csv(&c, input(false)).await.unwrap();
    assert!(done.written);
    // Two new rows, plus the release that gained a link.
    assert_eq!(done.changed.written.len(), 3);
    let sso = read(&f.root, &format!("{REQS_DIR}/rows/sso.md"));
    assert!(sso.contains("est: 13"), "{sso}");
    let release = read(&f.root, &format!("{RELS_DIR}/rows/v1-0.md"));
    let sso_id = sso.lines().find_map(|l| l.strip_prefix("id: ")).unwrap();
    assert!(release.contains(sso_id), "{release}");
    assert_eq!(f.service.list(&c).await.unwrap().collections[1].rows, 4);

    let bo = UserId::generate();
    f.grants.grant(bo, REQS, Need::Read);
    f.grants.grant(bo, RELS, Need::Read);
    assert_eq!(
        kind(f.service.import_csv(&ctx(bo), input(false)).await),
        ErrorKind::PermissionDenied
    );
    let huge = ImportCsvInput {
        collection_id: reqs,
        csv: "a".repeat(vellum_collections::service::MAX_CSV_BYTES + 1),
        dry_run: true,
    };
    assert_eq!(
        kind(f.service.import_csv(&c, huge).await),
        ErrorKind::InvalidInput
    );
}

#[tokio::test]
async fn creates_a_collection_from_csv() {
    let f = fixture().await;
    let ana = UserId::generate();
    let c = ctx(ana);
    let folder = "drives/shared/Northstar";
    let input = |dry_run| CreateFromCsvInput {
        folder: folder.into(),
        title: "Vendors".into(),
        csv: "Name,Annual cost,Renewal,Active\nAcme,1200,2026-12-01,true\n\
            Globex,800,2027-03-15,false\n"
            .into(),
        dry_run,
    };
    assert_eq!(
        kind(f.service.create_from_csv(&c, input(false)).await),
        ErrorKind::PermissionDenied
    );
    f.grants.grant_folder(ana, folder);

    let preview = f.service.create_from_csv(&c, input(true)).await.unwrap();
    assert!(!preview.written);
    let types: Vec<&str> = preview
        .collection
        .properties
        .iter()
        .map(|p| p.kind.as_str())
        .collect();
    assert_eq!(types, ["title", "number", "date", "checkbox"]);
    let dir = format!("{folder}/Vendors.collection");
    assert_eq!(preview.collection.path, dir);
    assert!(!f.root.join(&dir).exists());

    let made = f.service.create_from_csv(&c, input(false)).await.unwrap();
    assert!(made.written);
    assert_eq!(made.rows, 2);
    assert!(
        read(&f.root, &format!("{dir}/schema.yaml")).contains("title: Vendors")
    );
    assert!(read(&f.root, &format!("{dir}/rows/acme.md")).contains("Acme"));
    let id = made.collection.id;
    f.grants.grant(ana, &id.to_string(), Need::Edit);
    let info = f.service.get(&c, id).await.unwrap();
    assert_eq!(info.title, "Vendors");

    assert_eq!(
        kind(f.service.create_from_csv(&c, input(false)).await),
        ErrorKind::Conflict
    );
    let mut system = input(false);
    system.folder = ".vellum".into();
    f.grants.grant_folder(ana, ".vellum");
    assert_eq!(
        kind(f.service.create_from_csv(&c, system).await),
        ErrorKind::InvalidInput
    );
    let mut blank = input(false);
    blank.title = "  ".into();
    assert_eq!(
        kind(f.service.create_from_csv(&c, blank).await),
        ErrorKind::InvalidInput
    );
}

#[derive(Default)]
struct Feed(Mutex<Vec<DocId>>);

impl IndexFeed for Feed {
    fn rows_changed(&self, rows: Vec<DocId>) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(rows);
    }
}

#[cfg(test)]
fn take(feed: &Feed) -> BTreeSet<String> {
    std::mem::take(&mut *feed.0.lock().unwrap())
        .into_iter()
        .map(|d| d.to_string())
        .collect()
}

#[tokio::test]
async fn feeds_search_rows_with_computed_values() {
    let f = fixture().await;
    let feed = Arc::new(Feed::default());
    let service = CollectionsService::new(f.store.clone(), f.grants.clone())
        .with_clock(Arc::new(|| DateValue::ymd(2026, 10, 29).unwrap()))
        .with_index_feed(feed.clone());
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let reqs = REQS.parse().unwrap();

    // Every row is listed for a rebuild; the empty collection adds none.
    let all: BTreeSet<String> = service
        .index_rows()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.to_string())
        .collect();
    assert_eq!(all, BTreeSet::from([R1.into(), R2.into(), V10.into()]));

    // A row renders its stored and computed values as text.
    let r1 = service
        .index_row(R1.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r1.title, "Replay 7 days");
    assert_eq!(r1.collection_title, "Requirements");
    assert_eq!(r1.folder, REQS_DIR);
    assert_eq!(r1.path, format!("{REQS_DIR}/rows/replay.md"));
    assert_eq!(r1.body, "Replay notes.\n");
    let fields: Vec<(&str, &str, bool)> = r1
        .fields
        .iter()
        .map(|x| (x.name.as_str(), x.text.as_str(), x.computed))
        .collect();
    assert_eq!(
        fields,
        [
            ("Estimate", "8", false),
            ("Done", "Yes", false),
            ("Release", "v1.0", false),
            ("Remaining", "0", true),
        ]
    );

    // Editing a requirement re-indexes it and the release whose rollup reads
    // it.
    let _ = service
        .update_row(
            &ctx(ana),
            UpdateRowInput {
                collection_id: reqs,
                row_id: R1.parse().unwrap(),
                values: BTreeMap::from([(
                    "est".to_owned(),
                    Value::Number(13.0),
                )]),
                body: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(take(&feed), BTreeSet::from([R1.into(), V10.into()]));
    let v10 = service
        .index_row(V10.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    let points = v10.fields.iter().find(|x| x.key == "pts").unwrap();
    assert_eq!((points.text.as_str(), points.computed), ("13", true));

    // A deleted row is fed once more and then reads as gone.
    let _ = service
        .delete_row(
            &ctx(ana),
            RowKey {
                collection_id: reqs,
                row_id: R2.parse().unwrap(),
            },
        )
        .await
        .unwrap();
    assert!(take(&feed).contains(R2));
    assert!(
        service
            .index_row(R2.parse().unwrap())
            .await
            .unwrap()
            .is_none()
    );

    // Imports feed the rows they create.
    let imported = service
        .import_csv(
            &ctx(ana),
            ImportCsvInput {
                collection_id: reqs,
                csv: "Requirement,Estimate\nAudit log,3\n".into(),
                dry_run: false,
            },
        )
        .await
        .unwrap();
    let new_row = imported.changed.written.first().unwrap().row.to_string();
    assert!(take(&feed).contains(&new_row));
}

/// Access that runs a hook once, on the first edit check, then yields so
/// a task the hook spawned can queue for the write lock.
struct Interleave {
    grants: Arc<Grants>,
    hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

#[async_trait]
impl Access for Interleave {
    async fn allowed(
        &self,
        principal: &Principal,
        collection: CollectionId,
        path: &BlobPath,
        need: Need,
    ) -> Result<bool, Error> {
        let hook = if need == Need::Edit {
            self.hook
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
        } else {
            None
        };
        if let Some(hook) = hook {
            hook();
            tokio::task::yield_now().await;
        }
        self.grants.allowed(principal, collection, path, need).await
    }
}

#[tokio::test]
async fn an_import_does_not_overwrite_a_write_that_lands_meanwhile() {
    let f = fixture().await;
    let access = Arc::new(Interleave {
        grants: f.grants.clone(),
        hook: Mutex::new(None),
    });
    let service =
        Arc::new(CollectionsService::new(f.store.clone(), access.clone()));
    let ana = UserId::generate();
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let reqs = REQS.parse().unwrap();

    // While the import matches its file under the read lock, another
    // request edits a row and gets the write lock first.
    let other = Arc::clone(&service);
    let edit = Arc::new(Mutex::new(None));
    let slot = Arc::clone(&edit);
    *access.hook.lock().unwrap() = Some(Box::new(move || {
        let handle = tokio::spawn(async move {
            other
                .update_row(
                    &ctx(ana),
                    UpdateRowInput {
                        collection_id: reqs,
                        row_id: R2.parse().unwrap(),
                        values: BTreeMap::from([(
                            "est".to_owned(),
                            Value::Number(21.0),
                        )]),
                        body: None,
                    },
                )
                .await
                .map(|_| ())
        });
        *slot.lock().unwrap() = Some(handle);
    }));
    let imported = service
        .import_csv(
            &ctx(ana),
            ImportCsvInput {
                collection_id: reqs,
                csv: "Requirement,Estimate\nAudit log,3\n".into(),
                dry_run: false,
            },
        )
        .await
        .unwrap();
    let handle = edit.lock().unwrap().take().unwrap();
    handle.await.unwrap().unwrap();
    assert_eq!(imported.rows, 1);

    // Both writes survive, in memory and on disk.
    let key = |row: &str| RowKey {
        collection_id: reqs,
        row_id: row.parse().unwrap(),
    };
    let r2 = service.get_row(&ctx(ana), key(R2)).await.unwrap();
    assert_eq!(r2.row.cells["est"], Ok(Value::Number(21.0)));
    let new_row = imported.changed.written.first().unwrap().row.to_string();
    assert!(service.get_row(&ctx(ana), key(&new_row)).await.is_ok());
    service.invalidate().await;
    let r2 = service.get_row(&ctx(ana), key(R2)).await.unwrap();
    assert_eq!(r2.row.cells["est"], Ok(Value::Number(21.0)));
}

/// Another replica's writes (ADR 0038): a second service over the same
/// files sees them once the changed folders are refreshed, and always ends
/// up where a fresh full load would.
#[tokio::test]
async fn refresh_reloads_the_folders_another_replica_changed() {
    let f = fixture().await;
    let other =
        Arc::new(CollectionsService::new(f.store.clone(), f.grants.clone()));
    let ana = UserId::generate();
    for id in [REQS, RELS, SECRET] {
        f.grants.grant(ana, id, Need::Read);
    }
    let c = ctx(ana);
    type Summary = Vec<(String, usize)>;
    let summary = |list: CollectionList| -> Summary {
        list.collections
            .into_iter()
            .map(|s| (s.title, s.rows))
            .collect()
    };
    let fresh = || async {
        let s = CollectionsService::new(f.store.clone(), f.grants.clone());
        summary(s.list(&c).await.unwrap())
    };
    let dir = |d: &str| BTreeSet::from([BlobPath::new(d).unwrap()]);
    assert_eq!(
        summary(other.list(&c).await.unwrap()),
        [
            ("Releases".to_owned(), 1),
            ("Requirements".to_owned(), 2),
            ("Salaries".to_owned(), 0)
        ]
    );

    // A row written elsewhere shows after its folder is refreshed.
    write(
        &f.root,
        &format!("{REQS_DIR}/rows/sso.md"),
        "---\nid: 01JBQ8Y4ZK6V3W2N5R7T9X1C43\nt: SSO\nest: 3\n---\n",
    );
    assert_eq!(summary(other.list(&c).await.unwrap())[1].1, 2);
    other.refresh(&dir(REQS_DIR)).await;
    assert_eq!(summary(other.list(&c).await.unwrap())[1].1, 3);
    assert_eq!(summary(other.list(&c).await.unwrap()), fresh().await);

    // A folder that went is dropped.
    std::fs::remove_dir_all(f.root.join(SECRET_DIR)).unwrap();
    other.refresh(&dir(SECRET_DIR)).await;
    assert_eq!(summary(other.list(&c).await.unwrap()), fresh().await);
    assert_eq!(summary(other.list(&c).await.unwrap()).len(), 2);

    // Releases went but Requirements still relates to it: a targeted
    // reload cannot match a full one, so everything is read again.
    std::fs::remove_dir_all(f.root.join(RELS_DIR)).unwrap();
    other.refresh(&dir(RELS_DIR)).await;
    assert_eq!(summary(other.list(&c).await.unwrap()), fresh().await);
}

/// Refuses any row whose Markdown holds "SECRET", recording what it saw.
#[derive(Debug, Default)]
struct NoSecrets(Mutex<Vec<String>>);

#[async_trait]
impl vellum_collections::service::RowCheck for NoSecrets {
    async fn check(
        &self,
        _: &Principal,
        path: &BlobPath,
        text: &str,
    ) -> Result<(), Error> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(path.as_str().to_owned());
        if text.contains("SECRET") {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "secrets found: test",
            ));
        }
        Ok(())
    }
}

#[tokio::test]
async fn the_row_check_sees_every_row_write_and_a_refusal_writes_nothing() {
    let f = fixture().await;
    let check = Arc::new(NoSecrets::default());
    let service = CollectionsService::new(f.store.clone(), f.grants.clone())
        .with_clock(Arc::new(|| DateValue::ymd(2026, 10, 29).unwrap()))
        .with_row_check(check.clone());
    let ana = UserId::generate();
    let c = ctx(ana);
    f.grants.grant(ana, REQS, Need::Edit);
    f.grants.grant(ana, RELS, Need::Edit);
    let reqs: CollectionId = REQS.parse().unwrap();
    let rows_before = std::fs::read_dir(f.root.join(REQS_DIR).join("rows"))
        .unwrap()
        .count();

    // Create, update and CSV import are each refused, and nothing is written.
    let create = |body: &str| CreateRowInput {
        collection_id: reqs,
        values: BTreeMap::new(),
        body: body.to_owned(),
    };
    let refused = service.create_row(&c, create("key: SECRET\n")).await;
    assert_eq!(kind(refused), ErrorKind::InvalidInput);
    let update = UpdateRowInput {
        collection_id: reqs,
        row_id: R1.parse().unwrap(),
        values: BTreeMap::new(),
        body: Some("now with a SECRET\n".into()),
    };
    assert_eq!(
        kind(service.update_row(&c, update).await),
        ErrorKind::InvalidInput
    );
    assert!(
        check
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.ends_with("rows/replay.md"))
    );
    let import = ImportCsvInput {
        collection_id: reqs,
        csv: "Requirement\nSECRET token row\n".into(),
        dry_run: false,
    };
    assert_eq!(
        kind(service.import_csv(&c, import).await),
        ErrorKind::InvalidInput
    );
    let rows_after = std::fs::read_dir(f.root.join(REQS_DIR).join("rows"))
        .unwrap()
        .count();
    assert_eq!(rows_before, rows_after, "a refused write left a file");
    assert!(
        read(&f.root, &format!("{REQS_DIR}/rows/replay.md"))
            .contains("Replay notes.")
    );
    // The refused edits were dropped from memory too.
    let row = service
        .index_row(R1.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.body, "Replay notes.\n");

    // A clean row still goes through.
    let _ = service
        .create_row(&c, create("Rotates weekly.\n"))
        .await
        .unwrap();

    // So does creating a collection from CSV, unless a row holds a secret.
    let folder = "drives/shared/Northstar";
    f.grants.grant_folder(ana, folder);
    let from_csv = |csv: &str| CreateFromCsvInput {
        folder: folder.into(),
        title: "Vendors".into(),
        csv: csv.into(),
        dry_run: false,
    };
    assert_eq!(
        kind(
            service
                .create_from_csv(&c, from_csv("Name\nSECRET\n"))
                .await
        ),
        ErrorKind::InvalidInput
    );
    assert!(!f.root.join(folder).join("Vendors.collection").exists());
    let made = service
        .create_from_csv(&c, from_csv("Name\nAcme\n"))
        .await
        .unwrap();
    assert!(made.written);
}
