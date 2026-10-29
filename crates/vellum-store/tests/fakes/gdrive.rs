// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! An in-process fake of the parts of Google Drive API v3 that the
//! adapter uses, served by axum on 127.0.0.1:0.
//!
//! It keeps an in-memory file table and implements `files.list` (with the
//! `q` subset the adapter sends and a deliberately small page size),
//! `files.get` (metadata and `alt=media`), `files.create`, `files.update`
//! (metadata, moves, trash), `files.delete`, and multipart, media and
//! resumable uploads. Like Drive, it allows duplicate names, returns `size`
//! and `version` as strings, gives every content upload a new
//! `headRevisionId`, bumps `version` on every change, treats the children
//! of a trashed folder as trashed, and refuses to download Google-native
//! files. Requests need a bearer token from an allow-list. Tests can
//! inject responses, accept only part of each resumable chunk, and edit
//! the table directly to play an outside editor.

use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    io,
    net::SocketAddr,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
};

use axum::{
    Router,
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use vellum_store::remote::{TokenSource, gdrive::GoogleDriveConfig};

/// Id of the folder the store lives in.
pub const ROOT_ID: &str = "rootFolder";
/// The token the fake accepts unless told otherwise.
pub const GOOD_TOKEN: &str = "good-token";
/// Largest page `files.list` returns, to exercise paging.
const PAGE_CAP: usize = 2;
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
/// 2026-01-01T00:00:00Z in milliseconds since the epoch.
const CLOCK_BASE_MS: u64 = 1_767_225_600_000;

/// One row of the file table.
#[derive(Clone, Debug)]
pub struct FileRec {
    /// Drive id.
    pub id: String,
    /// File name (not unique within a folder).
    pub name: String,
    /// Parent folder ids.
    pub parents: Vec<String>,
    /// MIME type.
    pub mime: String,
    /// Content.
    pub bytes: Vec<u8>,
    /// Revision holding the content, for binary files.
    pub head_revision_id: Option<String>,
    /// Bumped on every change.
    pub version: u64,
    /// Milliseconds after [`CLOCK_BASE_MS`].
    pub created: u64,
    /// Milliseconds after [`CLOCK_BASE_MS`].
    pub modified: u64,
    /// Explicitly trashed.
    pub trashed: bool,
}

#[derive(Debug)]
struct Session {
    /// The file being replaced, or `None` for a create.
    target: Option<String>,
    metadata: Value,
    total: usize,
    received: Vec<u8>,
}

#[derive(Debug, Default)]
struct Table {
    files: BTreeMap<String, FileRec>,
    next_id: u64,
    next_rev: u64,
    clock: u64,
    sessions: HashMap<String, Session>,
    tokens: HashSet<String>,
    faults: VecDeque<(StatusCode, Value)>,
    partial_chunks: bool,
    list_calls: usize,
    chunk_puts: usize,
    addr: String,
}

/// A running fake Drive.
#[derive(Clone, Debug)]
pub struct FakeDrive {
    table: Arc<Mutex<Table>>,
    addr: SocketAddr,
}

type Reply = Response;

fn error(status: StatusCode, reason: &str, message: &str) -> Reply {
    let body = json!({
        "error": {
            "code": status.as_u16(),
            "message": message,
            "errors": [{ "reason": reason, "message": message }],
        }
    });
    (status, axum::Json(body)).into_response()
}

fn not_found(id: &str) -> Reply {
    error(
        StatusCode::NOT_FOUND,
        "notFound",
        &format!("File not found: {id}."),
    )
}

fn bad_request(message: &str) -> Reply {
    error(StatusCode::BAD_REQUEST, "badRequest", message)
}

/// Formats milliseconds since the epoch as Drive does.
fn rfc3339(ms: u64) -> String {
    let secs = ms / 1000;
    let days = i64::try_from(secs / 86_400).unwrap_or_default();
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        ms % 1000
    )
}

impl FileRec {
    fn json(&self) -> Value {
        let mut v = json!({
            "kind": "drive#file",
            "id": self.id,
            "name": self.name,
            "mimeType": self.mime,
            "parents": self.parents,
            "version": self.version.to_string(),
            "createdTime": rfc3339(CLOCK_BASE_MS + self.created),
            "modifiedTime": rfc3339(CLOCK_BASE_MS + self.modified),
            "trashed": self.trashed,
        });
        if !self.mime.starts_with("application/vnd.google-apps.")
            && let Some(map) = v.as_object_mut()
        {
            let _ = map.insert("size".into(), json!(self.bytes.len().to_string()));
            if let Some(rev) = &self.head_revision_id {
                let _ = map.insert("headRevisionId".into(), json!(rev));
            }
        }
        v
    }
}

