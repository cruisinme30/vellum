// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! File uploads to pages, mounted at [`PREFIX`] (D212).
//!
//! | Method   | Path                          | What                          |
//! |----------|-------------------------------|-------------------------------|
//! | `POST`   | `/{page_id}?name=chart.png`   | upload; the body is the file  |
//! | `GET`    | `/{page_id}`                  | the page's files              |
//! | `GET`    | `/{page_id}/{name}`           | one file's bytes              |
//! | `DELETE` | `/{page_id}/{name}`           | delete a file                 |
//!
//! The caller's rights on the page are checked before the body is read:
//! reading needs view rights and the `read` scope, changing needs edit
//! rights and `write:direct`. Every upload then goes through
//! [`upload::check`]: size, type sniffing against the extension, image
//! re-encoding, archive limits and the virus scanner, failing closed when
//! the scanner is down. An upload must say its `Content-Length`, which
//! must be at most the policy's limit (`100 MiB` by default).
//!
//! Upload bodies are held in memory while they are checked, so every
//! upload route first reserves its `Content-Length` from one [`Budget`]
//! per process (RT-045): 512 `MiB` in flight by default
//! (`VELLUM_UPLOAD_MEMORY_MIB`) and at most [`PER_USER`] uploads per
//! person. Past either, the upload is refused with 503 before its body is
//! read.
//!
//! A page links to its file as `vellum://attachment/{page_id}/{name}`,
//! which a copy of the page re-points at the copy (D211). Downloads are
//! served with the sniffed type, `nosniff` and a sandboxing CSP; anything
//! but raster images and PDF is sent as a download, never shown inline.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use serde::Serialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use vellum_api::error::ErrorResponse;
use vellum_contracts::{Principal, Scope};
use vellum_drive::{Attachment, Drive};
use vellum_kernel::{Actor, Error, ErrorKind, ObjectId, RequestId, UserId};
use vellum_security::upload::{self, Scanner, UploadError, UploadPolicy};

/// Where the upload routes are mounted.
pub const PREFIX: &str = "/api/v1/attachments";

/// Variable setting how many `MiB` of upload bodies may be in memory.
pub const MEMORY_VAR: &str = "VELLUM_UPLOAD_MEMORY_MIB";
/// The default for [`MEMORY_VAR`].
pub const DEFAULT_MEMORY_MIB: u32 = 512;
/// Uploads one person may have in flight at once.
pub const PER_USER: usize = 4;

/// Upload bytes held in memory at once, across every upload route, and
/// uploads per person (RT-045). Counted in `KiB`.
#[derive(Debug)]
pub struct Budget {
    kib: Arc<Semaphore>,
    total_kib: u32,
    per_user: usize,
    active: Mutex<HashMap<UserId, usize>>,
}

/// A reservation from the [`Budget`], returned when dropped.
#[derive(Debug)]
pub struct Reservation {
    _kib: OwnedSemaphorePermit,
    budget: Arc<Budget>,
    user: UserId,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.release(self.user);
    }
}

static SHARED: LazyLock<Arc<Budget>> = LazyLock::new(|| {
    let mib = std::env::var(MEMORY_VAR)
        .ok()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .filter(|m| *m > 0)
        .unwrap_or(DEFAULT_MEMORY_MIB);
    Arc::new(Budget::new(mib, PER_USER))
});

impl Budget {
    /// A budget of `mib` in flight and `per_user` uploads per person.
    #[must_use]
    pub fn new(mib: u32, per_user: usize) -> Self {
        let total_kib = mib
            .saturating_mul(1024)
            .min(u32::try_from(Semaphore::MAX_PERMITS).unwrap_or(u32::MAX));
        Self {
            kib: Arc::new(Semaphore::new(total_kib as usize)),
            total_kib,
            per_user: per_user.max(1),
            active: Mutex::new(HashMap::new()),
        }
    }

    /// The process's budget, from [`MEMORY_VAR`].
    #[must_use]
    pub fn shared() -> Arc<Self> {
        Arc::clone(&SHARED)
    }

