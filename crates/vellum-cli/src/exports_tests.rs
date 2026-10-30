// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The export and import routes over a real folder, with a stand-in virus
//! scanner (D229–D231).

use std::io::Write as _;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;
use tower::ServiceExt as _;
use vellum_contracts::{CredentialKind, Principal, Scope, ScopeSet};
use vellum_drive::memory::{MemoryAccess, MemoryIndex, Role};
use vellum_drive::{Drive, Node, NodeKind, Space, SpaceKind};
use vellum_kernel::{Actor, Timestamp, UserId};
use vellum_security::upload::{Scanner, UploadError, UploadPolicy};
use vellum_store::LocalStore;

use crate::exports::{EXPORT_PREFIX, IMPORT_PREFIX, Transfer};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// The antivirus test file, which every scanner reports.
const EICAR: &str = r"X5O!P%@AP[4\PZX54(P^)7CC)7}$EICAR-STANDARD-ANTIVIRUS-TEST-FILE!$H+H*";

/// Finds the antivirus test file, or is down.
struct FakeScanner {
    down: bool,
}

impl Scanner for FakeScanner {
    fn scan(&self, bytes: &[u8]) -> impl Future<Output = Result<(), UploadError>> + Send {
        // The real scanner looks inside archives; a stored zip entry
        // keeps the bytes visible to this stand-in.
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
    space: Space,
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
            .create_space(
                &actor,
                SpaceKind::SharedDrive,
                "northstar",
                "Northstar",
                None,
            )
            .await?;
        let page = drive
            .create_page(
                &actor,
                space.id,
                None,
                "Launch plan",
                "# Launch plan\n\nShip the **Northstar** beta.\n\n- Docs\n- Pricing\n",
            )
            .await?;
        Ok(Self {
            _dir: dir,
            drive,
            access,
            owner,
            space,
            page,
        })
    }

    fn app(&self, user: Option<UserId>, scopes: &[Scope], scanner: FakeScanner) -> Router {
        let principal = user.map_or_else(Principal::anonymous, |user| {
            Principal::user(
                user,
                CredentialKind::Session,
                ScopeSet::from_scopes(scopes.iter().copied()),
                Timestamp::now(),
            )
        });
        let transfer = Arc::new(Transfer::new(
            Arc::clone(&self.drive),
            None,
            None,
            Some(Arc::new(scanner)),
            UploadPolicy::default(),
            Some("https://vellum.northstar.dev".into()),
        ));
        Router::new()
            .nest(EXPORT_PREFIX, transfer.export_router())
            .nest(IMPORT_PREFIX, transfer.import_router())
            .layer(axum::Extension(principal))
    }

    fn export(&self, format: &str) -> String {
        format!("{EXPORT_PREFIX}/pages/{}?format={format}", self.page.id)
    }

    fn import(&self, source: &str, name: &str) -> String {
        format!(
            "{IMPORT_PREFIX}?space={}&source={source}&name={name}",
            self.space.id
        )
    }
}

const fn ok() -> FakeScanner {
    FakeScanner { down: false }
}

fn get(path: &str) -> Request<Body> {
    Request::get(path)
        .body(Body::empty())
        .unwrap_or_else(|e| unreachable!("{e}"))
}

fn post(path: &str, bytes: Vec<u8>) -> Request<Body> {
    Request::post(path)
        .header(header::CONTENT_LENGTH, bytes.len())
        .body(Body::from(bytes))
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

/// A zip of `(name, bytes, compressed)` entries.
fn zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, bytes, compressed) in entries {
        let method = if *compressed {
            zip::CompressionMethod::Deflated
        } else {
            zip::CompressionMethod::Stored
        };
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        out.start_file(*name, options)
            .unwrap_or_else(|e| unreachable!("{e}"));
        out.write_all(bytes).unwrap_or_else(|e| unreachable!("{e}"));
    }
    out.finish()
        .unwrap_or_else(|e| unreachable!("{e}"))
        .into_inner()
}