// ---- the `q` grammar ------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum Clause {
    Name(String),
    InParents(String),
    Trashed(bool),
    Mime { equal: bool, value: String },
}

struct Cursor<'a> {
    rest: &'a str,
}

impl Cursor<'_> {
    fn skip_ws(&mut self) {
        self.rest = self.rest.trim_start();
    }

    fn eat(&mut self, word: &str) -> Result<(), String> {
        self.skip_ws();
        self.rest = self
            .rest
            .strip_prefix(word)
            .ok_or_else(|| format!("expected {word:?} at {:?}", self.rest))?;
        Ok(())
    }

    fn word(&mut self) -> String {
        self.skip_ws();
        let len = self
            .rest
            .find(|c: char| !c.is_ascii_alphanumeric())
            .unwrap_or(self.rest.len());
        let (word, rest) = self.rest.split_at(len);
        self.rest = rest;
        word.to_owned()
    }

    fn string(&mut self) -> Result<String, String> {
        self.eat("'")?;
        let mut out = String::new();
        let mut chars = self.rest.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some((_, escaped)) => out.push(escaped),
                    None => return Err("dangling escape".into()),
                },
                '\'' => {
                    self.rest = self.rest.get(i + 1..).unwrap_or_default();
                    return Ok(out);
                }
                c => out.push(c),
            }
        }
        Err("unterminated string".into())
    }
}

/// Parses `clause (and clause)*` where a clause is `name = '…'`,
/// `'…' in parents`, `trashed = true|false` or `mimeType (=|!=) '…'`.
fn parse_q(q: &str) -> Result<Vec<Clause>, String> {
    let mut cur = Cursor { rest: q };
    let mut clauses = Vec::new();
    loop {
        cur.skip_ws();
        if cur.rest.starts_with('\'') {
            let id = cur.string()?;
            cur.eat("in")?;
            cur.eat("parents")?;
            clauses.push(Clause::InParents(id));
        } else {
            let field = cur.word();
            cur.skip_ws();
            let equal = if cur.rest.starts_with("!=") {
                cur.eat("!=")?;
                false
            } else {
                cur.eat("=")?;
                true
            };
            cur.skip_ws();
            clauses.push(match (field.as_str(), equal) {
                ("name", true) => Clause::Name(cur.string()?),
                ("mimeType", _) => Clause::Mime {
                    equal,
                    value: cur.string()?,
                },
                ("trashed", true) => match cur.word().as_str() {
                    "true" => Clause::Trashed(true),
                    "false" => Clause::Trashed(false),
                    other => return Err(format!("bad boolean {other:?}")),
                },
                _ => return Err(format!("unsupported clause on {field:?}")),
            });
        }
        cur.skip_ws();
        if cur.rest.is_empty() {
            return Ok(clauses);
        }
        cur.eat("and")?;
    }
}

// ---- multipart ------------------------------------------------------------

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Splits a two-part `multipart/related` body into metadata and media.
fn parse_multipart(content_type: &str, body: &[u8]) -> Result<(Value, Vec<u8>), String> {
    let boundary = content_type
        .split(';')
        .find_map(|p| p.trim().strip_prefix("boundary="))
        .ok_or("no boundary")?
        .trim_matches('"');
    let delimiter = format!("--{boundary}");
    let mut parts = Vec::new();
    let mut rest = body;
    loop {
        let start = find(rest, delimiter.as_bytes()).ok_or("missing delimiter")?;
        rest = rest.get(start + delimiter.len()..).ok_or("truncated")?;
        if rest.starts_with(b"--") {
            break;
        }
        let rest_after_crlf = rest.strip_prefix(b"\r\n").ok_or("missing CRLF")?;
        let headers_end = find(rest_after_crlf, b"\r\n\r\n").ok_or("missing part headers")?;
        let content = rest_after_crlf
            .get(headers_end + 4..)
            .ok_or("truncated part")?;
        let end =
            find(content, format!("\r\n{delimiter}").as_bytes()).ok_or("unterminated part")?;
        parts.push(content.get(..end).ok_or("bad part")?.to_vec());
        rest = content.get(end + 2..).ok_or("truncated")?;
    }
    match parts.as_slice() {
        [meta, media] => Ok((
            serde_json::from_slice(meta).map_err(|e| e.to_string())?,
            media.clone(),
        )),
        _ => Err(format!("expected 2 parts, got {}", parts.len())),
    }
}

