// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The endpoints that are not JSON: the calendar feed, CalDAV and the
//! capture hooks (R-MW-6, R-MW-8).
//!
//! They are transport-neutral: the binary mounts [`TasksService::handle_raw`]
//! on the gateway under [`PREFIXES`], converting its HTTP request into a
//! [`RawRequest`]. The gateway's middleware still runs, and puts the
//! caller's [`Principal`] in the request (anonymous for the feed and the
//! capture hooks, which carry their own secret).

use std::collections::HashMap;

use vellum_contracts::{Principal, Scope};
use vellum_kernel::{BlockId, DocId, Error, ErrorKind, UserId};

use crate::caldav::{
    self, DavError, Depth, ListInfo, Multistatus, Object, Report, Resource,
};
use crate::capture;
use crate::checklist::{self, Checklist};
use crate::feed::{self, FeedOptions};
use crate::ical;
use crate::model::{Origin, Source, Task};
use crate::port::{Document, SecretKind};
use crate::projection;
use crate::service::{Me, TasksService};

/// The path prefixes [`TasksService::handle_raw`] serves.
pub const PREFIXES: [&str; 4] =
    [FEED_PREFIX, caldav::PREFIX, "/capture/", "/connect/"];

/// The calendar feed: `GET /feeds/tasks/<token>.ics`.
pub const FEED_PREFIX: &str = "/feeds/tasks/";

/// Inbound e-mail, posted by the mail relay with the envelope recipient
/// in [`RECIPIENT_HEADER`] and the raw message as the body.
pub const CAPTURE_EMAIL: &str = "/capture/email";

/// The Slack message shortcut's request URL.
pub const CAPTURE_SLACK: &str = "/capture/slack";

/// The header that carries an inbound message's envelope recipient.
pub const RECIPIENT_HEADER: &str = "x-vellum-recipient";

/// An HTTP request for a raw route.
#[derive(Debug, Clone)]
pub struct RawRequest {
    /// The method, in upper case.
    pub method: String,
    /// The path, without the query string.
    pub path: String,
    /// The query string, without `?`.
    pub query: String,
    /// Headers; names are matched without regard to case.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
    /// The caller, as the gateway authenticated them.
    pub principal: Principal,
}

impl RawRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// The response to a raw route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawResponse {
    /// The status code.
    pub status: u16,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

impl RawResponse {
    const fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub(crate) fn with_header(
        mut self,
        name: &str,
        value: impl Into<String>,
    ) -> Self {
        self.headers.push((name.to_owned(), value.into()));
        self
    }

    fn with_body(
        mut self,
        content_type: &str,
        body: impl Into<Vec<u8>>,
    ) -> Self {
        self.body = body.into();
        self.with_header("Content-Type", content_type)
    }

    /// A `303 See Other` to `location`.
    pub(crate) fn redirect(location: &str) -> Self {
        Self::new(303).with_header("Location", location)
    }

    pub(crate) fn text(status: u16, text: &str) -> Self {
        Self::new(status).with_body("text/plain; charset=utf-8", text)
    }

    fn from_error(err: &Error) -> Self {
        let status = match err.kind() {
            ErrorKind::InvalidInput => 400,
            ErrorKind::Unauthenticated => 401,
            ErrorKind::PermissionDenied => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::RateLimited => 429,
            ErrorKind::Unavailable => 503,
            ErrorKind::Timeout => 504,
            _ => 500,
        };
        if status >= 500 {
            tracing::warn!(error = %err, "tasks raw route failed");
        }
        Self::text(status, err.message())
    }

    fn from_dav(err: &DavError) -> Self {
        let response = Self::new(err.status());
        match err.body() {
            Some(body) => {
                response.with_body("application/xml; charset=utf-8", body)
            }
            None => {
                response.with_body("text/plain; charset=utf-8", err.to_string())
            }
        }
    }
}

const XML: &str = "application/xml; charset=utf-8";
const ICS: &str = "text/calendar; charset=utf-8";

