// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An in-process fake of the Microsoft Graph driveItem API, enough for the
//! `OneDrive` adapter: path and id addressing, `If-Match`, conflict
//! behaviour, paged children, upload sessions and pre-authenticated
//! download and upload URLs.
//!
//! Every request is handled under one lock, so precondition checks and the
//! writes they guard are atomic, as Graph promises.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use axum::{
    Router,
    body::Body,
    extract::State as Extract,
    http::{
        HeaderMap, Method, StatusCode, Uri,
        header::{AUTHORIZATION, CONTENT_RANGE, IF_MATCH, LOCATION},
    },
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use serde_json::{Map, Value, json};
use vellum_kernel::Timestamp;
use vellum_store::remote::{
    TokenSource,
    onedrive::{Drive, OneDriveConfig},
};

/// The folder inside the fake drive that tests use as the store root.
pub const ROOT: &str = "Apps/Vellum";

/// Knobs for one fake.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Children per listing page.
    pub page_size: usize,
    /// Largest simple upload the fake accepts (`413` above it).
    pub simple_upload_max: usize,
    /// Chunk size the adapter is configured with.
    pub chunk_size: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            page_size: 1,
            simple_upload_max: 4 * 1024 * 1024,
            chunk_size: 320 * 1024,
        }
    }
}

/// What the fake saw, for assertions.
#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    /// Graph requests refused with `401`.
    pub unauthorized: u32,
    /// Requests to a pre-authenticated URL that carried `Authorization`.
    pub leaked_auth: u32,
    /// Simple uploads accepted.
    pub simple_uploads: u32,
    /// Upload sessions created.
    pub sessions: u32,
    /// Upload sessions whose last chunk wrote the file.
    pub session_commits: u32,
    /// Upload-session chunks accepted.
    pub chunks: u32,
    /// Children pages served after the first (via `$skiptoken`).
    pub next_pages: u32,
}

/// A running fake Graph server.
pub struct FakeGraph {
    /// `http://127.0.0.1:{port}/v1.0`.
    pub base: String,
    limits: Limits,
    state: Arc<Mutex<GraphDrive>>,
}

impl FakeGraph {
    /// Starts a fake on an ephemeral port that accepts the token `"t1"`.
    ///
    /// # Errors
    /// Returns the I/O error if the listener cannot bind.
    pub async fn start(limits: Limits) -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let state = Arc::new(Mutex::new(GraphDrive::new(limits)));
        let app = App {
            origin: origin.clone(),
            state: Arc::clone(&state),
        };
        let router = Router::new().fallback(handle).with_state(app);
        drop(tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        }));
        Ok(Self {
            base: format!("{origin}/v1.0"),
            limits,
            state,
        })
    }

    fn lock(&self) -> MutexGuard<'_, GraphDrive> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adapter settings pointing at this fake, rooted at [`ROOT`].
    pub fn config(&self, tokens: Arc<dyn TokenSource>) -> OneDriveConfig {
        let mut config = OneDriveConfig::new(Drive::Me, ROOT, tokens);
        config.base_url.clone_from(&self.base);
        config.simple_upload_max = self.limits.simple_upload_max;
        config.chunk_size = self.limits.chunk_size;
        config
    }

    /// Accept only `token` from now on.
    pub fn accept_token(&self, token: &str) {
        token.clone_into(&mut self.lock().token);
    }

    /// A snapshot of the counters.
    pub fn counters(&self) -> Counters {
        self.lock().counters
    }

    /// Places a file at `drive_path` directly, bypassing the API.
    pub fn insert(&self, drive_path: &str, bytes: &'static [u8]) {
        let mut drive = self.lock();
        let _ = drive.ensure_parents(drive_path);
        let _ = drive.write(drive_path, Bytes::from_static(bytes));
    }
}

#[derive(Clone)]
struct App {
    origin: String,
    state: Arc<Mutex<GraphDrive>>,
}

/// One drive: driveItems keyed by their path from the drive root (`""` is
/// the root folder).
struct GraphDrive {
    token: String,
    limits: Limits,
    items: BTreeMap<String, Node>,
    sessions: HashMap<String, Session>,
    next_id: u64,
    counters: Counters,
}

struct Node {
    id: String,
    /// `None` for folders.
    content: Option<Bytes>,
    /// Bumped on every change, content or metadata: the `eTag`.
    version: u64,
    /// Bumped on content changes only: the `cTag`.
    content_version: u64,
    modified: String,
}

struct Session {
    path: String,
    if_match: Option<String>,
    fail_if_exists: bool,
    received: Vec<u8>,
}