// ---- the table ------------------------------------------------------------

impl Table {
    /// A file as Drive shows it: inside a trashed folder counts as trashed.
    fn view(&self, file: &FileRec) -> Value {
        let mut v = file.json();
        if let Some(map) = v.as_object_mut() {
            let _ = map.insert("trashed".into(), json!(self.trashed(&file.id)));
        }
        v
    }

    const fn tick(&mut self) -> u64 {
        self.clock += 1;
        self.clock
    }

    fn new_id(&mut self) -> String {
        self.next_id += 1;
        format!("f{:04}", self.next_id)
    }

    fn new_rev(&mut self) -> String {
        self.next_rev += 1;
        format!("r{:04}", self.next_rev)
    }

    /// Trashed itself or beneath a trashed folder.
    fn trashed(&self, id: &str) -> bool {
        let mut seen = HashSet::new();
        let mut stack = vec![id.to_owned()];
        while let Some(next) = stack.pop() {
            if !seen.insert(next.clone()) {
                continue;
            }
            if let Some(f) = self.files.get(&next) {
                if f.trashed {
                    return true;
                }
                stack.extend(f.parents.iter().cloned());
            }
        }
        false
    }

    fn is_folder(&self, id: &str) -> bool {
        self.files.get(id).is_some_and(|f| f.mime == FOLDER_MIME)
    }

    fn create(&mut self, metadata: &Value, bytes: Option<Vec<u8>>) -> Result<FileRec, Box<Reply>> {
        let name = metadata
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Untitled")
            .to_owned();
        let parents: Vec<String> = metadata
            .get("parents")
            .and_then(Value::as_array)
            .map_or_else(
                || vec![ROOT_ID.to_owned()],
                |a| {
                    a.iter()
                        .filter_map(|p| p.as_str().map(str::to_owned))
                        .collect()
                },
            );
        if let Some(missing) = parents.iter().find(|p| !self.is_folder(p)) {
            return Err(Box::new(not_found(missing)));
        }
        let mime = metadata
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("application/octet-stream")
            .to_owned();
        let now = self.tick();
        let id = self.new_id();
        let head_revision_id = (mime != FOLDER_MIME).then(|| self.new_rev());
        let rec = FileRec {
            id: id.clone(),
            name,
            parents,
            mime,
            bytes: bytes.unwrap_or_default(),
            head_revision_id,
            version: 1,
            created: now,
            modified: now,
            trashed: false,
        };
        let _ = self.files.insert(id, rec.clone());
        Ok(rec)
    }

    fn replace_content(&mut self, id: &str, bytes: Vec<u8>) -> Result<FileRec, Box<Reply>> {
        let now = self.tick();
        let rev = self.new_rev();
        let file = self
            .files
            .get_mut(id)
            .ok_or_else(|| Box::new(not_found(id)))?;
        file.bytes = bytes;
        file.head_revision_id = Some(rev);
        file.version += 1;
        file.modified = now;
        Ok(file.clone())
    }

    fn remove_tree(&mut self, id: &str) {
        let children: Vec<String> = self
            .files
            .values()
            .filter(|f| f.parents.iter().any(|p| p == id))
            .map(|f| f.id.clone())
            .collect();
        for child in children {
            self.remove_tree(&child);
        }
        let _ = self.files.remove(id);
    }

    fn matches(&self, file: &FileRec, clauses: &[Clause]) -> bool {
        clauses.iter().all(|c| match c {
            Clause::Name(name) => file.name == *name,
            Clause::InParents(parent) => file.parents.contains(parent),
            Clause::Trashed(want) => self.trashed(&file.id) == *want,
            Clause::Mime { equal, value } => (file.mime == *value) == *equal,
        })
    }