/// A list file with its tasks as calendar objects.
struct DavList {
    doc: Document,
    info: ListInfo,
    tasks: Vec<Task>,
    objects: Vec<Object>,
}

impl DavList {
    fn new(doc: Document, me: &Me) -> Self {
        let title = doc
            .context
            .list
            .clone()
            .unwrap_or_else(|| crate::lists::read_meta(&doc.text).title);
        let info = ListInfo {
            id: doc.context.doc.to_string(),
            title,
            version: doc.version,
            color: None,
        };
        let tasks = projection::project_file(&doc.context, &doc.text);
        let uids: HashMap<BlockId, String> = tasks
            .iter()
            .filter_map(|t| match &t.source {
                Source::Checklist { block, .. } => Some((*block, ical::uid(t))),
                _ => None,
            })
            .collect();
        let objects = tasks
            .iter()
            .map(|task| {
                let parent =
                    task.parent.and_then(|p| uids.get(&p)).map(String::as_str);
                let todo = ical::to_vtodo(
                    task,
                    parent,
                    &me.profile.time_zone,
                    doc.modified,
                );
                Object {
                    uid: ical::uid(task),
                    ical: ical::calendar(None, vec![todo]).to_ical(),
                }
            })
            .collect();
        Self {
            doc,
            info,
            tasks,
            objects,
        }
    }

    fn find(&self, uid: &str) -> Option<(&Task, &Object)> {
        self.tasks
            .iter()
            .zip(&self.objects)
            .find(|(_, o)| o.uid == uid)
    }

    fn block_of(&self, uid: &str) -> Option<BlockId> {
        self.find(uid).and_then(|(t, _)| match &t.source {
            Source::Checklist { block, .. } => Some(*block),
            _ => None,
        })
    }
}

fn body_text(request: &RawRequest) -> Result<&str, DavError> {
    std::str::from_utf8(&request.body)
        .map_err(|_| DavError::BadRequest("the body is not UTF-8".into()))
}

fn multistatus(ms: Multistatus) -> RawResponse {
    RawResponse::new(207).with_body(XML, ms.finish())
}

impl TasksService {
    /// Serves a raw route: the feed, CalDAV or a capture hook.
    pub async fn handle_raw(&self, request: RawRequest) -> RawResponse {
        let path = request.path.clone();
        let result = if let Some(segment) = path.strip_prefix(FEED_PREFIX) {
            self.feed(&request, segment).await
        } else if path == caldav::PREFIX || path.starts_with("/dav/") {
            self.dav(&request).await
        } else if path == CAPTURE_EMAIL {
            self.capture_email(&request).await
        } else if path == CAPTURE_SLACK {
            self.capture_slack(&request).await
        } else if path == crate::google::CALLBACK {
            self.google_callback(&request).await
        } else {
            Ok(RawResponse::text(404, "not found"))
        };
        result.unwrap_or_else(|err| RawResponse::from_error(&err))
    }

