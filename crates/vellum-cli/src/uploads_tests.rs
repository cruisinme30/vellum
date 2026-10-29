// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The upload routes over a real folder, with a stand-in virus scanner.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use base64::Engine as _;
use http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt as _;
use vellum_contracts::{CredentialKind, Principal, Scope, ScopeSet};
use vellum_drive::memory::{MemoryAccess, MemoryIndex, Role};
use vellum_drive::{Drive, Node, SpaceKind};
use vellum_kernel::{Actor, Timestamp, UserId};
use vellum_security::upload::{Scanner, UploadError, UploadPolicy};
use vellum_store::LocalStore;

use crate::uploads::{self, PREFIX};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// A 1×1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
/// The antivirus test file, which every scanner reports.
const EICAR: &str = r"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

/// Finds the antivirus test file, or is down.
struct FakeScanner {
    down: bool,
}

impl Scanner for FakeScanner {
    fn scan(&self, bytes: &[u8]) -> impl Future<Output = Result<(), UploadError>> + Send {
        let found = bytes.windows(10).any(|w| w == b"EICAR-STAN");
        let down = self.down;
        async move {
            if down {
                Err(UploadError::ScannerUnavailable("connection refused".into()))
            } else if found {
                Err(UploadError::Infected("Eicar-Signature".into()))
            } else {
                Ok(())
            }
        }
    }
}

struct World {
    _dir: tempfile::TempDir,
    drive: Arc<Drive>,
    access: Arc<MemoryAccess>,
    owner: UserId,
    page: Node,
}

impl World {
    async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let dir = tempfile::tempdir()?;
        let store = Arc::new(LocalStore::open(dir.path()).await?);
        let index = Arc::new(MemoryIndex::new());
        let access = Arc::new(MemoryAccess::new(index.clone()));
        let drive = Arc::new(Drive::new(store, index, access.clone()));
        let owner = UserId::generate();
        let actor = Actor::User { user: owner };
        let space = drive
            .create_space(&actor, SpaceKind::SharedDrive, "launch", "Launch", None)
            .await?;
        let page = drive
            .create_page(&actor, space.id, None, "Launch plan", "Ship it.\n")
            .await?;
        Ok(Self {
            _dir: dir,
            drive,
            access,
            owner,
            page,
        })
    }

    /// The routes as `user` sees them, with `scopes`.
    fn app(&self, user: Option<UserId>, scopes: &[Scope], scanner: FakeScanner) -> Router {
        self.app_with(user, scopes, scanner, UploadPolicy::default())
    }

    fn app_with(
        &self,
        user: Option<UserId>,
        scopes: &[Scope],
        scanner: FakeScanner,
        policy: UploadPolicy,
    ) -> Router {
        let principal = user.map_or_else(Principal::anonymous, |user| {
            Principal::user(
                user,
                CredentialKind::Session,
                ScopeSet::from_scopes(scopes.iter().copied()),
                Timestamp::now(),
            )
        });
        Router::new()
            .nest(
                PREFIX,
                uploads::router(Arc::clone(&self.drive), Some(Arc::new(scanner)), policy),
            )
            .layer(axum::Extension(principal))
    }

    fn files(&self) -> String {
        format!("{PREFIX}/{}", self.page.id)
    }
}

const fn ok() -> FakeScanner {
    FakeScanner { down: false }
}

fn upload(path: &str, name: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::post(format!("{path}?name={name}"))
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
        .unwrap_or_else(|e| unreachable!("{e}"))
}

fn get(path: &str) -> Request<Body> {
    Request::get(path)
        .body(Body::empty())
        .unwrap_or_else(|e| unreachable!("{e}"))
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, http::HeaderMap, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .unwrap_or_else(|e| unreachable!("{e}"));
    let (parts, body) = response.into_parts();
    let bytes = axum::body::to_bytes(body, usize::MAX)
        .await
        .unwrap_or_default();
    (parts.status, parts.headers, bytes.to_vec())
}

fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap_or_default()
}