/// A Graph error: the status and the `error.code` of the JSON body.
struct Fail(StatusCode, &'static str);

impl IntoResponse for Fail {
    fn into_response(self) -> Response {
        let Self(status, code) = self;
        let body = json!({
            "error": { "code": code, "message": format!("fake graph: {code}") }
        });
        (status, axum::Json(body)).into_response()
    }
}

type Reply = Result<Response, Fail>;

const fn error(status: StatusCode, code: &'static str) -> Fail {
    Fail(status, code)
}

const fn not_found() -> Fail {
    error(StatusCode::NOT_FOUND, "itemNotFound")
}

const fn bad_request() -> Fail {
    error(StatusCode::BAD_REQUEST, "invalidRequest")
}

fn now() -> String {
    Timestamp::now().to_string()
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn name_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn child_path(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

impl Node {
    fn etag(&self) -> String {
        format!("\"{{{}}},{}\"", self.id, self.version)
    }
}

impl GraphDrive {
    fn new(limits: Limits) -> Self {
        let mut items = BTreeMap::new();
        let _ = items.insert(
            String::new(),
            Node {
                id: "ROOT".to_owned(),
                content: None,
                version: 1,
                content_version: 1,
                modified: now(),
            },
        );
        Self {
            token: "t1".to_owned(),
            limits,
            items,
            sessions: HashMap::new(),
            next_id: 0,
            counters: Counters::default(),
        }
    }

    fn new_id(&mut self) -> String {
        self.next_id += 1;
        format!("ITEM{:04}", self.next_id)
    }

    fn path_of(&self, id: &str) -> Option<String> {
        self.items
            .iter()
            .find(|(_, n)| n.id == id)
            .map(|(p, _)| p.clone())
    }

    fn is_folder(&self, path: &str) -> bool {
        self.items.get(path).is_some_and(|n| n.content.is_none())
    }

    /// Creates missing ancestor folders, as path-based upload does.
    fn ensure_parents(&mut self, path: &str) -> Result<(), Fail> {
        let mut dir = String::new();
        let parent = parent_of(path);
        if parent.is_empty() {
            return Ok(());
        }
        for segment in parent.split('/') {
            dir = child_path(&dir, segment);
            match self.items.get(&dir) {
                Some(n) if n.content.is_some() => {
                    return Err(error(
                        StatusCode::CONFLICT,
                        "nameAlreadyExists",
                    ));
                }
                Some(_) => {}
                None => {
                    let id = self.new_id();
                    let _ = self.items.insert(
                        dir.clone(),
                        Node {
                            id,
                            content: None,
                            version: 1,
                            content_version: 1,
                            modified: now(),
                        },
                    );
                }
            }
        }
        Ok(())
    }

    /// Writes content, returning whether the file was created.
    fn write(&mut self, path: &str, bytes: Bytes) -> bool {
        if let Some(node) = self.items.get_mut(path) {
            node.content = Some(bytes);
            node.version += 1;
            node.content_version += 1;
            node.modified = now();
            return false;
        }
        let id = self.new_id();
        let _ = self.items.insert(
            path.to_owned(),
            Node {
                id,
                content: Some(bytes),
                version: 1,
                content_version: 1,
                modified: now(),
            },
        );
        true
    }

    /// `412` unless the item exists with the expected `eTag`.
    fn check_if_match(
        &self,
        path: &str,
        want: Option<&str>,
    ) -> Result<(), Fail> {
        let Some(want) = want else { return Ok(()) };
        match self.items.get(path) {
            Some(n) if want == "*" || n.etag() == want => Ok(()),
            _ => {
                Err(error(StatusCode::PRECONDITION_FAILED, "resourceModified"))
            }
        }
    }

    fn item_json(&self, origin: &str, path: &str, download: bool) -> Value {
        let Some(node) = self.items.get(path) else {
            return Value::Null;
        };
        let parent = parent_of(path);
        let parent_id = self.items.get(parent).map(|n| n.id.clone());
        let mut item = Map::new();
        let mut set =
            |key: &str, value: Value| drop(item.insert(key.to_owned(), value));
        set("id", json!(node.id));
        set(
            "name",
            json!(if path.is_empty() {
                "root"
            } else {
                name_of(path)
            }),
        );
        set("eTag", json!(node.etag()));
        set("lastModifiedDateTime", json!(node.modified));
        set(
            "parentReference",
            json!({
                "driveId": "fake-drive",
                "id": parent_id,
                "path": format!("/drive/root:/{parent}"),
            }),
        );
        if let Some(bytes) = &node.content {
            let ctag =
                format!("\"c:{{{}}},{}\"", node.id, node.content_version);
            set("cTag", json!(ctag));
            set("size", json!(bytes.len()));
            set("file", json!({ "mimeType": "application/octet-stream" }));
            if download {
                let url =
                    format!("{origin}/download/{}?tempauth=secret", node.id);
                set("@microsoft.graph.downloadUrl", json!(url));
            }
        } else {
            set("size", json!(0));
            set("folder", json!({ "childCount": self.children(path).len() }));
        }
        Value::Object(item)
    }

    fn children(&self, dir: &str) -> Vec<String> {
        self.items
            .keys()
            .filter(|p| !p.is_empty() && parent_of(p) == dir)
            .cloned()
            .collect()
    }

    fn ok_item(
        &self,
        status: StatusCode,
        origin: &str,
        path: &str,
    ) -> Response {
        (status, axum::Json(self.item_json(origin, path, false)))
            .into_response()
    }
}

async fn handle(
    Extract(app): Extract<App>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut drive = app.state.lock().unwrap_or_else(PoisonError::into_inner);
    dispatch(&mut drive, &app.origin, &method, &uri, &headers, body)
        .unwrap_or_else(IntoResponse::into_response)
}

fn dispatch(
    drive: &mut GraphDrive,
    origin: &str,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
) -> Reply {
    let query = parse_query(uri.query());
    if let Some(rest) = uri.path().strip_prefix("/download/") {
        return download(drive, rest, headers);
    }
    if let Some(rest) = uri.path().strip_prefix("/upload/") {
        return upload_chunk(drive, origin, method, rest, headers, &body);
    }
    let bearer = format!("Bearer {}", drive.token);
    if headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok())
        != Some(bearer.as_str())
    {
        drive.counters.unauthorized += 1;
        return Err(error(
            StatusCode::UNAUTHORIZED,
            "InvalidAuthenticationToken",
        ));
    }
    let rest = uri
        .path()
        .strip_prefix("/v1.0/me/drive")
        .ok_or_else(bad_request)?;
    let (path, op) = address(drive, rest)?;
    let if_match = headers.get(IF_MATCH).and_then(|v| v.to_str().ok());
    let behavior = query
        .get("@microsoft.graph.conflictBehavior")
        .map(String::as_str);
    match (method.as_str(), op.as_str()) {
        ("GET", "") => {
            if !drive.items.contains_key(&path) {
                return Err(not_found());
            }
            Ok(
                axum::Json(drive.item_json(origin, &path, true))
                    .into_response(),
            )
        }
        ("GET", "content") => {
            match drive.items.get(&path) {
                Some(Node {
                    id,
                    content: Some(_),
                    ..
                }) => Ok((
                    StatusCode::FOUND,
                    [(
                        LOCATION,
                        format!("{origin}/download/{id}?tempauth=secret"),
                    )],
                )
                    .into_response()),
                _ => Err(not_found()),
            }
        }
        ("GET", "children") => list_children(drive, origin, &path, &query),
        ("PUT", "content") => {
            simple_upload(drive, origin, &path, if_match, behavior, body)
        }
        ("POST", "createUploadSession") => {
            create_session(drive, origin, path, if_match, &body)
        }
        ("POST", "children") => create_folder(drive, origin, &path, &body),
        ("DELETE", "") => {
            if !drive.items.contains_key(&path) || path.is_empty() {
                return Err(not_found());
            }
            drive.check_if_match(&path, if_match)?;
            let prefix = format!("{path}/");
            drive
                .items
                .retain(|p, _| p != &path && !p.starts_with(&prefix));
            Ok(StatusCode::NO_CONTENT.into_response())
        }
        ("PATCH", "") => move_item(drive, origin, &path, behavior, &body),
        _ => Err(error(StatusCode::METHOD_NOT_ALLOWED, "invalidRequest")),
    }
}

fn simple_upload(
    drive: &mut GraphDrive,
    origin: &str,
    path: &str,
    if_match: Option<&str>,
    behavior: Option<&str>,
    body: Bytes,
) -> Reply {
    if body.len() > drive.limits.simple_upload_max {
        return Err(error(StatusCode::PAYLOAD_TOO_LARGE, "requestTooLarge"));
    }
    drive.check_if_match(path, if_match)?;
    if behavior == Some("fail") && drive.items.contains_key(path)
        || drive.is_folder(path)
    {
        return Err(error(StatusCode::CONFLICT, "nameAlreadyExists"));
    }
    drive.ensure_parents(path)?;
    let created = drive.write(path, body);
    drive.counters.simple_uploads += 1;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok(drive.ok_item(status, origin, path))
}

fn create_session(
    drive: &mut GraphDrive,
    origin: &str,
    path: String,
    if_match: Option<&str>,
    body: &Bytes,
) -> Reply {
    let request: Value =
        serde_json::from_slice(body).map_err(|_| bad_request())?;
    let fail = request
        .pointer("/item/@microsoft.graph.conflictBehavior")
        .and_then(Value::as_str)
        == Some("fail");
    drive.check_if_match(&path, if_match)?;
    if fail && drive.items.contains_key(&path) {
        return Err(error(StatusCode::CONFLICT, "nameAlreadyExists"));
    }
    let sid = drive.new_id();
    let _ = drive.sessions.insert(
        sid.clone(),
        Session {
            path,
            if_match: if_match.map(str::to_owned),
            fail_if_exists: fail,
            received: Vec::new(),
        },
    );
    drive.counters.sessions += 1;
    Ok(axum::Json(json!({
        "uploadUrl": format!("{origin}/upload/{sid}?tempauth=secret"),
        "expirationDateTime": now(),
    }))
    .into_response())
}

/// Resolves `/root`, `/root:/{path}:/{op}` and `/items/{id}/{op}` to a
/// drive path and an operation.
fn address(drive: &GraphDrive, rest: &str) -> Result<(String, String), Fail> {
    let bad = bad_request;
    if let Some(after) = rest.strip_prefix("/root:/") {
        let (encoded, op) = after.split_once(':').ok_or_else(bad)?;
        let decoded: Option<Vec<String>> =
            encoded.split('/').map(decode).collect();
        let path = decoded.ok_or_else(bad)?.join("/");
        return Ok((path, op.trim_start_matches('/').to_owned()));
    }
    if let Some(op) = rest.strip_prefix("/root") {
        return Ok((String::new(), op.trim_start_matches('/').to_owned()));
    }
    if let Some(after) = rest.strip_prefix("/items/") {
        let (id, op) = after.split_once('/').unwrap_or((after, ""));
        let id = decode(id).ok_or_else(bad)?;
        let path = drive.path_of(&id).ok_or_else(not_found)?;
        return Ok((path, op.to_owned()));
    }
    Err(bad())
}

fn list_children(
    drive: &mut GraphDrive,
    origin: &str,
    path: &str,
    query: &HashMap<String, String>,
) -> Reply {
    if !drive.is_folder(path) {
        return Err(not_found());
    }
    let skip: usize = match query.get("$skiptoken") {
        Some(token) => {
            drive.counters.next_pages += 1;
            token.parse().map_err(|_| bad_request())?
        }
        None => 0,
    };
    let all = drive.children(path);
    let take = drive.limits.page_size.max(1);
    let value: Vec<Value> = all
        .iter()
        .skip(skip)
        .take(take)
        .map(|p| drive.item_json(origin, p, true))
        .collect();
    let mut page = Map::new();
    let _ = page.insert("value".to_owned(), Value::Array(value));
    if skip + take < all.len() {
        let id = drive
            .items
            .get(path)
            .map(|n| n.id.clone())
            .unwrap_or_default();
        let next = format!(
            "{origin}/v1.0/me/drive/items/{id}/children?$skiptoken={}",
            skip + take
        );
        let _ = page.insert("@odata.nextLink".to_owned(), json!(next));
    }
    Ok(axum::Json(Value::Object(page)).into_response())
}

fn create_folder(
    drive: &mut GraphDrive,
    origin: &str,
    parent: &str,
    body: &Bytes,
) -> Reply {
    let request: Value =
        serde_json::from_slice(body).map_err(|_| bad_request())?;
    let name = request
        .get("name")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty() && !n.contains('/'))
        .ok_or_else(bad_request)?;
    if !drive.is_folder(parent) || request.get("folder").is_none() {
        return Err(bad_request());
    }
    let path = child_path(parent, name);
    if drive.items.contains_key(&path) {
        return Err(error(StatusCode::CONFLICT, "nameAlreadyExists"));
    }
    let id = drive.new_id();
    let _ = drive.items.insert(
        path.clone(),
        Node {
            id,
            content: None,
            version: 1,
            content_version: 1,
            modified: now(),
        },
    );
    Ok(drive.ok_item(StatusCode::CREATED, origin, &path))
}

fn move_item(
    drive: &mut GraphDrive,
    origin: &str,
    from: &str,
    behavior: Option<&str>,
    body: &Bytes,
) -> Reply {
    let request: Value =
        serde_json::from_slice(body).map_err(|_| bad_request())?;
    if !drive.items.contains_key(from) {
        return Err(not_found());
    }
    if drive.is_folder(from) {
        return Err(error(StatusCode::NOT_IMPLEMENTED, "notSupported")); // Not needed here.
    }
    let parent = match request
        .pointer("/parentReference/id")
        .and_then(Value::as_str)
    {
        Some(id) => drive.path_of(id).ok_or_else(not_found)?,
        None => parent_of(from).to_owned(),
    };
    if !drive.is_folder(&parent) {
        return Err(bad_request());
    }
    let name = request
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| name_of(from));
    let to = child_path(&parent, name);
    if to == from {
        return Ok(drive.ok_item(StatusCode::OK, origin, &to));
    }
    // Moves default to `fail`; only an explicit `replace` overwrites.
    if drive.items.contains_key(&to) && behavior != Some("replace") {
        return Err(error(StatusCode::CONFLICT, "nameAlreadyExists"));
    }
    let mut node = drive.items.remove(from).ok_or_else(not_found)?;
    node.version += 1; // eTag changes on a move; cTag does not.
    let _ = drive.items.insert(to.clone(), node);
    Ok(drive.ok_item(StatusCode::OK, origin, &to))
}