    /// Reserves `length` bytes for `user`'s upload, without waiting.
    ///
    /// # Errors
    ///
    /// [`ErrorKind::Unavailable`] when the person has [`PER_USER`]
    /// uploads in flight or the budget is spent.
    pub fn reserve(self: &Arc<Self>, user: UserId, length: u64) -> Result<Reservation, Error> {
        let busy = || {
            Error::new(
                ErrorKind::Unavailable,
                "the server is busy with other uploads; try again in a moment",
            )
        };
        let kib = u32::try_from(length.div_ceil(1024).max(1))
            .unwrap_or(u32::MAX)
            .min(self.total_kib.max(1));
        {
            let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
            let n = active.entry(user).or_insert(0);
            if *n >= self.per_user {
                return Err(Error::new(
                    ErrorKind::Unavailable,
                    "you have too many uploads in progress; wait for one to finish",
                ));
            }
            *n += 1;
        }
        let Ok(permit) = Arc::clone(&self.kib).try_acquire_many_owned(kib) else {
            self.release(user);
            return Err(busy());
        };
        Ok(Reservation {
            _kib: permit,
            budget: Arc::clone(self),
            user,
        })
    }

    /// Gives back one of `user`'s upload slots.
    fn release(&self, user: UserId) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = active.get_mut(&user) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                let _ = active.remove(&user);
            }
        }
    }
}

/// The declared `Content-Length`, checked against `max`, with a
/// reservation for it from `budget`.
///
/// # Errors
///
/// `InvalidInput` without a length, an upload refusal over `max`, and
/// `Unavailable` when the budget is spent.
pub fn reserve_body(
    budget: &Arc<Budget>,
    request: &Request,
    user: UserId,
    max: u64,
) -> Result<Reservation, Error> {
    let length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| invalid("an upload must say its Content-Length"))?;
    if length > max {
        return Err(refused(&UploadError::TooLarge(max)));
    }
    budget.reserve(user, length)
}

/// What the routes share.
struct Uploads<S> {
    drive: Arc<Drive>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
    budget: Arc<Budget>,
}

/// The upload routes over `drive`, scanning with `scanner` when there is
/// one.
pub fn router<S: Scanner + 'static>(
    drive: Arc<Drive>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
) -> Router {
    router_with_budget(drive, scanner, policy, Budget::shared())
}

/// [`router`] with its own upload budget.
pub fn router_with_budget<S: Scanner + 'static>(
    drive: Arc<Drive>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
    budget: Arc<Budget>,
) -> Router {
    let state = Arc::new(Uploads {
        drive,
        scanner,
        policy,
        budget,
    });
    Router::new()
        .route("/{page_id}", get(list::<S>).post(put::<S>))
        .route("/{page_id}/{name}", get(fetch::<S>).delete(remove::<S>))
        .with_state(state)
}

/// The routes when `vellum serve` runs without a workspace: 503.
pub fn unavailable_router() -> Router {
    Router::new().fallback(|request: Request| async move {
        fail(
            &request,
            Error::new(
                ErrorKind::Unavailable,
                "file uploads need a workspace index",
            ),
        )
    })
}

/// One file, as the routes describe it.
#[derive(Debug, Serialize)]
struct FileView {
    page_id: ObjectId,
    name: String,
    size: u64,
    /// The sniffed media type, on upload.
    #[serde(skip_serializing_if = "Option::is_none")]
    media_type: Option<&'static str>,
    /// What a page links to.
    #[serde(rename = "ref")]
    link: String,
    /// Where to download it.
    url: String,
}

impl FileView {
    fn new(file: &Attachment, media_type: Option<&'static str>) -> Self {
        let name = encode(&file.name);
        Self {
            page_id: file.page,
            name: file.name.clone(),
            size: file.size,
            media_type,
            link: format!("vellum://attachment/{}/{name}", file.page),
            url: format!("{PREFIX}/{}/{name}", file.page),
        }
    }
}

