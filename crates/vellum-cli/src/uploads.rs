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
//! A page links to its file as `vellum://attachment/{page_id}/{name}`,
//! which a copy of the page re-points at the copy (D211). Downloads are
//! served with the sniffed type, `nosniff` and a sandboxing CSP; anything
//! but raster images and PDF is sent as a download, never shown inline.

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use bytes::Bytes;
use serde::Serialize;
use vellum_api::error::ErrorResponse;
use vellum_contracts::{Principal, Scope};
use vellum_drive::{Attachment, Drive};
use vellum_kernel::{Actor, Error, ErrorKind, ObjectId, RequestId};
use vellum_security::upload::{self, Scanner, UploadError, UploadPolicy};

/// Where the upload routes are mounted.
pub const PREFIX: &str = "/api/v1/attachments";

/// What the routes share.
struct Uploads<S> {
    drive: Arc<Drive>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
}

/// The upload routes over `drive`, scanning with `scanner` when there is
/// one.
pub fn router<S: Scanner + 'static>(
    drive: Arc<Drive>,
    scanner: Option<Arc<S>>,
    policy: UploadPolicy,
) -> Router {
    let state = Arc::new(Uploads {
        drive,
        scanner,
        policy,
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
    let length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or_else(|| at(invalid("an upload must say its Content-Length")))?;
    if length > uploads.policy.max_bytes {
        return Err(at(refused(&UploadError::TooLarge(
            uploads.policy.max_bytes,
        ))));
    }
    let bytes = axum::body::to_bytes(request.into_body(), usize::MAX)
        .await
        .map_err(|_| at(invalid("the upload did not arrive whole")))?;
    let cleaned = upload::check(
        uploads.policy,
        uploads.scanner.as_deref(),
        name,
        bytes.to_vec(),
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