    fn list(&mut self, query: &HashMap<String, String>) -> Reply {
        self.list_calls += 1;
        let clauses = match parse_q(query.get("q").map_or("", String::as_str)) {
            Ok(c) => c,
            Err(e) => return bad_request(&format!("Invalid Value: q ({e})")),
        };
        let mut hits: Vec<&FileRec> = self
            .files
            .values()
            .filter(|f| f.id != ROOT_ID && self.matches(f, &clauses))
            .collect();
        // Newest first, so the adapter must do its own ordering.
        hits.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
        let size = query
            .get("pageSize")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(100)
            .clamp(1, PAGE_CAP);
        let offset = query
            .get("pageToken")
            .and_then(|s| s.strip_prefix("page-"))
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        let page: Vec<Value> = hits
            .iter()
            .skip(offset)
            .take(size)
            .map(|f| self.view(f))
            .collect();
        let mut body =
            json!({ "kind": "drive#fileList", "incompleteSearch": false, "files": page });
        if offset + size < hits.len()
            && let Some(map) = body.as_object_mut()
        {
            let _ = map.insert(
                "nextPageToken".into(),
                json!(format!("page-{}", offset + size)),
            );
        }
        axum::Json(body).into_response()
    }

    fn get(&self, id: &str, query: &HashMap<String, String>) -> Reply {
        let Some(file) = self.files.get(id) else {
            return not_found(id);
        };
        if query.get("alt").is_some_and(|a| a == "media") {
            if file.mime.starts_with("application/vnd.google-apps.") {
                return error(
                    StatusCode::FORBIDDEN,
                    "fileNotDownloadable",
                    "Only files with binary content can be downloaded. Use Export with Docs Editors files.",
                );
            }
            return (StatusCode::OK, file.bytes.clone()).into_response();
        }
        axum::Json(self.view(file)).into_response()
    }

    fn update_metadata(&mut self, id: &str, query: &HashMap<String, String>, body: &[u8]) -> Reply {
        let patch: Value = if body.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => return bad_request(&e.to_string()),
            }
        };
        if let Some(add) = query.get("addParents")
            && !self.is_folder(add)
        {
            return not_found(add);
        }
        let now = self.tick();
        let Some(file) = self.files.get_mut(id) else {
            return not_found(id);
        };
        if let Some(name) = patch.get("name").and_then(Value::as_str) {
            name.clone_into(&mut file.name);
        }
        if let Some(trashed) = patch.get("trashed").and_then(Value::as_bool) {
            file.trashed = trashed;
        }
        if let Some(remove) = query.get("removeParents") {
            file.parents.retain(|p| p != remove);
        }
        if let Some(add) = query.get("addParents") {
            file.parents.push(add.clone());
        }
        file.version += 1;
        file.modified = now;
        let file = file.clone();
        axum::Json(self.view(&file)).into_response()
    }

    fn start_session(&mut self, target: Option<String>, headers: &HeaderMap, body: &[u8]) -> Reply {
        let metadata: Value = if body.is_empty() {
            json!({})
        } else {
            match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => return bad_request(&e.to_string()),
            }
        };
        let Some(total) = headers
            .get("x-upload-content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
        else {
            return bad_request("missing X-Upload-Content-Length");
        };
        if let Some(id) = &target
            && !self.files.contains_key(id)
        {
            return not_found(id);
        }
        let upload_id = format!("u{}", self.sessions.len() + 1);
        let location = format!(
            "http://{}/upload/drive/v3/files?uploadType=resumable&upload_id={upload_id}",
            self.addr
        );
        let _ = self.sessions.insert(
            upload_id,
            Session {
                target,
                metadata,
                total,
                received: Vec::new(),
            },
        );
        (StatusCode::OK, [(header::LOCATION, location)]).into_response()
    }

    fn put_chunk(
        &mut self,
        query: &HashMap<String, String>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> Reply {
        self.chunk_puts += 1;
        let partial = self.partial_chunks;
        let Some(session) = query
            .get("upload_id")
            .and_then(|u| self.sessions.get_mut(u))
        else {
            return not_found("upload session");
        };
        let range = headers
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let Some(spec) = range.strip_prefix("bytes ") else {
            return bad_request("bad Content-Range");
        };
        let Some((span, total)) = spec.split_once('/') else {
            return bad_request("bad Content-Range");
        };
        if total.parse::<usize>().ok() != Some(session.total) {
            return bad_request("total changed");
        }
        if span != "*" {
            let Some(start) = span
                .split_once('-')
                .and_then(|(s, _)| s.parse::<usize>().ok())
            else {
                return bad_request("bad Content-Range");
            };
            if start > session.received.len() {
                return bad_request("gap in upload");
            }
            session.received.truncate(start);
            // Optionally keep only half a chunk, as a flaky network might.
            let keep = if partial && body.len() > 1 && start + body.len() < session.total {
                body.len() / 2
            } else {
                body.len()
            };
            session
                .received
                .extend_from_slice(body.get(..keep).unwrap_or_default());
        }
        if session.received.len() < session.total {
            let mut resp = StatusCode::PERMANENT_REDIRECT.into_response();
            if let Some(last) = session.received.len().checked_sub(1)
                && let Ok(v) = format!("bytes=0-{last}").parse()
            {
                let _ = resp.headers_mut().insert(header::RANGE, v);
            }
            return resp;
        }
        let Some(session) = query.get("upload_id").and_then(|u| self.sessions.remove(u)) else {
            return not_found("upload session");
        };
        let result = match &session.target {
            Some(id) => self.replace_content(id, session.received),
            None => self.create(&session.metadata, Some(session.received)),
        };
        match result {
            Ok(file) => axum::Json(file.json()).into_response(),
            Err(reply) => *reply,
        }
    }
}