#[derive(Debug, Serialize)]
struct FileList {
    items: Vec<FileView>,
}

async fn put<S: Scanner + 'static>(
    State(uploads): State<Arc<Uploads<S>>>,
    Path(page_id): Path<String>,
    request: Request,
) -> Response {
    match store(&uploads, &page_id, request).await {
        Ok(response) => response,
        Err((request_id, error)) => respond(request_id, error),
    }
}

/// Checks the caller, then the upload, then stores it.
async fn store<S: Scanner>(
    uploads: &Uploads<S>,
    page_id: &str,
    request: Request,
) -> Result<Response, (RequestId, Error)> {
    let id = request_id(&request);
    let at = |e: Error| (id, e);
    let actor = caller(&request, Scope::WriteDirect).map_err(at)?;
    let page = parse_page(page_id).map_err(at)?;
    let name = url::form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
        .find(|(k, _)| k == "name")
        .map(|(_, v)| v.into_owned())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| at(invalid("say the file's name in the `name` query parameter")))?;
    let _ = uploads
        .drive
        .attachment_page(&actor, page, true)
        .await
        .map_err(|e| at(e.into()))?;
    let Actor::User { user } = actor else {
        return Err(at(invalid("uploads need a signed-in person")));
    };
    let _reserved =
        reserve_body(&uploads.budget, &request, user, uploads.policy.max_bytes).map_err(at)?;
    let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .map_err(|_| at(invalid("the upload did not arrive whole")))?;
    let cleaned = upload::check(
        uploads.policy,
        uploads.scanner.as_deref(),
        name,
        Vec::from(bytes),
    )
    .await
    .map_err(|e| at(refused(&e)))?;
    let file = uploads
        .drive
        .attach(&actor, page, &cleaned.file_name, Bytes::from(cleaned.bytes))
        .await
        .map_err(|e| at(e.into()))?;
    tracing::info!(page = %page, name = %file.name, size = file.size, kind = ?cleaned.kind, "file uploaded");
    Ok(json(
        StatusCode::CREATED,
        &FileView::new(&file, Some(cleaned.kind.mime())),
    ))
}

async fn list<S: Scanner + 'static>(
    State(uploads): State<Arc<Uploads<S>>>,
    Path(page_id): Path<String>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let actor = caller(&request, Scope::Read);
    drop(request);
    let result = async {
        let files = uploads
            .drive
            .attachments(&actor?, parse_page(&page_id)?)
            .await?;
        Ok::<_, Error>(FileList {
            items: files.iter().map(|f| FileView::new(f, None)).collect(),
        })
    }
    .await;
    match result {
        Ok(list) => json(StatusCode::OK, &list),
        Err(e) => respond(id, e),
    }
}

async fn fetch<S: Scanner + 'static>(
    State(uploads): State<Arc<Uploads<S>>>,
    Path((page_id, name)): Path<(String, String)>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let actor = caller(&request, Scope::Read);
    drop(request);
    let result = async {
        let found = uploads
            .drive
            .attachment(&actor?, parse_page(&page_id)?, &name)
            .await?;
        Ok::<_, Error>(found)
    }
    .await;
    match result {
        Ok((file, bytes)) => download(&file, bytes),
        Err(e) => respond(id, e),
    }
}

async fn remove<S: Scanner + 'static>(
    State(uploads): State<Arc<Uploads<S>>>,
    Path((page_id, name)): Path<(String, String)>,
    request: Request,
) -> Response {
    let id = request_id(&request);
    let actor = caller(&request, Scope::WriteDirect);
    drop(request);
    let result = async {
        let page = parse_page(&page_id)?;
        uploads.drive.detach(&actor?, page, &name).await?;
        tracing::info!(page = %page, name = %name, "file deleted");
        Ok::<_, Error>(())
    }
    .await;
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => respond(id, e),
    }
}

