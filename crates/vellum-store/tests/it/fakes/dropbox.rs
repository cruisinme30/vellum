// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An in-process fake of the Dropbox API v2 file endpoints, enough to run
//! the conformance suite against `DropboxStore`.
//!
//! It keeps files in memory, keyed by lower-cased path as Dropbox paths
//! are case-insensitive, and treats folders as implied by the files
//! beneath them. Every write checks its mode and applies it under one lock,
//! so conditional writes are atomic as on Dropbox. Errors use Dropbox's
//! shapes: `409` with an `error_summary`, `401` for a bad token, and `400`
//! with plain text for a malformed call (such as a non-ASCII
//! `Dropbox-API-Arg`).

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Write as _,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::SystemTime,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

/// A running fake. The server stops when this is dropped.
#[derive(Debug)]
pub struct FakeDropbox {
    /// Base URL for both the API and the content endpoints.
    pub url: String,
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

#[derive(Debug)]
struct Shared {
    token: Mutex<String>,
    page_limit: AtomicUsize,
    data: Mutex<Data>,
    calls: Mutex<Vec<Call>>,
}

/// One request the fake received.
#[derive(Clone, Debug)]
pub struct Call {
    /// Endpoint, such as `files/upload`.
    pub endpoint: String,
    /// Raw `Dropbox-API-Arg` header, if any.
    pub api_arg: Option<Vec<u8>>,
    /// Status the fake answered with.
    pub status: u16,
}

#[derive(Debug, Default)]
struct Data {
    files: BTreeMap<String, File>,
    next_rev: u64,
    next_id: u64,
    sessions: HashMap<String, Vec<u8>>,
    cursors: HashMap<String, Vec<Value>>,
}

#[derive(Clone, Debug)]
struct File {
    display: String,
    bytes: Bytes,
    rev: String,
    modified: SystemTime,
}

type Reply = Result<Response, Fault>;

/// A failed call, in one of Dropbox's error shapes.
#[derive(Debug)]
enum Fault {
    /// `409` with an `error_summary`.
    Api(String),
    /// `400` with plain text.
    BadRequest(String),
    /// `404` for an endpoint the fake does not know.
    UnknownEndpoint,
}

impl IntoResponse for Fault {
    fn into_response(self) -> Response {
        match self {
            Self::Api(summary) => {
                let tag = summary.split('/').next().unwrap_or_default().to_owned();
                let body = json!({ "error_summary": summary, "error": { ".tag": tag } });
                (StatusCode::CONFLICT, axum::Json(body)).into_response()
            }
            Self::BadRequest(message) => {
                let text = format!("Error in call to API function: {message}");
                (StatusCode::BAD_REQUEST, text).into_response()
            }
            Self::UnknownEndpoint => (StatusCode::NOT_FOUND, "Unknown endpoint").into_response(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeDropbox {
    /// Starts a fake on `127.0.0.1` that accepts the bearer token `token`.
    ///
    /// # Errors
    /// Returns the I/O error if the listener cannot bind.
    pub async fn start(token: &str) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            token: Mutex::new(token.to_owned()),
            page_limit: AtomicUsize::new(1000),
            data: Mutex::new(Data::default()),
            calls: Mutex::new(Vec::new()),
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let app = Router::new()
            .fallback(dispatch)
            .with_state(Arc::clone(&shared));
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { url, shared, task })
    }

    /// Makes listings return at most `limit` entries per page.
    pub fn set_page_limit(&self, limit: usize) {
        self.shared.page_limit.store(limit, Ordering::SeqCst);
    }

    /// Every request received so far.
    pub fn calls(&self) -> Vec<Call> {
        lock(&self.shared.calls).clone()
    }

    /// How many requests went to `endpoint`.
    pub fn count(&self, endpoint: &str) -> usize {
        lock(&self.shared.calls)
            .iter()
            .filter(|c| c.endpoint == endpoint)
            .count()
    }

    /// Places a file directly, as another Dropbox client would.
    pub fn insert(&self, display: &str, bytes: &'static [u8]) {
        let mut data = lock(&self.shared.data);
        let rev = data.rev();
        let file = File {
            display: display.to_owned(),
            bytes: Bytes::from_static(bytes),
            rev,
            modified: SystemTime::now(),
        };
        let _ = data.files.insert(display.to_lowercase(), file);
    }

    /// The display path of the file at `path`, if there is one.
    pub fn display(&self, path: &str) -> Option<String> {
        let data = lock(&self.shared.data);
        data.files
            .get(&path.to_lowercase())
            .map(|f| f.display.clone())
    }
}

impl Drop for FakeDropbox {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn dispatch(
    State(shared): State<Arc<Shared>>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let endpoint = uri.path().trim_start_matches("/2/").to_owned();
    let api_arg = headers
        .get("Dropbox-API-Arg")
        .map(|v| v.as_bytes().to_vec());
    let reply = handle(&shared, &endpoint, &headers, api_arg.as_deref(), &body);
    let response = reply.unwrap_or_else(IntoResponse::into_response);
    lock(&shared.calls).push(Call {
        endpoint,
        api_arg,
        status: response.status().as_u16(),
    });
    response
}

fn handle(
    shared: &Shared,
    endpoint: &str,
    headers: &HeaderMap,
    api_arg: Option<&[u8]>,
    body: &Bytes,
) -> Reply {
    let bearer = format!("Bearer {}", lock(&shared.token));
    if headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        != Some(bearer.as_bytes())
    {
        let body = json!({
            "error_summary": "expired_access_token/..",
            "error": { ".tag": "expired_access_token" },
        });
        return Ok((StatusCode::UNAUTHORIZED, axum::Json(body)).into_response());
    }
    let content = matches!(
        endpoint,
        "files/download"
            | "files/upload"
            | "files/upload_session/start"
            | "files/upload_session/append_v2"
            | "files/upload_session/finish"
    );
    let arg = if content {
        let raw = api_arg.ok_or_else(|| bad_request("missing Dropbox-API-Arg"))?;
        if !raw.iter().all(|b| b.is_ascii() && *b != 0x7f) {
            return Err(bad_request(
                "Dropbox-API-Arg must be ASCII; escape other characters as \\uXXXX",
            ));
        }
        if endpoint != "files/download"
            && headers.get(header::CONTENT_TYPE).map(HeaderValue::as_bytes)
                != Some(b"application/octet-stream")
        {
            return Err(bad_request("Content-Type must be application/octet-stream"));
        }
        parse(raw)?
    } else {
        parse(body)?
    };
    let page_limit = shared.page_limit.load(Ordering::SeqCst).max(1);
    let mut data = lock(&shared.data);
    match endpoint {
        "files/download" => data.download(&arg),
        "files/get_metadata" => data.get_metadata(&arg).map(ok_json),
        "files/upload" => data.commit(&arg, body.clone()).map(ok_json),
        "files/upload_session/start" => Ok(ok_json(data.start(body))),
        "files/upload_session/append_v2" => data.append(&arg, body).map(ok_json),
        "files/upload_session/finish" => data.finish(&arg, body).map(ok_json),
        "files/delete_v2" => data.delete(&arg).map(ok_json),
        "files/move_v2" => data.move_file(&arg).map(ok_json),
        "files/list_folder" => data.list_folder(&arg, page_limit).map(ok_json),
        "files/list_folder/continue" => data.list_continue(&arg, page_limit).map(ok_json),
        _ => Err(Fault::UnknownEndpoint),
    }
}

impl Data {
    fn rev(&mut self) -> String {
        self.next_rev += 1;
        format!("{:015x}", 0x5f00_0000_0000 + self.next_rev)
    }

    fn id(&mut self) -> String {
        self.next_id += 1;
        format!("id{}", self.next_id)
    }

    fn is_folder(&self, lower: &str) -> bool {
        lower.is_empty() || {
            let prefix = format!("{lower}/");
            self.files.keys().any(|k| k.starts_with(&prefix))
        }
    }

    /// An ancestor of `lower` that is a file, if any.
    fn file_ancestor(&self, lower: &str) -> bool {
        lower
            .match_indices('/')
            .any(|(i, _)| lower.get(..i).is_some_and(|a| self.files.contains_key(a)))
    }

    fn download(&self, arg: &Value) -> Reply {
        let lower = path_arg(arg, "path")?;
        match self.files.get(&lower) {
            Some(f) => {
                let meta = ascii(&file_json(f, false).to_string());
                Ok((
                    StatusCode::OK,
                    [
                        ("Dropbox-API-Result", meta),
                        (
                            header::CONTENT_TYPE.as_str(),
                            "application/octet-stream".to_owned(),
                        ),
                    ],
                    f.bytes.clone(),
                )
                    .into_response())
            }
            None if self.is_folder(&lower) => Err(api_error("path/not_file/..")),
            None => Err(api_error("path/not_found/...")),
        }
    }

    fn get_metadata(&self, arg: &Value) -> Result<Value, Fault> {
        let lower = path_arg(arg, "path")?;
        if lower.is_empty() {
            return Err(bad_request("The root folder is unsupported."));
        }
        match self.files.get(&lower) {
            Some(f) => Ok(file_json(f, true)),
            None if self.is_folder(&lower) => Ok(folder_json(&lower)),
            None => Err(api_error("path/not_found/..")),
        }
    }

    /// Applies an upload under its write mode.
    fn commit(&mut self, commit: &Value, bytes: Bytes) -> Result<Value, Fault> {
        let display = commit
            .get("path")
            .and_then(Value::as_str)
            .filter(|p| p.starts_with('/') && !p.ends_with('/'))
            .ok_or_else(|| api_error("path/malformed_path/.."))?
            .to_owned();
        let lower = display.to_lowercase();
        let strict = commit
            .get("strict_conflict")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if commit.get("autorename").and_then(Value::as_bool) == Some(true) {
            return Err(bad_request("the fake does not autorename"));
        }
        if self.is_folder(&lower) {
            return Err(api_error("path/conflict/folder/..."));
        }
        if self.file_ancestor(&lower) {
            return Err(api_error("path/conflict/file/..."));
        }
        let mode = commit.get("mode").unwrap_or(&Value::Null);
        let tag = mode
            .as_str()
            .or_else(|| mode.get(".tag").and_then(Value::as_str))
            .unwrap_or("add");
        let existing = self.files.get(&lower).cloned();
        let same = existing.as_ref().is_some_and(|f| f.bytes == bytes);
        match tag {
            "add" if existing.is_some() && (strict || !same) => {
                return Err(api_error("path/conflict/file/..."));
            }
            "update" => {
                let want = mode.get("update").and_then(Value::as_str).unwrap_or("");
                match &existing {
                    Some(f) if f.rev == want => {}
                    None if !strict => {}
                    _ => return Err(api_error("path/conflict/file/...")),
                }
            }
            "add" | "overwrite" => {}
            _ => return Err(bad_request("unknown write mode")),
        }
        // Like Dropbox, identical content keeps the current revision.
        if let Some(f) = existing.filter(|_| same) {
            return Ok(file_json(&f, false));
        }
        let display = self
            .files
            .get(&lower)
            .map_or(display, |f| f.display.clone());
        let file = File {
            display,
            bytes,
            rev: self.rev(),
            modified: SystemTime::now(),
        };
        let out = file_json(&file, false);
        let _ = self.files.insert(lower, file);
        Ok(out)
    }

    fn start(&mut self, body: &Bytes) -> Value {
        let id = self.id();
        let _ = self.sessions.insert(id.clone(), body.to_vec());
        json!({ "session_id": id })
    }

    fn session(&mut self, cursor: Option<&Value>) -> Result<(String, &mut Vec<u8>), Fault> {
        let cursor = cursor.ok_or_else(|| bad_request("missing cursor"))?;
        let id = cursor
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let offset = cursor.get("offset").and_then(Value::as_u64);
        let buffer = self
            .sessions
            .get_mut(&id)
            .ok_or_else(|| api_error("lookup_failed/not_found/.."))?;
        if offset != Some(buffer.len() as u64) {
            return Err(api_error("lookup_failed/incorrect_offset/.."));
        }
        Ok((id, buffer))
    }

    fn append(&mut self, arg: &Value, body: &Bytes) -> Result<Value, Fault> {
        let (_, buffer) = self.session(arg.get("cursor"))?;
        buffer.extend_from_slice(body);
        Ok(Value::Null)
    }

    fn finish(&mut self, arg: &Value, body: &Bytes) -> Result<Value, Fault> {
        let (id, buffer) = self.session(arg.get("cursor"))?;
        buffer.extend_from_slice(body);
        let bytes = Bytes::from(std::mem::take(buffer));
        let _ = self.sessions.remove(&id);
        let commit = arg
            .get("commit")
            .ok_or_else(|| bad_request("missing commit"))?;
        self.commit(commit, bytes)
    }

    fn delete(&mut self, arg: &Value) -> Result<Value, Fault> {
        let lower = path_arg(arg, "path")?;
        let parent_rev = arg.get("parent_rev").and_then(Value::as_str);
        if let Some(f) = self.files.get(&lower) {
            if parent_rev.is_some_and(|rev| rev != f.rev) {
                return Err(api_error("path_lookup/not_found/.."));
            }
            let meta = file_json(f, true);
            let _ = self.files.remove(&lower);
            return Ok(json!({ "metadata": meta }));
        }
        if self.is_folder(&lower) && !lower.is_empty() {
            if parent_rev.is_some() {
                return Err(api_error("path_lookup/not_file/.."));
            }
            let prefix = format!("{lower}/");
            self.files.retain(|k, _| !k.starts_with(&prefix));
            return Ok(json!({ "metadata": folder_json(&lower) }));
        }
        Err(api_error("path_lookup/not_found/.."))
    }

    fn move_file(&mut self, arg: &Value) -> Result<Value, Fault> {
        let from = path_arg(arg, "from_path")?;
        let to_display = arg
            .get("to_path")
            .and_then(Value::as_str)
            .filter(|p| p.starts_with('/'))
            .ok_or_else(|| api_error("to/malformed_path/.."))?
            .to_owned();
        let to = to_display.to_lowercase();
        let is_file = self.files.contains_key(&from);
        if !is_file && (from.is_empty() || !self.is_folder(&from)) {
            return Err(api_error("from_lookup/not_found/.."));
        }
        if to != from {
            if self.files.contains_key(&to) || self.file_ancestor(&to) {
                return Err(api_error("to/conflict/file/.."));
            }
            if self.is_folder(&to) {
                return Err(api_error("to/conflict/folder/.."));
            }
        }
        if let Some(mut f) = self.files.remove(&from) {
            f.display = to_display;
            let meta = file_json(&f, true);
            let _ = self.files.insert(to, f);
            return Ok(json!({ "metadata": meta }));
        }
        let prefix = format!("{from}/");
        let moved: Vec<String> = self
            .files
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        for key in moved {
            if let Some(mut f) = self.files.remove(&key) {
                let rest = key.get(prefix.len()..).unwrap_or_default();
                let tail = f.display.rsplit('/').next().unwrap_or_default().to_owned();
                let mid = rest.rsplit_once('/').map_or("", |(m, _)| m);
                f.display = if mid.is_empty() {
                    format!("{to_display}/{tail}")
                } else {
                    format!("{to_display}/{mid}/{tail}")
                };
                let _ = self.files.insert(format!("{to}/{rest}"), f);
            }
        }
        Ok(json!({ "metadata": folder_json(&to) }))
    }

    fn list_folder(&mut self, arg: &Value, page_limit: usize) -> Result<Value, Fault> {
        let lower = path_arg(arg, "path")?;
        if self.files.contains_key(&lower) {
            return Err(api_error("path/not_folder/.."));
        }
        if !self.is_folder(&lower) {
            return Err(api_error("path/not_found/.."));
        }
        let recursive = arg
            .get("recursive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let limit = arg
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|l| usize::try_from(l).ok())
            .unwrap_or(page_limit);
        let prefix = if lower.is_empty() {
            "/".to_owned()
        } else {
            format!("{lower}/")
        };
        let mut entries = Vec::new();
        let mut folders = BTreeSet::new();
        if recursive && !lower.is_empty() {
            entries.push(folder_json(&lower));
        }
        for (key, f) in self.files.range(prefix.clone()..) {
            let Some(rest) = key.strip_prefix(&prefix) else {
                break;
            };
            let depth = rest.matches('/').count();
            if !recursive && depth > 0 {
                let _ = folders.insert(format!("{prefix}{}", rest.split('/').next().unwrap_or("")));
                continue;
            }
            // Implied folders on the way down, each listed once.
            let mut at = prefix.clone();
            for segment in rest.split('/').take(depth) {
                at.push_str(segment);
                let _ = folders.insert(at.clone());
                at.push('/');
            }
            entries.push(file_json(f, true));
        }
        entries.extend(folders.iter().map(|f| folder_json(f)));
        let id = self.id();
        let _ = self.cursors.insert(id.clone(), entries);
        self.page(&id, limit.max(1))
    }

    fn list_continue(&mut self, arg: &Value, page_limit: usize) -> Result<Value, Fault> {
        let cursor = arg
            .get("cursor")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !self.cursors.contains_key(&cursor) {
            return Err(api_error("reset/.."));
        }
        self.page(&cursor, page_limit)
    }

    fn page(&mut self, cursor: &str, limit: usize) -> Result<Value, Fault> {
        let rest = self
            .cursors
            .get_mut(cursor)
            .ok_or_else(|| api_error("reset/.."))?;
        let take = limit.min(rest.len());
        let page: Vec<Value> = rest.drain(..take).collect();
        let has_more = !rest.is_empty();
        Ok(json!({ "entries": page, "cursor": cursor, "has_more": has_more }))
    }
}

fn parse(raw: &[u8]) -> Result<Value, Fault> {
    serde_json::from_slice(raw).map_err(|e| bad_request(&format!("bad JSON argument: {e}")))
}

/// The lower-cased path in `arg[key]`: `""` for the root, else `/...`.
fn path_arg(arg: &Value, key: &str) -> Result<String, Fault> {
    let path = arg
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad_request("missing path"))?;
    if !(path.is_empty() || (path.starts_with('/') && !path.ends_with('/'))) {
        return Err(api_error(&format!("{key}/malformed_path/..")));
    }
    Ok(path.to_lowercase())
}

fn ok_json(value: Value) -> Response {
    (StatusCode::OK, axum::Json(value)).into_response()
}

fn api_error(summary: &str) -> Fault {
    Fault::Api(summary.to_owned())
}

fn bad_request(message: &str) -> Fault {
    Fault::BadRequest(message.to_owned())
}

fn file_json(f: &File, tagged: bool) -> Value {
    let name = f.display.rsplit('/').next().unwrap_or_default();
    let stamp = rfc3339(f.modified);
    let mut v = json!({
        "name": name,
        "path_lower": f.display.to_lowercase(),
        "path_display": f.display,
        "id": format!("id:{}", f.rev),
        "client_modified": stamp,
        "server_modified": stamp,
        "rev": f.rev,
        "size": f.bytes.len(),
        "is_downloadable": true,
    });
    if tagged && let Some(map) = v.as_object_mut() {
        let _ = map.insert(".tag".to_owned(), json!("file"));
    }
    v
}

fn folder_json(lower: &str) -> Value {
    let name = lower.rsplit('/').next().unwrap_or_default();
    json!({
        ".tag": "folder",
        "name": name,
        "path_lower": lower,
        "path_display": lower,
        "id": format!("id:{lower}"),
    })
}

/// JSON with `0x7F` and non-ASCII characters escaped, as Dropbox sends
/// `Dropbox-API-Result`.
fn ascii(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        if c.is_ascii() && c != '\u{7f}' {
            out.push(c);
        } else {
            let mut units = [0_u16; 2];
            for unit in c.encode_utf16(&mut units) {
                let _ = write!(out, "\\u{unit:04x}");
            }
        }
    }
    out
}

/// `%Y-%m-%dT%H:%M:%SZ`, Dropbox's timestamp format.
fn rfc3339(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(i64::try_from(days).unwrap_or_default());
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Howard Hinnant's `civil_from_days`.
const fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}