    async fn feed(
        &self,
        request: &RawRequest,
        segment: &str,
    ) -> Result<RawResponse, Error> {
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            return Ok(RawResponse::text(405, "method not allowed")
                .with_header("Allow", "GET, HEAD"));
        }
        let Some(token) = feed::token_from_path(segment) else {
            return Ok(RawResponse::text(404, "not found"));
        };
        let Some(user) = self
            .workspace
            .user_for_secret(SecretKind::Feed, token.hash())
            .await?
        else {
            return Ok(RawResponse::text(404, "not found"));
        };
        let me = self.me(user).await?;
        let tasks = self.gather(user, &me.profile.handle).await?;
        let options = FeedOptions {
            name: format!("My Work: {}", me.profile.display_name),
            time_zone: me.profile.time_zone.clone(),
            base_url: self.config.base_url.trim_end_matches('/').to_owned(),
            today: me.today(),
            now: me.now.timestamp(),
        };
        let body = feed::render(&tasks, &options);
        let mut response = RawResponse::new(200)
            .with_body(feed::CONTENT_TYPE, body)
            .with_header("Cache-Control", "private, max-age=300")
            .with_header("Referrer-Policy", "no-referrer");
        if request.method == "HEAD" {
            response.body.clear();
        }
        Ok(response)
    }

    async fn dav(&self, request: &RawRequest) -> Result<RawResponse, Error> {
        let Some(user) = request.principal.user_id() else {
            return Ok(RawResponse::text(401, "sign in with an app password")
                .with_header(
                    "WWW-Authenticate",
                    "Basic realm=\"Vellum\", charset=\"UTF-8\"",
                ));
        };
        let writes = matches!(request.method.as_str(), "PUT" | "DELETE");
        request.principal.require_scope(if writes {
            Scope::WriteDirect
        } else {
            Scope::Read
        })?;
        if request.method == "OPTIONS" {
            return Ok(RawResponse::new(200)
                .with_header("Allow", caldav::ALLOW)
                .with_header("DAV", caldav::DAV_HEADER));
        }
        let Some(resource) = caldav::parse_path(&request.path) else {
            return Ok(RawResponse::from_dav(&DavError::NotFound));
        };
        let me = self.me(user).await?;
        let owner = match &resource {
            Resource::Root => None,
            Resource::Principal { user }
            | Resource::Home { user }
            | Resource::List { user, .. }
            | Resource::Object { user, .. } => Some(user.as_str()),
        };
        if owner.is_some_and(|o| o != me.profile.handle) {
            return Ok(RawResponse::from_dav(&DavError::NotFound));
        }
        let outcome = match request.method.as_str() {
            "PROPFIND" => self.propfind(request, &resource, user, &me).await,
            "REPORT" => self.report(request, &resource, user, &me).await,
            "GET" | "HEAD" => self.dav_get(request, &resource, user, &me).await,
            "PUT" => self.dav_put(request, &resource, user, &me).await,
            "DELETE" => self.dav_delete(request, &resource, user, &me).await,
            _ => Ok(Err(DavError::MethodNotAllowed)),
        }?;
        Ok(outcome.unwrap_or_else(|err| {
            let response = RawResponse::from_dav(&err);
            if err == DavError::MethodNotAllowed {
                response.with_header("Allow", caldav::ALLOW)
            } else {
                response
            }
        }))
    }

    async fn dav_lists(
        &self,
        user: UserId,
        me: &Me,
    ) -> Result<Vec<DavList>, Error> {
        let files = self.workspace.task_files(user).await?;
        Ok(files
            .into_iter()
            .filter(|f| {
                matches!(f.context.origin, Origin::Inbox | Origin::List)
            })
            .map(|f| DavList::new(f, me))
            .collect())
    }

    async fn dav_list(
        &self,
        user: UserId,
        me: &Me,
        list: &str,
    ) -> Result<Option<DavList>, Error> {
        let Ok(doc) = list.parse::<DocId>() else {
            return Ok(None);
        };
        Ok(self
            .dav_lists(user, me)
            .await?
            .into_iter()
            .find(|l| l.doc.context.doc == doc))
    }

    async fn propfind(
        &self,
        request: &RawRequest,
        resource: &Resource,
        user: UserId,
        me: &Me,
    ) -> Result<Result<RawResponse, DavError>, Error> {
        let props = match body_text(request).and_then(caldav::parse_propfind) {
            Ok(props) => props,
            Err(err) => return Ok(Err(err)),
        };
        let depth = Depth::from_header(request.header("Depth"));
        let handle = &me.profile.handle;
        let mut ms = Multistatus::new();
        match resource {
            Resource::Root | Resource::Principal { .. } => {
                ms.principal(
                    &caldav::href(resource),
                    handle,
                    &me.profile.display_name,
                    &props,
                );
            }
            Resource::Home { .. } => {
                ms.home(handle, &props);
                if depth == Depth::One {
                    for list in self.dav_lists(user, me).await? {
                        ms.list(handle, &list.info, &props);
                    }
                }
            }
            Resource::List { list, .. } => {
                let Some(list) = self.dav_list(user, me, list).await? else {
                    return Ok(Err(DavError::NotFound));
                };
                ms.list(handle, &list.info, &props);
                if depth == Depth::One {
                    for object in &list.objects {
                        ms.object(handle, &list.info.id, object, &props);
                    }
                }
            }
            Resource::Object { list, uid, .. } => {
                let Some(list) = self.dav_list(user, me, list).await? else {
                    return Ok(Err(DavError::NotFound));
                };
                let Some((_, object)) = list.find(uid) else {
                    return Ok(Err(DavError::NotFound));
                };
                ms.object(handle, &list.info.id, object, &props);
            }
        }
        Ok(Ok(multistatus(ms)))
    }

    async fn report(
        &self,
        request: &RawRequest,
        resource: &Resource,
        user: UserId,
        me: &Me,
    ) -> Result<Result<RawResponse, DavError>, Error> {
        let Resource::List { list, .. } = resource else {
            return Ok(Err(DavError::MethodNotAllowed));
        };
        let report = match body_text(request).and_then(caldav::parse_report) {
            Ok(report) => report,
            Err(err) => return Ok(Err(err)),
        };
        let Some(list) = self.dav_list(user, me, list).await? else {
            return Ok(Err(DavError::NotFound));
        };
        let handle = &me.profile.handle;
        let mut ms = Multistatus::new();
        match report {
            Report::Query { props, open_only } => {
                for object in list
                    .objects
                    .iter()
                    .filter(|o| !open_only || caldav::is_open(o))
                {
                    ms.object(handle, &list.info.id, object, &props);
                }
            }
            Report::Multiget { props, hrefs } => {
                for href in hrefs {
                    let found = match caldav::parse_path(&href) {
                        Some(Resource::Object {
                            user: owner,
                            list: id,
                            uid,
                        }) if owner == *handle && id == list.info.id => {
                            list.find(&uid)
                        }
                        _ => None,
                    };
                    match found {
                        Some((_, object)) => {
                            ms.object(handle, &list.info.id, object, &props)
                        }
                        None => ms.not_found(&href),
                    }
                }
            }
            Report::Sync { props, token } => {
                match caldav::check_sync_token(&list.info, token.as_deref()) {
                    Ok(true) => {}
                    Ok(false) => {
                        for object in &list.objects {
                            ms.object(handle, &list.info.id, object, &props);
                        }
                    }
                    Err(err) => return Ok(Err(err)),
                }
                ms.sync_token(&list.info);
            }
        }
        Ok(Ok(multistatus(ms)))
    }

    async fn dav_get(
        &self,
        request: &RawRequest,
        resource: &Resource,
        user: UserId,
        me: &Me,
    ) -> Result<Result<RawResponse, DavError>, Error> {
        let Resource::Object { list, uid, .. } = resource else {
            return Ok(Err(DavError::MethodNotAllowed));
        };
        let Some(list) = self.dav_list(user, me, list).await? else {
            return Ok(Err(DavError::NotFound));
        };
        let Some((_, object)) = list.find(uid) else {
            return Ok(Err(DavError::NotFound));
        };
        let mut response = RawResponse::new(200)
            .with_body(ICS, object.ical.clone())
            .with_header("ETag", caldav::etag(&object.ical));
        if request.method == "HEAD" {
            response.body.clear();
        }
        Ok(Ok(response))
    }

    async fn dav_put(
        &self,
        request: &RawRequest,
        resource: &Resource,
        user: UserId,
        me: &Me,
    ) -> Result<Result<RawResponse, DavError>, Error> {
        let Resource::Object { list, uid, .. } = resource else {
            return Ok(Err(DavError::MethodNotAllowed));
        };
        let Some(list) = self.dav_list(user, me, list).await? else {
            return Ok(Err(DavError::NotFound));
        };
        let text = match body_text(request) {
            Ok(text) => text,
            Err(err) => return Ok(Err(err)),
        };
        let now = me.now.timestamp();
        let todo = match ical::from_vtodo(text, &me.profile.time_zone, now) {
            Ok(todo) => todo,
            Err(err) => {
                return Ok(Err(DavError::UnsupportedData(err.to_string())));
            }
        };
        let current = list.find(uid).map(|(_, o)| caldav::etag(&o.ical));
        if let Err(err) = caldav::check_preconditions(
            current.as_deref(),
            request.header("If-Match"),
            request.header("If-None-Match"),
        ) {
            return Ok(Err(err));
        }
        let existing = list.block_of(uid);
        let parent = todo.parent_uid.as_deref().and_then(|p| list.block_of(p));
        let block = existing.unwrap_or_else(BlockId::generate);
        let doc = list.doc.context.doc;
        let zoned = me.now.clone();
        let created = existing.is_none();
        let (written, ()) = self
            .edit(user, doc, |file| {
                let edited = if created {
                    let mut fields = todo.fields.clone();
                    if !todo.uid.ends_with("@vellum") {
                        fields.src =
                            Some(format!("{}{}", ical::CALDAV_SRC, todo.uid));
                    }
                    if todo.done && fields.done.is_none() {
                        fields.done = Some(now);
                    }
                    checklist::append(
                        &file.text,
                        parent,
                        &todo.title,
                        &fields,
                        block,
                    )
                } else {
                    put_existing(&file.text, block, &todo, &zoned)
                };
                edited.map(|text| (text, ())).map_err(|_| {
                    Error::new(ErrorKind::NotFound, "the task is gone")
                })
            })
            .await?;
        let list = DavList::new(written, me);
        let etag = list
            .objects
            .iter()
            .zip(&list.tasks)
            .find(|(_, t)| {
                matches!(
                    &t.source,
                    Source::Checklist { block: b, .. } if *b == block
                )
            })
            .map(|(o, _)| caldav::etag(&o.ical));
        let mut response = RawResponse::new(if created { 201 } else { 204 });
        if let Some(etag) = etag {
            response = response.with_header("ETag", etag);
        }
        Ok(Ok(response))
    }

    async fn dav_delete(
        &self,
        request: &RawRequest,
        resource: &Resource,
        user: UserId,
        me: &Me,
    ) -> Result<Result<RawResponse, DavError>, Error> {
        let Resource::Object { list, uid, .. } = resource else {
            return Ok(Err(DavError::MethodNotAllowed));
        };
        let Some(list) = self.dav_list(user, me, list).await? else {
            return Ok(Err(DavError::NotFound));
        };
        let Some((_, object)) = list.find(uid) else {
            return Ok(Err(DavError::NotFound));
        };
        if let Err(err) = caldav::check_preconditions(
            Some(&caldav::etag(&object.ical)),
            request.header("If-Match"),
            None,
        ) {
            return Ok(Err(err));
        }
        let Some(block) = list.block_of(uid) else {
            return Ok(Err(DavError::NotFound));
        };
        let _ = self
            .edit(user, list.doc.context.doc, |file| {
                checklist::remove(&file.text, block)
                    .map(|text| (text, ()))
                    .map_err(|_| {
                        Error::new(ErrorKind::NotFound, "the task is gone")
                    })
            })
            .await?;
        Ok(Ok(RawResponse::new(204)))
    }

    async fn capture_email(
        &self,
        request: &RawRequest,
    ) -> Result<RawResponse, Error> {
        if request.method != "POST" {
            return Ok(RawResponse::text(405, "method not allowed")
                .with_header("Allow", "POST"));
        }
        let Some(token) = request
            .header(RECIPIENT_HEADER)
            .and_then(|r| capture::token_from_address(r).ok())
        else {
            return Ok(RawResponse::text(404, "unknown address"));
        };
        let Some(user) = self
            .workspace
            .user_for_secret(SecretKind::Capture, token.hash())
            .await?
        else {
            return Ok(RawResponse::text(404, "unknown address"));
        };
        let me = self.me(user).await?;
        let Ok(task) = capture::from_email(&request.body, me.now.datetime())
        else {
            return Ok(RawResponse::text(400, "cannot read the message"));
        };
        self.add_capture(user, &me, &task).await?;
        Ok(RawResponse::new(202))
    }

    async fn capture_slack(
        &self,
        request: &RawRequest,
    ) -> Result<RawResponse, Error> {
        if request.method != "POST" {
            return Ok(RawResponse::text(405, "method not allowed")
                .with_header("Allow", "POST"));
        }
        let Some(secret) = &self.config.slack_signing_secret else {
            return Ok(RawResponse::text(404, "Slack capture is not set up"));
        };
        let verified = capture::verify_slack(
            secret,
            request
                .header("X-Slack-Request-Timestamp")
                .unwrap_or_default(),
            request.header("X-Slack-Signature").unwrap_or_default(),
            &request.body,
            self.now(),
        );
        if verified.is_err() {
            return Ok(RawResponse::text(401, "invalid signature"));
        }
        let Ok(body) = std::str::from_utf8(&request.body) else {
            return Ok(RawResponse::text(400, "the body is not UTF-8"));
        };
        let saved = match capture::from_slack(body) {
            Ok(saved) => saved,
            Err(err) => return Ok(RawResponse::text(400, &err.to_string())),
        };
        let Some(user) = self
            .workspace
            .slack_user(saved.team.clone(), saved.slack_user.clone())
            .await?
        else {
            // Not linked yet: answer with a link that, opened while signed
            // in to Vellum, links this Slack user (#163, D177).
            let code = capture::slack_link_code(
                secret,
                &saved.team,
                &saved.slack_user,
                self.now(),
            );
            let text = format!(
                "Link your Slack account to Vellum to save messages as tasks: \
                    {}/my-work?slack_link={code} (the link works for 15 \
                    minutes). Then use Save as task again.",
                self.config.base_url
            );
            return Ok(self.slack_reply(saved.response_url, text).await);
        };
        let me = self.me(user).await?;
        self.add_capture(user, &me, &saved.capture).await?;
        let text =
            format!("Saved to your Vellum inbox: {}", saved.capture.title);
        Ok(self.slack_reply(saved.response_url, text).await)
    }

    /// Tells the Slack user `text`: through `response_url` (Slack ignores
    /// the body of a message shortcut's answer) and in the body (which a
    /// slash command shows).
    async fn slack_reply(
        &self,
        response_url: Option<String>,
        text: String,
    ) -> RawResponse {
        if let Some(url) = response_url {
            self.workspace.slack_say(url, text.clone()).await;
        }
        let body =
            serde_json::json!({ "response_type": "ephemeral", "text": text })
                .to_string();
        RawResponse::new(200).with_body("application/json", body)
    }

    async fn add_capture(
        &self,
        user: UserId,
        me: &Me,
        task: &capture::Capture,
    ) -> Result<(), Error> {
        let inbox = self.inbox(user, &me.profile.handle).await?;
        let _ = self
            .edit(user, inbox, |file| {
                Ok((
                    capture::append_to_inbox(&file.text, task)
                        .map_or_else(|| file.text.clone(), |(text, _)| text),
                    (),
                ))
            })
            .await?;
        Ok(())
    }
}

/// Applies a CalDAV client's version of an existing task.
fn put_existing(
    text: &str,
    block: BlockId,
    todo: &ical::TodoUpdate,
    now: &jiff::Zoned,
) -> Result<String, checklist::EditError> {
    let list = Checklist::parse(text);
    let item = list
        .find(block)
        .ok_or(checklist::EditError::NotFound(block))?;
    let completing = todo.done && !item.checked;
    let mut fields = todo.fields.clone();
    fields.src.clone_from(&item.fields.src);
    if completing && item.fields.recurrence.is_some() {
        // Ticking a repeating task moves it on, as in the web app.
        fields.done = None;
        let text = checklist::update(text, block, &todo.title, &fields)?;
        return checklist::complete(&text, block, now, false)
            .map(|(text, _)| text);
    }
    if !todo.done {
        fields.done = None;
    }
    let text = checklist::update(text, block, &todo.title, &fields)?;
    if todo.done == item.checked {
        return Ok(text);
    }
    checklist::set_checked(
        &text,
        block,
        todo.done,
        fields.done.unwrap_or_else(|| now.timestamp()),
    )
}