/// The file's bytes with headers that keep a browser from running them.
fn download(file: &Attachment, bytes: Bytes) -> Response {
    let kind = upload::sniff(&bytes);
    let media_type = kind.map_or("application/octet-stream", upload::Kind::mime);
    let disposition = if kind.is_some_and(upload::Kind::inline) {
        "inline"
    } else {
        "attachment"
    };
    let mut response = (StatusCode::OK, Body::from(bytes)).into_response();
    let headers = response.headers_mut();
    let fixed = [
        (header::CONTENT_TYPE, media_type),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; sandbox",
        ),
        (header::CACHE_CONTROL, "private, no-cache"),
    ];
    for (name, value) in fixed {
        let _ = headers.insert(name, HeaderValue::from_static(value));
    }
    if let Ok(value) = HeaderValue::from_str(&format!(
        "{disposition}; filename*=UTF-8''{}",
        encode(&file.name)
    )) {
        let _ = headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// The signed-in person behind the request, when the credential allows
/// `scope`. Share links and anonymous callers have no drive.
pub fn caller(request: &Request, scope: Scope) -> Result<Actor, Error> {
    let principal = request
        .extensions()
        .get::<Principal>()
        .cloned()
        .unwrap_or_else(Principal::anonymous);
    let user = principal.require_user()?;
    principal.require_scope(scope)?;
    Ok(Actor::User { user })
}

pub fn parse_page(raw: &str) -> Result<ObjectId, Error> {
    raw.parse()
        .map_err(|_| Error::new(ErrorKind::NotFound, "page not found"))
}

pub fn invalid(message: &'static str) -> Error {
    Error::new(ErrorKind::InvalidInput, message)
}

/// An upload the check refused, as an error the caller can act on. The
/// HTTP status is set by [`status_for`].
pub fn refused(error: &UploadError) -> Error {
    let kind = if matches!(error, UploadError::ScannerUnavailable(_)) {
        ErrorKind::Unavailable
    } else {
        ErrorKind::InvalidInput
    };
    let message = match error {
        // The scanner's address and reply stay in the logs.
        UploadError::ScannerUnavailable(_) => "the virus scanner is unavailable; try again".into(),
        other => other.to_string(),
    };
    tracing::info!(error = %error, "upload refused");
    Error::new(kind, message).with_source(UploadRefusal(status_for(error)))
}

/// The status an upload refusal is sent with: 413 too large, 415 a type
/// Vellum does not take, 422 a file that failed its checks.
const fn status_for(error: &UploadError) -> StatusCode {
    match error {
        UploadError::TooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
        UploadError::Unknown | UploadError::Mismatch(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        UploadError::ScannerUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::UNPROCESSABLE_ENTITY,
    }
}

/// Carries an upload refusal's status through the kernel error.
#[derive(Debug)]
pub struct UploadRefusal(pub StatusCode);

impl std::fmt::Display for UploadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "upload refused with {}", self.0)
    }
}

impl std::error::Error for UploadRefusal {}

pub fn request_id(request: &Request) -> RequestId {
    request
        .extensions()
        .get::<RequestId>()
        .copied()
        .unwrap_or_else(RequestId::generate)
}

fn fail(request: &Request, error: Error) -> Response {
    respond(request_id(request), error)
}

/// The error as the gateway sends it, with an upload refusal's status.
pub fn respond(request_id: RequestId, error: Error) -> Response {
    let status = std::error::Error::source(&error)
        .and_then(|s| s.downcast_ref::<UploadRefusal>())
        .map(|r| r.0);
    let mut response = ErrorResponse::new(error, request_id).into_response();
    *response.status_mut() = status.unwrap_or_else(|| response.status());
    response
}

pub fn json(status: StatusCode, body: &impl Serialize) -> Response {
    serde_json::to_vec(body).map_or_else(
        |_| StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        |bytes| {
            (
                status,
                [(header::CONTENT_TYPE, "application/json")],
                Body::from(bytes),
            )
                .into_response()
        },
    )
}

/// Percent-encodes everything but RFC 3986's unreserved characters.
pub fn encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}
