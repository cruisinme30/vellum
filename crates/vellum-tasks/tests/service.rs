// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The My Work service end to end, over an in-memory workspace.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::significant_drop_tightening,
    clippy::too_many_lines
)]

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use vellum_contracts::{BoxFuture, CredentialKind, Principal, Scope, ScopeSet, Service};
use vellum_kernel::{CommentId, DocId, Error, ErrorKind, ShareTokenHash, UserId};
use vellum_tasks::lists;
use vellum_tasks::model::{Origin, Task};
use vellum_tasks::port::{Document, Profile, SecretKind, Workspace};
use vellum_tasks::projection::FileContext;
use vellum_tasks::raw::{RECIPIENT_HEADER, RawRequest, RawResponse};
use vellum_tasks::schedule::NoticeKind;
use vellum_tasks::service::{
    BucketName, CompleteInput, Config, CreateInput, CreateListInput, ImportFrom, ImportInput,
    MemberInput, MentionRef, MoveInput, MyWorkView, NoInput, ParseInput, TaskRef, TasksService,
    UpdateInput,
};
use vellum_tasks::settings::{MyWorkSettingsInput, Preferences};

#[derive(Debug, Clone)]
struct File {
    path: String,
    text: String,
    version: u64,
}

#[derive(Default)]
struct State {
    files: BTreeMap<DocId, File>,
    secrets: HashMap<SecretKind, ([u8; 32], UserId)>,
    members: Vec<(DocId, String)>,
    preferences: Preferences,
    dismissed: Vec<CommentId>,
}

struct Mem {
    user: UserId,
    state: Mutex<State>,
}

fn context(doc: DocId, file: &File) -> FileContext {
    let parts: Vec<&str> = file.path.split('/').collect();
    let (origin, owner, list) = match parts.as_slice() {
        ["tasks", owner, "inbox.md"] => (
            Origin::Inbox,
            Some((*owner).to_owned()),
            Some("Inbox".to_owned()),
        ),
        ["tasks", owner, _] => (
            Origin::List,
            Some((*owner).to_owned()),
            Some(lists::read_meta(&file.text).title),
        ),
        _ => (Origin::Doc, None, None),
    };
    FileContext {
        doc,
        origin,
        owner,
        list,
    }
}

fn document(doc: DocId, file: &File) -> Document {
    Document {
        context: context(doc, file),
        text: file.text.clone(),
        version: file.version,
        modified: "2026-11-01T09:00:00Z".parse().unwrap(),
    }
}

fn ready<T: Send + 'static>(value: T) -> BoxFuture<'static, T> {
    Box::pin(std::future::ready(value))
}

impl Workspace for Mem {
    fn profile(&self, user: UserId) -> BoxFuture<'_, Result<Profile, Error>> {
        let preferences = self.state.lock().unwrap().preferences.clone();
        ready(Ok(Profile {
            user,
            handle: "ana".into(),
            display_name: "Ana".into(),
            time_zone: preferences.zone_or(&TimeZone::get("Europe/London").unwrap()),
            preferences,
        }))
    }

    fn task_files(&self, _user: UserId) -> BoxFuture<'_, Result<Vec<Document>, Error>> {
        let state = self.state.lock().unwrap();
        ready(Ok(state
            .files
            .iter()
            .map(|(d, f)| document(*d, f))
            .collect()))
    }

    fn assigned(&self, _user: UserId) -> BoxFuture<'_, Result<Vec<Task>, Error>> {
        ready(Ok(Vec::new()))
    }

    fn read(&self, _user: UserId, doc: DocId) -> BoxFuture<'_, Result<Document, Error>> {
        let state = self.state.lock().unwrap();
        ready(
            state
                .files
                .get(&doc)
                .map(|f| document(doc, f))
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "no such file")),
        )
    }

    fn write(
        &self,
        _user: UserId,
        doc: DocId,
        text: String,
        version: u64,
    ) -> BoxFuture<'_, Result<u64, Error>> {
        let mut state = self.state.lock().unwrap();
        let file = state.files.get_mut(&doc).unwrap();
        if file.version != version {
            return ready(Err(Error::new(ErrorKind::Conflict, "stale")));
        }
        file.text = text;
        file.version += 1;
        ready(Ok(file.version))
    }

    fn create(
        &self,
        _user: UserId,
        path: String,
        text: String,
    ) -> BoxFuture<'_, Result<DocId, Error>> {
        let id = lists::read_meta(&text).id.unwrap();
        let mut state = self.state.lock().unwrap();
        let _ = state.files.insert(
            id,
            File {
                path,
                text,
                version: 1,
            },
        );
        ready(Ok(id))
    }

    fn set_member(
        &self,
        _user: UserId,
        list: DocId,
        handle: String,
        member: bool,
    ) -> BoxFuture<'_, Result<(), Error>> {
        let mut state = self.state.lock().unwrap();
        state.members.retain(|(d, h)| !(*d == list && *h == handle));
        if member {
            state.members.push((list, handle));
        }
        ready(Ok(()))
    }

    fn set_secret(
        &self,
        user: UserId,
        kind: SecretKind,
        hash: Option<ShareTokenHash>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        let mut state = self.state.lock().unwrap();
        match hash {
            Some(hash) => {
                let _ = state.secrets.insert(kind, (*hash.as_bytes(), user));
            }
            None => {
                let _ = state.secrets.remove(&kind);
            }
        }
        ready(Ok(()))
    }

    fn user_for_secret(
        &self,
        kind: SecretKind,
        hash: ShareTokenHash,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>> {
        let state = self.state.lock().unwrap();
        ready(Ok(state
            .secrets
            .get(&kind)
            .filter(|(h, _)| h == hash.as_bytes())
            .map(|(_, u)| *u)))
    }

    fn slack_user(
        &self,
        _team: String,
        _slack_user: String,
    ) -> BoxFuture<'_, Result<Option<UserId>, Error>> {
        ready(Ok(Some(self.user)))
    }

    fn save_preferences(
        &self,
        _user: UserId,
        preferences: Preferences,
    ) -> BoxFuture<'_, Result<(), Error>> {
        self.state.lock().unwrap().preferences = preferences;
        ready(Ok(()))
    }

    fn dismiss_mention(
        &self,
        _user: UserId,
        comment: CommentId,
    ) -> BoxFuture<'_, Result<(), Error>> {
        self.state.lock().unwrap().dismissed.push(comment);
        ready(Ok(()))
    }

    fn people(&self) -> BoxFuture<'_, Result<Vec<UserId>, Error>> {
        ready(Ok(vec![self.user]))
    }
}