#[tokio::test]
async fn uploads_are_checked_stored_and_served() -> TestResult {
    let w = World::new().await?;
    let app = w.app(Some(w.owner), &Scope::ALL, ok());

    let (status, _, body) = call(&app, upload(&w.files(), "Q3%20chart.png", png())).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let file: Value = serde_json::from_slice(&body)?;
    assert_eq!(file["name"], "Q3 chart.png");
    assert_eq!(file["media_type"], "image/png");
    let url = format!("{}/Q3%20chart.png", w.files());
    assert_eq!(file["url"], url.as_str());
    assert_eq!(
        file["ref"],
        format!("vellum://attachment/{}/Q3%20chart.png", w.page.id).as_str()
    );

    // An image is shown inline, with nothing that lets it run.
    let (status, headers, bytes) = call(&app, get(&url)).await;
    assert_eq!(status, StatusCode::OK);
    assert!(bytes.starts_with(b"\x89PNG"));
    let header = |name: header::HeaderName| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned()
    };
    assert_eq!(header(header::CONTENT_TYPE), "image/png");
    assert_eq!(header(header::X_CONTENT_TYPE_OPTIONS), "nosniff");
    assert!(header(header::CONTENT_SECURITY_POLICY).contains("sandbox"));
    assert!(header(header::CONTENT_DISPOSITION).starts_with("inline;"));

    // Markdown and SVG are files to download, never pages or drawings.
    let (status, ..) = call(&app, upload(&w.files(), "notes.md", b"# Notes\n".to_vec())).await;
    assert_eq!(status, StatusCode::CREATED);
    let svg = b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec();
    let (status, ..) = call(&app, upload(&w.files(), "logo.svg", svg)).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, headers, _) = call(&app, get(&format!("{}/logo.svg", w.files()))).await;
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(disposition.starts_with("attachment;"), "{disposition}");

    let (status, _, body) = call(&app, get(&w.files())).await;
    assert_eq!(status, StatusCode::OK);
    let list: Value = serde_json::from_slice(&body)?;
    let names: Vec<&str> = list["items"]
        .as_array()
        .map(|items| items.iter().filter_map(|f| f["name"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(names, ["Q3 chart.png", "logo.svg", "notes.md"]);

    let delete = Request::delete(&url).body(Body::empty())?;
    assert_eq!(call(&app, delete).await.0, StatusCode::NO_CONTENT);
    assert_eq!(call(&app, get(&url)).await.0, StatusCode::NOT_FOUND);
    Ok(())
}

#[tokio::test]
async fn bad_uploads_are_refused_and_not_stored() -> TestResult {
    let w = World::new().await?;
    let app = w.app(Some(w.owner), &Scope::ALL, ok());
    let files = w.files();

    // Content that does not match its name, or is not a kind Vellum takes.
    let status = call(&app, upload(&files, "chart.pdf", png())).await.0;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let status = call(&app, upload(&files, "tool.exe", vec![0x4d, 0x5a, 0, 0x90]))
        .await
        .0;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // The virus scanner's verdict holds, and a scanner that is down
    // refuses everything.
    let (status, _, body) = call(&app, upload(&files, "eicar.txt", EICAR.into())).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(String::from_utf8_lossy(&body).contains("virus scanner"));
    let down = w.app(Some(w.owner), &Scope::ALL, FakeScanner { down: true });
    let (status, headers, _) = call(&down, upload(&files, "notes.txt", b"hello".to_vec())).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(headers.contains_key(header::RETRY_AFTER));

    // Too large is refused from the declared length, before reading.
    let mut small = UploadPolicy::default();
    small.max_bytes = 16;
    let tight = w.app_with(Some(w.owner), &Scope::ALL, ok(), small);
    let status = call(&tight, upload(&files, "a.txt", vec![b'a'; 17]))
        .await
        .0;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // An upload says its length and its name.
    let no_length = Request::post(format!("{files}?name=a.txt")).body(Body::from("hello"))?;
    assert_eq!(call(&app, no_length).await.0, StatusCode::BAD_REQUEST);
    let no_name = upload(&files, "", b"hello".to_vec());
    assert_eq!(call(&app, no_name).await.0, StatusCode::BAD_REQUEST);

    let (_, _, body) = call(&app, get(&files)).await;
    let list: Value = serde_json::from_slice(&body)?;
    assert_eq!(list["items"].as_array().map(Vec::len), Some(0), "{list}");
    Ok(())
}

#[tokio::test]
async fn rights_are_checked_before_the_body() -> TestResult {
    let w = World::new().await?;
    let files = w.files();
    let owner = w.app(Some(w.owner), &Scope::ALL, ok());
    assert_eq!(
        call(&owner, upload(&files, "notes.txt", b"hello".to_vec()))
            .await
            .0,
        StatusCode::CREATED
    );

    // Signed out, or reading with a read-only token: no upload.
    let anonymous = w.app(None, &[], ok());
    assert_eq!(
        call(&anonymous, upload(&files, "a.txt", b"x".to_vec()))
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    let read_only = w.app(Some(w.owner), &[Scope::Read], ok());
    assert_eq!(
        call(&read_only, upload(&files, "a.txt", b"x".to_vec()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(&read_only, get(&files)).await.0, StatusCode::OK);

    // A viewer reads the files but cannot add or delete one.
    let viewer = UserId::generate();
    w.access.grant(viewer, w.page.id, Role::Viewer);
    let as_viewer = w.app(Some(viewer), &Scope::ALL, ok());
    let file = format!("{files}/notes.txt");
    assert_eq!(call(&as_viewer, get(&file)).await.0, StatusCode::OK);
    assert_eq!(
        call(&as_viewer, upload(&files, "b.txt", b"x".to_vec()))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let delete = Request::delete(&file).body(Body::empty())?;
    assert_eq!(call(&as_viewer, delete).await.0, StatusCode::FORBIDDEN);

    // Someone with no access learns nothing, and a bad ID is not found.
    let stranger = w.app(Some(UserId::generate()), &Scope::ALL, ok());
    assert_eq!(call(&stranger, get(&file)).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        call(&owner, get(&format!("{PREFIX}/not-an-id"))).await.0,
        StatusCode::NOT_FOUND
    );
    Ok(())
}