// ---- routing --------------------------------------------------------------

fn route(
    table: &mut Table,
    method: &Method,
    path: &str,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
    body: &[u8],
) -> Reply {
    let upload_type = query.get("uploadType").map_or("", String::as_str);
    if let Some(rest) = path.strip_prefix("/upload/drive/v3/files") {
        let id = rest.strip_prefix('/');
        return match (method, id, upload_type) {
            (&Method::PUT, None, "resumable") => table.put_chunk(query, headers, body),
            (&Method::POST, None, "resumable") => table.start_session(None, headers, body),
            (&Method::PATCH, Some(id), "resumable") => {
                table.start_session(Some(id.to_owned()), headers, body)
            }
            (&Method::POST, None, "multipart") => {
                let content_type = headers
                    .get(header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or_default();
                match parse_multipart(content_type, body) {
                    Ok((metadata, media)) => match table.create(&metadata, Some(media)) {
                        Ok(file) => axum::Json(file.json()).into_response(),
                        Err(reply) => *reply,
                    },
                    Err(e) => bad_request(&format!("bad multipart body: {e}")),
                }
            }
            (&Method::PATCH, Some(id), "media") => match table.replace_content(id, body.to_vec()) {
                Ok(file) => axum::Json(file.json()).into_response(),
                Err(reply) => *reply,
            },
            _ => bad_request("unsupported upload"),
        };
    }
    let Some(rest) = path.strip_prefix("/drive/v3/files") else {
        return not_found(path);
    };
    match (method, rest.strip_prefix('/')) {
        (&Method::GET, None) => table.list(query),
        (&Method::POST, None) => {
            let metadata: Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => return bad_request(&e.to_string()),
            };
            match table.create(&metadata, None) {
                Ok(file) => axum::Json(file.json()).into_response(),
                Err(reply) => *reply,
            }
        }
        (&Method::GET, Some(id)) => table.get(id, query),
        (&Method::PATCH, Some(id)) => table.update_metadata(id, query, body),
        (&Method::DELETE, Some(id)) => {
            if !table.files.contains_key(id) {
                return not_found(id);
            }
            table.remove_tree(id);
            StatusCode::NO_CONTENT.into_response()
        }
        _ => bad_request("unsupported request"),
    }
}

async fn handle(
    State(table): State<Arc<Mutex<Table>>>,
    method: Method,
    uri: Uri,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: Bytes,
) -> Reply {
    let mut table = table.lock().unwrap_or_else(PoisonError::into_inner);
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if !bearer.is_some_and(|t| table.tokens.contains(t)) {
        return error(
            StatusCode::UNAUTHORIZED,
            "authError",
            "Request had invalid authentication credentials.",
        );
    }
    if let Some((status, fault)) = table.faults.pop_front() {
        return (status, axum::Json(fault)).into_response();
    }
    let reply = route(&mut table, &method, uri.path(), &query, &headers, &body);
    drop(table);
    reply
}