struct Fixture {
    user: UserId,
    mem: Arc<Mem>,
    service: Arc<TasksService>,
}

fn fixture() -> Fixture {
    let user = UserId::generate();
    let mem = Arc::new(Mem {
        user,
        state: Mutex::new(State::default()),
    });
    let config = Config {
        base_url: "https://vellum.northstar.dev/".into(),
        mail_domain: "northstar.dev".into(),
        slack_signing_secret: None,
    };
    let service = TasksService::new(mem.clone(), config).with_clock(Arc::new(|| {
        "2026-11-03T10:00:00Z".parse::<Timestamp>().unwrap()
    }));
    Fixture {
        user,
        mem,
        service: Arc::new(service),
    }
}

impl Fixture {
    fn file(&self, path: &str) -> String {
        let state = self.mem.state.lock().unwrap();
        state
            .files
            .values()
            .find(|f| f.path == path)
            .map(|f| f.text.clone())
            .unwrap_or_default()
    }

    fn principal(&self, scopes: &[Scope]) -> Principal {
        Principal::user(
            self.user,
            CredentialKind::ApiToken,
            ScopeSet::from_scopes(scopes.iter().copied()),
            vellum_kernel::Timestamp::now(),
        )
    }

    async fn raw(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> RawResponse {
        self.service
            .handle_raw(RawRequest {
                method: method.into(),
                path: path.split_once('?').map_or(path, |(p, _)| p).into(),
                query: path.split_once('?').map_or("", |(_, q)| q).into(),
                headers: headers
                    .iter()
                    .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
                    .collect(),
                body: body.as_bytes().to_vec(),
                principal: self.principal(&[Scope::Read, Scope::WriteDirect]),
            })
            .await
    }

    async fn add(&self, text: &str) -> String {
        self.service
            .create(
                self.user,
                CreateInput {
                    text: text.into(),
                    list: None,
                    parent: None,
                },
            )
            .await
            .unwrap()
            .id
    }
}

fn section(view: &MyWorkView, bucket: BucketName) -> Vec<&str> {
    view.sections
        .iter()
        .filter(|s| s.bucket == bucket)
        .flat_map(|s| s.tasks.iter().map(|t| t.title.as_str()))
        .collect()
}

fn body(response: &RawResponse) -> String {
    String::from_utf8(response.body.clone()).unwrap()
}

fn header<'a>(response: &'a RawResponse, name: &str) -> Option<&'a str> {
    response
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[test]
fn operations_are_unique_and_versioned() {
    let f = fixture();
    let operations = Arc::clone(&f.service).operations();
    assert_eq!(f.service.name(), "tasks");
    let mut ids: Vec<&str> = operations
        .iter()
        .map(vellum_contracts::Operation::id)
        .collect();
    assert!(ids.iter().all(|id| id.starts_with("tasks.")));
    assert!(operations.iter().all(|o| o.path().starts_with("/v1/")));
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count);
    assert_eq!(count, 25);
}