/// The pre-authenticated download host: it must never see a bearer token.
fn download(drive: &mut GraphDrive, rest: &str, headers: &HeaderMap) -> Reply {
    if headers.contains_key(AUTHORIZATION) {
        drive.counters.leaked_auth += 1;
        return Err(error(StatusCode::UNAUTHORIZED, "unauthenticated"));
    }
    let id = rest.split('?').next().unwrap_or_default();
    let path = drive.path_of(id).ok_or_else(not_found)?;
    let bytes = drive
        .items
        .get(&path)
        .and_then(|n| n.content.clone())
        .ok_or_else(not_found)?;
    Ok(Response::new(Body::from(bytes)))
}

/// The pre-authenticated upload-session URL.
fn upload_chunk(
    drive: &mut GraphDrive,
    origin: &str,
    method: &Method,
    sid: &str,
    headers: &HeaderMap,
    body: &Bytes,
) -> Reply {
    if headers.contains_key(AUTHORIZATION) {
        drive.counters.leaked_auth += 1;
        return Err(error(StatusCode::UNAUTHORIZED, "unauthenticated"));
    }
    if *method == Method::DELETE {
        let _ = drive.sessions.remove(sid);
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let bad_range = || error(StatusCode::RANGE_NOT_SATISFIABLE, "invalidRange");
    let (start, end, total) = headers
        .get(CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_range)
        .ok_or_else(bad_range)?;
    let session = drive.sessions.get_mut(sid).ok_or_else(not_found)?;
    if start != session.received.len()
        || end + 1 - start != body.len()
        || end >= total
    {
        return Err(bad_range());
    }
    session.received.extend_from_slice(body);
    drive.counters.chunks += 1;
    if end + 1 < total {
        return Ok((
            StatusCode::ACCEPTED,
            axum::Json(json!({
                "expirationDateTime": now(),
                "nextExpectedRanges": [format!("{}-", end + 1)],
            })),
        )
            .into_response());
    }
    // The last chunk commits: check the session's conditions atomically.
    let session = drive.sessions.remove(sid).ok_or_else(not_found)?;
    drive.check_if_match(&session.path, session.if_match.as_deref())?;
    if session.fail_if_exists && drive.items.contains_key(&session.path) {
        return Err(error(StatusCode::CONFLICT, "nameAlreadyExists"));
    }
    drive.ensure_parents(&session.path)?;
    let created = drive.write(&session.path, Bytes::from(session.received));
    drive.counters.session_commits += 1;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok(drive.ok_item(status, origin, &session.path))
}

/// Parses `bytes {start}-{end}/{total}`.
fn parse_range(value: &str) -> Option<(usize, usize, usize)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

fn parse_query(query: Option<&str>) -> HashMap<String, String> {
    query
        .unwrap_or_default()
        .split('&')
        .filter(|kv| !kv.is_empty())
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            Some((decode(k)?, decode(v)?))
        })
        .collect()
}

fn decode(raw: &str) -> Option<String> {
    let mut out = Vec::with_capacity(raw.len());
    let mut bytes = raw.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = [bytes.next()?, bytes.next()?];
            out.push(
                u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?,
            );
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).ok()
}