impl FakeDrive {
    /// Starts a fake with an empty root folder.
    pub async fn start() -> io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let mut table = Table {
            addr: addr.to_string(),
            ..Table::default()
        };
        let _ = table.tokens.insert(GOOD_TOKEN.to_owned());
        let _ = table.files.insert(
            ROOT_ID.to_owned(),
            FileRec {
                id: ROOT_ID.to_owned(),
                name: "Vellum".to_owned(),
                parents: Vec::new(),
                mime: FOLDER_MIME.to_owned(),
                bytes: Vec::new(),
                head_revision_id: None,
                version: 1,
                created: 0,
                modified: 0,
                trashed: false,
            },
        );
        let table = Arc::new(Mutex::new(table));
        let app = Router::new()
            .fallback(handle)
            .with_state(Arc::clone(&table));
        drop(tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::error!(error = %e, "fake drive stopped");
            }
        }));
        Ok(Self { table, addr })
    }

    fn lock(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A store configuration pointing at this fake.
    pub fn config(&self, tokens: Arc<dyn TokenSource>) -> GoogleDriveConfig {
        let mut config = GoogleDriveConfig::new(ROOT_ID, tokens);
        config.api_base = format!("http://{}/drive/v3", self.addr);
        config.upload_base = format!("http://{}/upload/drive/v3", self.addr);
        config
    }

    /// Adds a file or folder directly, as another Drive client would.
    /// `created` overrides the creation time (ms after the clock base).
    pub fn insert(
        &self,
        parent: &str,
        name: &str,
        mime: &str,
        bytes: &[u8],
        created: Option<u64>,
    ) -> String {
        let mut table = self.lock();
        let now = table.tick();
        let id = table.new_id();
        let head_revision_id =
            (!mime.starts_with("application/vnd.google-apps.")).then(|| table.new_rev());
        let created = created.unwrap_or(now);
        let rec = FileRec {
            id: id.clone(),
            name: name.to_owned(),
            parents: vec![parent.to_owned()],
            mime: mime.to_owned(),
            bytes: bytes.to_vec(),
            head_revision_id,
            version: 1,
            created,
            modified: now,
            trashed: false,
        };
        let _ = table.files.insert(id.clone(), rec);
        id
    }

    /// Ids of the untrashed children of `parent` named `name`, oldest first.
    pub fn ids(&self, parent: &str, name: &str) -> Vec<String> {
        let table = self.lock();
        let mut found: Vec<&FileRec> = table
            .files
            .values()
            .filter(|f| f.name == name && f.parents.iter().any(|p| p == parent) && !f.trashed)
            .collect();
        found.sort_by(|a, b| a.created.cmp(&b.created).then(a.id.cmp(&b.id)));
        let ids = found.into_iter().map(|f| f.id.clone()).collect();
        drop(table);
        ids
    }

    /// A copy of one row.
    pub fn file(&self, id: &str) -> Option<FileRec> {
        self.lock().files.get(id).cloned()
    }

    /// Deletes a file permanently, as another client would.
    pub fn remove(&self, id: &str) {
        self.lock().remove_tree(id);
    }

    /// Trashes a file, as another client would.
    pub fn trash(&self, id: &str) {
        if let Some(f) = self.lock().files.get_mut(id) {
            f.trashed = true;
        }
    }

    /// Answers the next authenticated request with `status` and `body`.
    pub fn inject(&self, status: StatusCode, body: Value) {
        self.lock().faults.push_back((status, body));
    }

    /// Keep only half of each non-final resumable chunk.
    pub fn accept_partial_chunks(&self, partial: bool) {
        self.lock().partial_chunks = partial;
    }

    /// `files.list` calls so far.
    pub fn list_calls(&self) -> usize {
        self.lock().list_calls
    }

    /// Resumable chunk `PUT`s so far.
    pub fn chunk_puts(&self) -> usize {
        self.lock().chunk_puts
    }

    /// Number of rows in the table, the root included.
    pub fn len(&self) -> usize {
        self.lock().files.len()
    }
}

#[test]
fn parses_the_query_subset() {
    let q =
        r"name = 'it\'s a \\ and b' and 'p1' in parents and trashed = false and mimeType != 'x'";
    assert_eq!(
        parse_q(q),
        Ok(vec![
            Clause::Name(r"it's a \ and b".into()),
            Clause::InParents("p1".into()),
            Clause::Trashed(false),
            Clause::Mime {
                equal: false,
                value: "x".into()
            },
        ])
    );
    assert!(parse_q("fullText contains 'x'").is_err());
}

#[test]
fn formats_drive_timestamps() {
    assert_eq!(rfc3339(CLOCK_BASE_MS + 1), "2026-01-01T00:00:00.001Z");
    assert_eq!(rfc3339(951_868_800_000), "2000-03-01T00:00:00.000Z");
}