#[tokio::test]
async fn a_page_exports_to_every_format() -> TestResult {
    let w = World::new().await?;
    let app = w.app(Some(w.owner), &Scope::ALL, ok());

    let (status, headers, body) = call(&app, get(&w.export("md"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(String::from_utf8(body)?.contains("Ship the **Northstar** beta."));
    let disposition = headers
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert_eq!(disposition, "attachment; filename*=UTF-8''Launch%20plan.md");
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .map(http::HeaderValue::as_bytes),
        Some(&b"nosniff"[..])
    );

    let (status, headers, body) = call(&app, get(&w.export("pdf"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(body.starts_with(b"%PDF-"));
    assert_eq!(
        headers
            .get(header::CONTENT_TYPE)
            .map(http::HeaderValue::as_bytes),
        Some(&b"application/pdf"[..])
    );

    let (status, _, body) = call(&app, get(&w.export("docx"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.starts_with(b"PK\x03\x04"));

    let (status, headers, body) = call(&app, get(&w.export("html"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(body)?.contains("Northstar"));
    let csp = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(csp.contains("sandbox"), "{csp}");

    let (status, _, body) = call(&app, get(&w.export("preview"))).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let preview: Value = serde_json::from_slice(&body)?;
    assert_eq!(preview["title"], "Launch plan");
    assert!(preview["page_count"].as_u64() >= Some(1));
    let first = preview["pages"][0].as_str().unwrap_or_default();
    assert!(
        first.starts_with("<svg"),
        "{}",
        first.get(..40).unwrap_or(first)
    );
    assert!(!first.contains("<script"));
    Ok(())
}

#[tokio::test]
async fn export_needs_view_rights_and_the_read_scope() -> TestResult {
    let w = World::new().await?;
    let stranger = UserId::generate();
    let (status, _, _) = call(
        &w.app(Some(stranger), &Scope::ALL, ok()),
        get(&w.export("pdf")),
    )
    .await;
    assert!(
        matches!(status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN),
        "{status}"
    );
    let (status, _, _) = call(&w.app(None, &[], ok()), get(&w.export("md"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = call(&w.app(Some(w.owner), &[], ok()), get(&w.export("md"))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let app = w.app(Some(w.owner), &Scope::ALL, ok());
    let (status, _, _) = call(&app, get(&w.export("exe"))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = call(&app, get(&format!("{EXPORT_PREFIX}/books/x?format=pdf"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let book = vellum_kernel::BookId::generate();
    let (status, _, _) = call(
        &app,
        get(&format!("{EXPORT_PREFIX}/books/{book}?format=pdf")),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    Ok(())
}

#[tokio::test]
async fn a_markdown_zip_imports_into_a_new_folder() -> TestResult {
    let w = World::new().await?;
    let app = w.app(Some(w.owner), &Scope::ALL, ok());
    let archive = zip(&[
        (
            "Handbook/Welcome.md",
            b"# Welcome\n\nSee [Setup](Setup.md).\n",
            true,
        ),
        ("Handbook/Setup.md", b"# Setup\n\nInstall Vellum.\n", true),
    ]);
    let (status, _, body) = call(
        &app,
        post(&w.import("markdown", "Team%20wiki.zip"), archive),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let result: Value = serde_json::from_slice(&body)?;
    assert_eq!(result["title"], "Team wiki");
    assert!(result["nodes"].as_u64() >= Some(2), "{result}");

    let actor = Actor::User { user: w.owner };
    let top = w.drive.list(&actor, w.space.id, None).await?;
    let folder = top
        .iter()
        .find(|n| n.title == "Team wiki")
        .ok_or("no import folder")?;
    assert_eq!(folder.kind, NodeKind::Folder);
    assert_eq!(
        folder.id.to_string(),
        result["folder_id"].as_str().unwrap_or_default()
    );
    // The pages are indexed and readable as the importer.
    let mut pages = Vec::new();
    let mut queue = vec![folder.id];
    while let Some(parent) = queue.pop() {
        for node in w.drive.list(&actor, w.space.id, Some(parent)).await? {
            if node.kind == NodeKind::Page {
                pages.push(node.title.clone());
            } else {
                queue.push(node.id);
            }
        }
    }
    pages.sort();
    assert_eq!(pages, vec!["Setup".to_owned(), "Welcome".to_owned()]);
    Ok(())
}

#[tokio::test]
async fn hostile_imports_are_refused_before_the_importer() -> TestResult {
    let w = World::new().await?;
    let app = w.app(Some(w.owner), &Scope::ALL, ok());
    let before = w
        .drive
        .list(&Actor::User { user: w.owner }, w.space.id, None)
        .await?
        .len();

    // Malware inside the archive.
    let infected = zip(&[("Notes/eicar.md", EICAR.as_bytes(), false)]);
    let (status, _, body) = call(&app, post(&w.import("markdown", "notes.zip"), infected)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );

    // A zip bomb: 64 MiB of zeros packed into a few KiB.
    let zeros = vec![0_u8; 64 * 1024 * 1024];
    let bomb = zip(&[("Notes/bomb.md", &zeros, true)]);
    let (status, _, body) = call(&app, post(&w.import("markdown", "bomb.zip"), bomb)).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert!(String::from_utf8_lossy(&body).contains("archive refused"));

    // Not an archive at all.
    let (status, _, _) = call(
        &app,
        post(&w.import("markdown", "notes.md"), b"# Hi\n".to_vec()),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // An extension that lies about the content.
    let archive = zip(&[("a.md", b"# A\n", true)]);
    let (status, _, _) = call(&app, post(&w.import("docx", "a.png"), archive.clone())).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // The scanner is down: fail closed.
    let down = w.app(Some(w.owner), &Scope::ALL, FakeScanner { down: true });
    let (status, _, _) = call(&down, post(&w.import("markdown", "a.zip"), archive)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let after = w
        .drive
        .list(&Actor::User { user: w.owner }, w.space.id, None)
        .await?
        .len();
    assert_eq!(before, after, "a refused import wrote something");
    Ok(())
}

#[tokio::test]
async fn import_needs_edit_rights_before_the_body_is_read() -> TestResult {
    let w = World::new().await?;
    let viewer = UserId::generate();
    w.access.grant(viewer, w.space.node_id(), Role::Viewer);
    let archive = zip(&[("a.md", b"# A\n", true)]);

    let app = w.app(Some(viewer), &Scope::ALL, ok());
    let (status, _, _) = call(&app, post(&w.import("markdown", "a.zip"), archive.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let app = w.app(Some(w.owner), &[Scope::Read], ok());
    let (status, _, _) = call(&app, post(&w.import("markdown", "a.zip"), archive.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // An import must say its length, so the limit applies before reading.
    let app = w.app(Some(w.owner), &Scope::ALL, ok());
    let no_length = Request::post(w.import("markdown", "a.zip")).body(Body::from(archive))?;
    let (status, _, _) = call(&app, no_length).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _, _) = call(&app, post(&w.import("evernote", "a.zip"), Vec::new())).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    Ok(())
}