#[tokio::test]
async fn quick_add_lands_in_the_right_bucket() {
    let f = fixture();
    let _ = f.add("Buy milk").await;
    let _ = f.add("Pay rent tomorrow p1").await;
    let _ = f.add("Call Bo today 3pm +bo").await;
    let inbox = f.file("tasks/ana/inbox.md");
    assert!(inbox.starts_with("---\nid: "), "{inbox}");
    assert!(inbox.contains("- [ ] Buy milk ^blk-"), "{inbox}");
    assert!(
        inbox.contains("- [ ] Pay rent {due=2026-11-04 p=1} ^blk-"),
        "{inbox}"
    );
    assert!(
        inbox.contains("- [ ] Call Bo @bo {due=2026-11-03T15:00} ^blk-"),
        "{inbox}"
    );

    let view = f.service.my_work(f.user, NoInput {}).await.unwrap();
    assert_eq!(view.today, "2026-11-03");
    assert_eq!(section(&view, BucketName::Inbox), ["Buy milk"]);
    // Assigned to Bo, so it is on Bo's My Work, not Ana's.
    assert!(section(&view, BucketName::Today).is_empty());
    assert_eq!(section(&view, BucketName::Upcoming), ["Pay rent"]);
    assert_eq!(view.open, 2);

    let parsed = f
        .service
        .parse(
            f.user,
            ParseInput {
                text: "Café run friday".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(parsed.title, "Café run");
    assert_eq!(parsed.fields.due.as_deref(), Some("2026-11-06"));
    // UTF-16 offsets: "Café run " is nine code units.
    assert_eq!(
        (parsed.highlights[0].start, parsed.highlights[0].end),
        (9, 15)
    );

    let empty = f
        .service
        .create(
            f.user,
            CreateInput {
                text: "tomorrow".into(),
                list: None,
                parent: None,
            },
        )
        .await;
    assert_eq!(empty.unwrap_err().kind(), ErrorKind::InvalidInput);
}

#[tokio::test]
async fn tasks_complete_move_update_and_delete() {
    let f = fixture();
    let weekly = f.add("Water plants every monday").await;
    let done = f
        .service
        .complete(
            f.user,
            CompleteInput {
                task: weekly.clone(),
                cascade: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(done.next.as_deref(), Some("2026-11-16"));
    assert!(!done.task.done);

    let id = f.add("Write report").await;
    let child = f
        .service
        .create(
            f.user,
            CreateInput {
                text: "Outline".into(),
                list: None,
                parent: Some(id.clone()),
            },
        )
        .await
        .unwrap();
    assert!(
        f.file("tasks/ana/inbox.md")
            .contains("\n  - [ ] Outline ^blk-")
    );

    let moved = f
        .service
        .move_to(
            f.user,
            MoveInput {
                task: id.clone(),
                to: "2026-11-05".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(moved.fields.start.as_deref(), Some("2026-11-05"));
    let bad = f
        .service
        .move_to(
            f.user,
            MoveInput {
                task: id.clone(),
                to: "later".into(),
            },
        )
        .await;
    assert_eq!(bad.unwrap_err().kind(), ErrorKind::InvalidInput);

    let updated = f
        .service
        .update(
            f.user,
            UpdateInput {
                task: id.clone(),
                title: Some("Write the report".into()),
                due: Some("2026-11-07".into()),
                start: Some(String::new()),
                labels: Some("work, q4".into()),
                ..UpdateInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Write the report");
    assert_eq!(updated.fields.start, None);
    assert_eq!(updated.fields.labels, ["work", "q4"]);
    let invalid = f
        .service
        .update(
            f.user,
            UpdateInput {
                task: id.clone(),
                priority: Some("9".into()),
                ..UpdateInput::default()
            },
        )
        .await;
    assert_eq!(invalid.unwrap_err().kind(), ErrorKind::InvalidInput);

    let completed = f
        .service
        .complete(
            f.user,
            CompleteInput {
                task: id.clone(),
                cascade: true,
            },
        )
        .await
        .unwrap();
    assert!(completed.task.done);
    assert!(
        f.file("tasks/ana/inbox.md")
            .contains("  - [x] Outline {done=2026-11-03T10:00:00Z}")
    );
    let reopened = f
        .service
        .reopen(f.user, TaskRef { task: id.clone() })
        .await
        .unwrap();
    assert!(!reopened.done);

    let _ = f
        .service
        .delete(f.user, TaskRef { task: id.clone() })
        .await
        .unwrap();
    let inbox = f.file("tasks/ana/inbox.md");
    assert!(!inbox.contains("report") && !inbox.contains(&child.id[child.id.len() - 12..]));
    let gone = f.service.delete(f.user, TaskRef { task: id }).await;
    assert_eq!(gone.unwrap_err().kind(), ErrorKind::NotFound);
    let issue = f
        .service
        .reopen(
            f.user,
            TaskRef {
                task: "issue.x".into(),
            },
        )
        .await;
    assert_eq!(issue.unwrap_err().kind(), ErrorKind::InvalidInput);
}

#[tokio::test]
async fn lists_are_created_shared_and_imported_once() {
    let f = fixture();
    let _ = f.add("Eggs #Groceries").await;
    let _ = f.add("Inbox item").await;
    let lists = f.service.lists(f.user, NoInput {}).await.unwrap().lists;
    let titles: Vec<&str> = lists.iter().map(|l| l.title.as_str()).collect();
    assert_eq!(titles, ["Inbox", "Groceries"]);
    assert!(
        f.file("tasks/ana/groceries.md")
            .contains("- [ ] Eggs ^blk-")
    );

    let groceries = lists[1].doc;
    let shared = f
        .service
        .share_list(
            f.user,
            MemberInput {
                list_id: groceries,
                member: "@bo".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(shared.members, ["bo"]);
    assert_eq!(
        f.mem.state.lock().unwrap().members,
        [(groceries, "bo".to_owned())]
    );
    let unshared = f
        .service
        .unshare_list(
            f.user,
            MemberInput {
                list_id: groceries,
                member: "bo".into(),
            },
        )
        .await
        .unwrap();
    assert!(unshared.members.is_empty());

    let duplicate = f
        .service
        .create_list(
            f.user,
            CreateListInput {
                title: "groceries".into(),
            },
        )
        .await;
    assert_eq!(duplicate.unwrap_err().kind(), ErrorKind::Conflict);
    let work = f
        .service
        .create_list(
            f.user,
            CreateListInput {
                title: "Work".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(work.owner.as_deref(), Some("ana"));

    let export = r#"{"kind":"tasks#taskLists","items":[{"kind":"tasks#tasks","title":"Errands","items":[
      {"id":"a","title":"Post parcel","status":"needsAction","position":"000"},
      {"id":"b","title":"Stamps","status":"needsAction","parent":"a","position":"001"}]}]}"#;
    let input = || ImportInput {
        from: ImportFrom::GoogleTasks,
        content: export.into(),
        project: None,
    };
    let first = f.service.import(f.user, input()).await.unwrap();
    assert_eq!(first.lists[0].added, 2);
    let again = f.service.import(f.user, input()).await.unwrap();
    assert_eq!(again.lists[0].added, 0);
    assert_eq!(again.lists[0].doc, first.lists[0].doc);
    let bad = f
        .service
        .import(
            f.user,
            ImportInput {
                from: ImportFrom::Things,
                content: "nope".into(),
                project: None,
            },
        )
        .await;
    assert_eq!(bad.unwrap_err().kind(), ErrorKind::InvalidInput);
}

#[tokio::test]
async fn the_feed_needs_its_secret() {
    let f = fixture();
    let _ = f.add("Dentist friday 9am").await;
    let url = f
        .service
        .rotate_feed(f.user, NoInput {})
        .await
        .unwrap()
        .value;
    let path = url.strip_prefix("https://vellum.northstar.dev").unwrap();
    assert!(path.starts_with("/feeds/tasks/"));
    assert_eq!(path.rsplit_once('.').map(|(_, ext)| ext), Some("ics"));

    let response = f.raw("GET", path, &[], "").await;
    assert_eq!(response.status, 200);
    let text = body(&response);
    assert!(text.starts_with("BEGIN:VCALENDAR\r\n"));
    assert!(text.contains("SUMMARY:Dentist\r\n"));
    assert!(text.contains("DTSTART:20261106T090000Z\r\n"));
    assert_eq!(
        header(&response, "Content-Type"),
        Some("text/calendar; charset=utf-8")
    );

    let _ = f.service.rotate_feed(f.user, NoInput {}).await.unwrap();
    assert_eq!(f.raw("GET", path, &[], "").await.status, 404);
    assert_eq!(
        f.raw("GET", "/feeds/tasks/short.ics", &[], "").await.status,
        404
    );
    assert_eq!(f.raw("POST", path, &[], "").await.status, 405);
}

#[tokio::test]
async fn caldav_round_trip() {
    let f = fixture();
    let id = f.add("Buy milk").await;
    let inbox = id.split_once('.').unwrap().0.to_owned();
    let list_path = format!("/dav/calendars/ana/{inbox}/");

    let options = f.raw("OPTIONS", "/dav/", &[], "").await;
    assert_eq!(header(&options, "DAV"), Some("1, 3, calendar-access"));

    let home = f
        .raw("PROPFIND", "/dav/calendars/ana/", &[("Depth", "1")], "")
        .await;
    assert_eq!(home.status, 207);
    assert!(body(&home).contains(&list_path), "{}", body(&home));

    let other = f
        .raw("PROPFIND", "/dav/calendars/bo/", &[("Depth", "1")], "")
        .await;
    assert_eq!(other.status, 404);

    let listing = f.raw("PROPFIND", &list_path, &[("Depth", "1")], "").await;
    let uid = format!("{id}@vellum");
    assert!(body(&listing).contains(&uid));

    let object_path = format!("{list_path}{uid}.ics");
    let get = f.raw("GET", &object_path, &[], "").await;
    assert_eq!(get.status, 200);
    assert!(body(&get).contains("SUMMARY:Buy milk\r\n"));
    let etag = header(&get, "ETag").unwrap().to_owned();

    // A new task from the client keeps its UID.
    let todo = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Test//EN\r\nBEGIN:VTODO\r\nUID:abc-123\r\n\
DTSTAMP:20261103T100000Z\r\nSUMMARY:From the phone\r\nDUE;VALUE=DATE:20261110\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    let put = f
        .raw(
            "PUT",
            &format!("{list_path}abc-123.ics"),
            &[("If-None-Match", "*")],
            todo,
        )
        .await;
    assert_eq!(put.status, 201, "{}", body(&put));
    assert!(header(&put, "ETag").is_some());
    assert!(
        f.file("tasks/ana/inbox.md")
            .contains("- [ ] From the phone {due=2026-11-10 src=caldav:abc-123} ^blk-")
    );
    let again = f
        .raw(
            "PUT",
            &format!("{list_path}abc-123.ics"),
            &[("If-None-Match", "*")],
            todo,
        )
        .await;
    assert_eq!(again.status, 412);

    // Completing from the client ticks the item.
    let done = body(&get).replace(
        "STATUS:NEEDS-ACTION",
        "STATUS:COMPLETED\r\nCOMPLETED:20261103T093000Z",
    );
    let stale = f
        .raw("PUT", &object_path, &[("If-Match", "\"stale\"")], &done)
        .await;
    assert_eq!(stale.status, 412);
    let put = f
        .raw("PUT", &object_path, &[("If-Match", &etag)], &done)
        .await;
    assert_eq!(put.status, 204, "{}", body(&put));
    let inbox = f.file("tasks/ana/inbox.md");
    assert!(
        inbox.contains("- [x] Buy milk {done=2026-11-03T09:30:00Z} ^blk-"),
        "{inbox}"
    );

    let report = r#"<?xml version="1.0"?><D:sync-collection xmlns:D="DAV:"><D:sync-token/><D:prop><D:getetag/></D:prop></D:sync-collection>"#;
    let sync = f.raw("REPORT", &list_path, &[], report).await;
    assert_eq!(sync.status, 207);
    assert!(body(&sync).contains("abc-123.ics"));
    let stale_sync = report.replace(
        "<D:sync-token/>",
        "<D:sync-token>https://vellum.invalid/sync/x/1</D:sync-token>",
    );
    assert_eq!(
        f.raw("REPORT", &list_path, &[], &stale_sync).await.status,
        403
    );

    let delete = f
        .raw("DELETE", &format!("{list_path}abc-123.ics"), &[], "")
        .await;
    assert_eq!(delete.status, 204);
    assert!(!f.file("tasks/ana/inbox.md").contains("From the phone"));

    let anonymous = f
        .service
        .handle_raw(RawRequest {
            method: "PROPFIND".into(),
            path: "/dav/".into(),
            query: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
            principal: Principal::anonymous(),
        })
        .await;
    assert_eq!(anonymous.status, 401);
    assert!(header(&anonymous, "WWW-Authenticate").is_some());

    let read_only = f
        .service
        .handle_raw(RawRequest {
            method: "DELETE".into(),
            path: object_path,
            query: String::new(),
            headers: Vec::new(),
            body: Vec::new(),
            principal: f.principal(&[Scope::Read]),
        })
        .await;
    assert_eq!(read_only.status, 403);
}

#[tokio::test]
async fn email_capture_goes_to_the_inbox_once() {
    let f = fixture();
    let address = f
        .service
        .rotate_capture(f.user, NoInput {})
        .await
        .unwrap()
        .value;
    assert!(address.starts_with("tasks+") && address.ends_with("@northstar.dev"));
    let message = "From: Ann <ann@northstar.dev>\r\nTo: x\r\nSubject: Fwd: Renew domain friday\r\n\
Message-ID: <m1@northstar.dev>\r\nContent-Type: text/plain\r\n\r\nIt expires soon.\r\n";
    let post = f
        .raw(
            "POST",
            "/capture/email",
            &[(RECIPIENT_HEADER, &address)],
            message,
        )
        .await;
    assert_eq!(post.status, 202, "{}", body(&post));
    let repeat = f
        .raw(
            "POST",
            "/capture/email",
            &[(RECIPIENT_HEADER, &address)],
            message,
        )
        .await;
    assert_eq!(repeat.status, 202);
    let inbox = f.file("tasks/ana/inbox.md");
    assert_eq!(inbox.matches("Renew domain").count(), 1, "{inbox}");
    assert!(inbox.contains("due=2026-11-06"));

    let _ = f.service.revoke_capture(f.user, NoInput {}).await.unwrap();
    let revoked = f
        .raw(
            "POST",
            "/capture/email",
            &[(RECIPIENT_HEADER, &address)],
            message,
        )
        .await;
    assert_eq!(revoked.status, 404);
    assert_eq!(f.raw("POST", "/capture/slack", &[], "").await.status, 404);
    assert_eq!(f.raw("GET", "/capture/nowhere", &[], "").await.status, 404);
}

#[tokio::test]
async fn the_digest_and_reminders() {
    let f = fixture();
    let _ = f.add("Pay invoice today remind me 30 minutes before").await;
    let digest = f.service.digest(f.user, NoInput {}).await.unwrap();
    let text = digest.markdown.unwrap();
    assert!(
        text.starts_with("# My Work for Tuesday 3 November\n"),
        "{text}"
    );
    assert!(text.contains("Pay invoice"));

    let due = f
        .service
        .reminders_due(
            f.user,
            "2026-11-03T08:00:00Z".parse().unwrap(),
            "2026-11-03T09:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(due.len(), 1, "{}", f.file("tasks/ana/inbox.md"));
    assert_eq!(due[0].at.to_string(), "2026-11-03T08:30:00Z");
}

// --- Google Calendar --------------------------------------------------

/// A Google Calendar in memory: events keep their offset form, as the
/// real API returns them, and every change bumps the sync token.
#[derive(Default)]
struct FakeGoogle {
    state: Mutex<FakeCalendar>,
}

#[derive(Default)]
struct FakeCalendar {
    events: BTreeMap<String, serde_json::Value>,
    changed: Vec<(u64, String)>,
    clock: u64,
    next_id: u64,
    revoked: bool,
}

impl FakeCalendar {
    fn touch(&mut self, id: &str) {
        self.clock += 1;
        self.changed.push((self.clock, id.to_owned()));
    }

    fn offset_form(value: &mut serde_json::Value) {
        for key in ["start", "end"] {
            let Some(at) = value.get_mut(key) else {
                continue;
            };
            let zone = at["timeZone"].as_str().unwrap_or("UTC").to_owned();
            if let Some(local) = at["dateTime"].as_str()
                && let Ok(local) = local.parse::<jiff::civil::DateTime>()
            {
                let zoned = local.to_zoned(TimeZone::get(&zone).unwrap()).unwrap();
                at["dateTime"] = zoned.strftime("%Y-%m-%dT%H:%M:%S%:z").to_string().into();
            }
        }
    }

    fn merge(target: &mut serde_json::Value, patch: &serde_json::Value) {
        if let (Some(target), Some(patch)) = (target.as_object_mut(), patch.as_object()) {
            for (k, v) in patch {
                let _ = target.insert(k.clone(), v.clone());
            }
        }
    }
}

impl FakeGoogle {
    fn event(&self, id: &str) -> serde_json::Value {
        self.state.lock().unwrap().events[id].clone()
    }

    /// The person edits the calendar directly.
    fn user_edit(&self, id: &str, change: &serde_json::Value) {
        let mut state = self.state.lock().unwrap();
        let mut event = state
            .events
            .get(id)
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"id": id, "status": "confirmed"}));
        FakeCalendar::merge(&mut event, change);
        FakeCalendar::offset_form(&mut event);
        let _ = state.events.insert(id.to_owned(), event);
        state.touch(id);
    }
}

impl vellum_tasks::google::GoogleCalendar for FakeGoogle {
    fn exchange(
        &self,
        code: String,
        verifier: String,
        redirect: String,
    ) -> BoxFuture<'_, Result<String, Error>> {
        assert_eq!(code, "the-code");
        assert!(verifier.len() >= 43);
        assert_eq!(
            redirect,
            "https://vellum.northstar.dev/connect/google/callback"
        );
        ready(Ok("refresh-1".to_owned()))
    }

    fn create_calendar(
        &self,
        grant: String,
        name: String,
        zone: String,
    ) -> BoxFuture<'_, Result<String, Error>> {
        assert_eq!(
            (grant.as_str(), name.as_str(), zone.as_str()),
            ("refresh-1", "Vellum", "Europe/London")
        );
        ready(Ok("cal-1".to_owned()))
    }

    fn changes(
        &self,
        _grant: String,
        calendar: String,
        sync_token: Option<String>,
    ) -> BoxFuture<'_, Result<vellum_tasks::google::Changes, Error>> {
        assert_eq!(calendar, "cal-1");
        let state = self.state.lock().unwrap();
        let since: u64 = sync_token
            .as_deref()
            .and_then(|t| t.strip_prefix('t'))
            .and_then(|t| t.parse().ok())
            .unwrap_or(0);
        let mut ids: Vec<&String> = state
            .changed
            .iter()
            .filter(|(at, _)| *at > since)
            .map(|(_, id)| id)
            .collect();
        ids.dedup();
        let events = ids.iter().map(|id| state.events[*id].clone()).collect();
        ready(Ok(vellum_tasks::google::Changes::Page {
            events,
            next_sync_token: Some(format!("t{}", state.clock)),
        }))
    }

    fn insert(
        &self,
        _grant: String,
        _calendar: String,
        mut body: serde_json::Value,
    ) -> BoxFuture<'_, Result<String, Error>> {
        let mut state = self.state.lock().unwrap();
        state.next_id += 1;
        let id = format!("e{}", state.next_id);
        body["id"] = id.clone().into();
        body["status"] = "confirmed".into();
        FakeCalendar::offset_form(&mut body);
        let _ = state.events.insert(id.clone(), body);
        state.touch(&id);
        ready(Ok(id))
    }

    fn patch(
        &self,
        _grant: String,
        _calendar: String,
        event: String,
        body: serde_json::Value,
    ) -> BoxFuture<'_, Result<(), Error>> {
        let mut state = self.state.lock().unwrap();
        let Some(mut current) = state.events.get(&event).cloned() else {
            return ready(Err(Error::new(ErrorKind::NotFound, "no such event")));
        };
        FakeCalendar::merge(&mut current, &body);
        FakeCalendar::offset_form(&mut current);
        let _ = state.events.insert(event.clone(), current);
        state.touch(&event);
        ready(Ok(()))
    }

    fn delete(
        &self,
        _grant: String,
        _calendar: String,
        event: String,
    ) -> BoxFuture<'_, Result<(), Error>> {
        let mut state = self.state.lock().unwrap();
        if let Some(e) = state.events.get_mut(&event) {
            e["status"] = "cancelled".into();
            state.touch(&event);
        }
        ready(Ok(()))
    }

    fn revoke(&self, grant: String) -> BoxFuture<'_, Result<(), Error>> {
        assert_eq!(grant, "refresh-1");
        self.state.lock().unwrap().revoked = true;
        ready(Ok(()))
    }
}

#[derive(Default)]
struct MemLinks(Mutex<HashMap<UserId, vellum_tasks::google::GoogleLink>>);

impl vellum_tasks::google::GoogleLinks for MemLinks {
    fn link(
        &self,
        user: UserId,
    ) -> BoxFuture<'_, Result<Option<vellum_tasks::google::GoogleLink>, Error>> {
        ready(Ok(self.0.lock().unwrap().get(&user).cloned()))
    }

    fn set_link(
        &self,
        user: UserId,
        link: Option<vellum_tasks::google::GoogleLink>,
    ) -> BoxFuture<'_, Result<(), Error>> {
        let mut map = self.0.lock().unwrap();
        match link {
            Some(link) => {
                let _ = map.insert(user, link);
            }
            None => {
                let _ = map.remove(&user);
            }
        }
        ready(Ok(()))
    }

    fn linked(&self) -> BoxFuture<'_, Result<Vec<UserId>, Error>> {
        ready(Ok(self.0.lock().unwrap().keys().copied().collect()))
    }
}

fn query_value(url: &str, name: &str) -> String {
    url.split(['?', '&'])
        .find_map(|pair| pair.strip_prefix(&format!("{name}=")))
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn time_blocks_sync_with_google_calendar_both_ways() {
    let mut f = fixture();
    let google = Arc::new(FakeGoogle::default());
    let config = Config {
        base_url: "https://vellum.northstar.dev".into(),
        mail_domain: "northstar.dev".into(),
        slack_signing_secret: None,
    };
    let service = TasksService::new(f.mem.clone(), config)
        .with_clock(Arc::new(|| {
            "2026-11-03T10:00:00Z".parse::<Timestamp>().unwrap()
        }))
        .with_google(vellum_tasks::google::Google::new(
            google.clone(),
            Arc::new(MemLinks::default()),
            "client-1",
        ));
    f.service = Arc::new(service);
    let sync = || f.service.google_sync(f.user, NoInput {});

    // Connecting: consent, then the callback stores the link.
    let status = f.service.google_status(f.user, NoInput {}).await.unwrap();
    assert!(status.configured && !status.connected);
    let url = f
        .service
        .google_connect(f.user, NoInput {})
        .await
        .unwrap()
        .value;
    assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?client_id=client-1&"));
    assert!(url.contains("calendar.app.created") && url.contains("code_challenge_method=S256"));
    let state = query_value(&url, "state");
    let expired = f
        .raw(
            "GET",
            "/connect/google/callback?state=nope&code=the-code",
            &[],
            "",
        )
        .await;
    assert_eq!(
        header(&expired, "Location"),
        Some("https://vellum.northstar.dev/my-work?google=expired")
    );
    let back = f
        .raw(
            "GET",
            &format!("/connect/google/callback?state={state}&code=the-code"),
            &[],
            "",
        )
        .await;
    assert_eq!(back.status, 303);
    assert_eq!(
        header(&back, "Location"),
        Some("https://vellum.northstar.dev/my-work?google=connected")
    );
    assert!(
        f.service
            .google_status(f.user, NoInput {})
            .await
            .unwrap()
            .connected
    );
    // A consent link works once.
    let again = f
        .raw(
            "GET",
            &format!("/connect/google/callback?state={state}&code=the-code"),
            &[],
            "",
        )
        .await;
    assert_eq!(
        header(&again, "Location"),
        Some("https://vellum.northstar.dev/my-work?google=expired")
    );

    // A block on a task becomes an event.
    let id = f.add("Write the report").await;
    let _ = f
        .service
        .update(
            f.user,
            UpdateInput {
                task: id.clone(),
                block: Some("2026-11-03T14:00/90".into()),
                ..UpdateInput::default()
            },
        )
        .await
        .unwrap();
    let report = sync().await.unwrap();
    assert_eq!(report.created, 1, "{report:?}");
    assert!(
        f.file("tasks/ana/inbox.md")
            .contains("block=2026-11-03T14:00/90 gcal=e1")
    );
    let event = google.event("e1");
    assert_eq!(event["start"]["dateTime"], "2026-11-03T14:00:00+00:00");
    assert_eq!(
        event["extendedProperties"]["private"]["vellumTask"],
        id.as_str()
    );

    // Its own echo changes nothing.
    assert_eq!(
        sync().await.unwrap(),
        vellum_tasks::google::SyncView::default()
    );

    // Moving the event in Google moves the block.
    google.user_edit(
        "e1",
        &serde_json::json!({
            "start": {"dateTime": "2026-11-03T16:00:00", "timeZone": "Europe/London"},
            "end": {"dateTime": "2026-11-03T17:00:00", "timeZone": "Europe/London"},
        }),
    );
    assert_eq!(sync().await.unwrap().moved, 1);
    assert!(
        f.file("tasks/ana/inbox.md")
            .contains("block=2026-11-03T16:00/60 gcal=e1")
    );

    // Renaming the task updates the event.
    let _ = f
        .service
        .update(
            f.user,
            UpdateInput {
                task: id.clone(),
                title: Some("Write the final report".into()),
                ..UpdateInput::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(sync().await.unwrap().updated, 1);
    assert_eq!(google.event("e1")["summary"], "Write the final report");

    // A new event in the Vellum calendar becomes a task.
    google.user_edit(
        "g7",
        &serde_json::json!({
            "summary": "Dentist",
            "start": {"dateTime": "2026-11-04T09:00:00", "timeZone": "Europe/London"},
            "end": {"dateTime": "2026-11-04T09:45:00", "timeZone": "Europe/London"},
        }),
    );
    assert_eq!(sync().await.unwrap().adopted, 1);
    let inbox = f.file("tasks/ana/inbox.md");
    assert!(
        inbox.contains("- [ ] Dentist {block=2026-11-04T09:00/45 src=gcal:g7 gcal=g7}"),
        "{inbox}"
    );
    assert!(google.event("g7")["extendedProperties"]["private"]["vellumTask"].is_string());

    // Deleting the event in Google unblocks the task; it stays.
    google.user_edit("e1", &serde_json::json!({"status": "cancelled"}));
    assert_eq!(sync().await.unwrap().unblocked, 1);
    let inbox = f.file("tasks/ana/inbox.md");
    assert!(
        inbox.contains("- [ ] Write the final report ^blk-"),
        "{inbox}"
    );

    // Deleting the task deletes its event.
    let dentist = f
        .service
        .my_work(f.user, NoInput {})
        .await
        .unwrap()
        .sections
        .iter()
        .flat_map(|s| s.tasks.iter())
        .find(|t| t.title == "Dentist")
        .unwrap()
        .id
        .clone();
    let _ = f
        .service
        .delete(f.user, TaskRef { task: dentist })
        .await
        .unwrap();
    assert_eq!(sync().await.unwrap().deleted, 1);
    assert_eq!(google.event("g7")["status"], "cancelled");

    // Disconnecting revokes the grant.
    let _ = f
        .service
        .google_disconnect(f.user, NoInput {})
        .await
        .unwrap();
    assert!(google.state.lock().unwrap().revoked);
    assert!(
        !f.service
            .google_status(f.user, NoInput {})
            .await
            .unwrap()
            .connected
    );
    assert!(sync().await.is_err());
}

#[tokio::test]
async fn settings_move_today_into_the_persons_time_zone() {
    let f = fixture();
    let settings = f.service.settings(f.user, NoInput {}).await.unwrap();
    assert_eq!(settings.time_zone, None);
    assert_eq!(settings.effective_time_zone, "Europe/London");
    assert_eq!(settings.digest_at.as_deref(), Some("07:30"));
    let before = f.service.my_work(f.user, NoInput {}).await.unwrap();

    // Fourteen hours ahead of UTC, 10:00 UTC is already tomorrow.
    let input = MyWorkSettingsInput {
        time_zone: Some("Pacific/Kiritimati".into()),
        day_starts_at: "08:00".into(),
        digest_at: None,
    };
    let saved = f.service.set_settings(f.user, input).await.unwrap();
    assert_eq!(saved.effective_time_zone, "Pacific/Kiritimati");
    assert_eq!(saved.digest_at, None);
    let after = f.service.my_work(f.user, NoInput {}).await.unwrap();
    assert_ne!(
        before.today, after.today,
        "{} vs {}",
        before.today, after.today
    );
    assert_eq!(f.service.take_edited(), [f.user]);

    let bad = MyWorkSettingsInput {
        time_zone: Some("Atlantis/Lost".into()),
        day_starts_at: "08:00".into(),
        digest_at: None,
    };
    let err = f.service.set_settings(f.user, bad).await.unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

#[tokio::test]
async fn notices_carry_reminders_and_the_digest() {
    let f = fixture();
    let _ = f.service.take_edited();
    let _ = f.add("Pay invoice today remind me 30 minutes before").await;
    assert_eq!(f.service.take_edited(), [f.user]);
    assert!(f.service.take_edited().is_empty());

    // London, 07:00 to 09:00 UTC: the 07:30 digest and the 08:30 reminder.
    let notices = f
        .service
        .notices(
            f.user,
            "2026-11-03T07:00:00Z".parse().unwrap(),
            "2026-11-03T09:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
    let kinds: Vec<NoticeKind> = notices.iter().map(|n| n.kind).collect();
    assert_eq!(kinds, [NoticeKind::Digest, NoticeKind::Reminder]);
    assert_eq!(notices[0].at.to_string(), "2026-11-03T07:30:00Z");
    assert!(
        notices[0].body.starts_with("1 today"),
        "{}",
        notices[0].body
    );
    assert_eq!(notices[1].title, "Pay invoice");
    assert!(notices[1].url.starts_with("/p/"), "{}", notices[1].url);
    assert_ne!(notices[0].subject, notices[1].subject);

    // With the digest off, only the reminder is left.
    let input = MyWorkSettingsInput {
        time_zone: None,
        day_starts_at: "09:00".into(),
        digest_at: None,
    };
    let _ = f.service.set_settings(f.user, input).await.unwrap();
    let notices = f
        .service
        .notices(
            f.user,
            "2026-11-03T07:00:00Z".parse().unwrap(),
            "2026-11-03T09:00:00Z".parse().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
}

#[tokio::test]
async fn dismissing_a_mention_reaches_the_workspace() {
    let f = fixture();
    let comment = CommentId::generate();
    let _ = f
        .service
        .dismiss_mention(
            f.user,
            MentionRef {
                comment: comment.to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(f.mem.state.lock().unwrap().dismissed, [comment]);
    let err = f
        .service
        .dismiss_mention(
            f.user,
            MentionRef {
                comment: "not-an-id".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}
