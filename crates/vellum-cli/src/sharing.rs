// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Share links on the public origin (#95, R-SE-3).
//!
//! [`SharedPages`] is the gateway's [`PublicSite`]: `/s/{token}` asks the
//! drive's [`Sharing`] whether the link opens (known, live, password
//! right, its grant still there), then renders the page as sanitised HTML.
//! The visitor is anonymous: the page is read as Vellum itself only after
//! that check, internal links are not turned into URLs, and diagrams come
//! from the cache only, so a visitor never makes the server fetch anything.
//!
//! [`IdentityPasswords`] hashes link passwords with the identity service's
//! Argon2id on the blocking pool.

use std::sync::Arc;

use async_trait::async_trait;
use vellum_api::public::{PublicPage, PublicSite};
use vellum_contracts::BoxFuture;
use vellum_drive::sharing::{LinkPasswords, Opened, Sharing};
use vellum_drive::{DriveError, DriveResult};
use vellum_kernel::{Actor, Error, ErrorKind, ShareToken};
use vellum_render::resolve::Resolver;
use vellum_render::{RenderOptions, Renderer};

/// The password checks share links may run at once (RT-042): two per
/// CPU. The public origin has no sign-in rate limit, so without a bound a
/// flood of guesses would run Argon2 (19 `MiB` each) without end, as
/// sign-in did before red team round 4.
static SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> = std::sync::LazyLock::new(|| {
    let cpus = std::thread::available_parallelism().map_or(2, std::num::NonZero::get);
    tokio::sync::Semaphore::new(cpus.saturating_mul(2).clamp(2, 64))
});

/// How long a check waits for a slot before it is refused.
const SLOT_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

/// A slot for one password check, or `None` when every slot stays taken.
async fn slot() -> Option<tokio::sync::SemaphorePermit<'static>> {
    tokio::time::timeout(SLOT_WAIT, SLOTS.acquire())
        .await
        .ok()
        .and_then(Result::ok)
}

/// Link passwords through `vellum_identity::password` (Argon2id, pinned
/// parameters), off the async workers and at most [`SLOTS`] at once.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdentityPasswords;

#[async_trait]
impl LinkPasswords for IdentityPasswords {
    async fn hash(&self, password: String) -> DriveResult<String> {
        let _slot = slot()
            .await
            .ok_or_else(|| DriveError::Index("the server is busy; try again".into()))?;
        tokio::task::spawn_blocking(move || {
            vellum_identity::password::hash(&password).map_err(|e| DriveError::Index(e.to_string()))
        })
        .await
        .map_err(|e| DriveError::Index(format!("password hashing stopped: {e}")))?
    }

    async fn verify(&self, password: String, stored: String) -> bool {
        // No slot: the guess is refused, and counts as wrong.
        let Some(_slot) = slot().await else {
            return false;
        };
        tokio::task::spawn_blocking(move || vellum_identity::password::verify(&password, &stored))
            .await
            .unwrap_or(false)
    }
}

/// Resolves nothing: a shared page's internal links stay text, so the
/// public origin never points into the workspace.
struct PublicResolver;

impl Resolver for PublicResolver {
    fn href(&self, _url: &str) -> Option<String> {
        None
    }
}

/// Share links served on the public origin.
pub struct SharedPages {
    sharing: Arc<Sharing>,
    renderer: Renderer,
}

impl std::fmt::Debug for SharedPages {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedPages").finish_non_exhaustive()
    }
}

impl SharedPages {
    /// Share links over `sharing`, rendered by `renderer`.
    #[must_use]
    pub const fn new(sharing: Arc<Sharing>, renderer: Renderer) -> Self {
        Self { sharing, renderer }
    }

    async fn open(&self, token: ShareToken, password: Option<String>) -> Result<PublicPage, Error> {
        let gone = || {
            Error::new(
                ErrorKind::NotFound,
                "This link does not work. It may have expired or been turned off.",
            )
        };
        let opened = match self.sharing.open(&token, password.as_deref()).await {
            Ok(opened) => opened,
            Err(DriveError::NotFound(_)) => return Err(gone()),
            Err(other) => return Err(other.into()),
        };
        let node = match opened {
            Opened::PasswordRequired { retry } => {
                return Ok(PublicPage::PasswordRequired { retry });
            }
            Opened::Page(node) => node,
        };
        let (node, blob) = self
            .sharing
            .drive()
            .read(&Actor::System, node.id)
            .await
            .map_err(|_| gone())?;
        let markdown = String::from_utf8(blob.bytes.to_vec()).map_err(|_| gone())?;
        let doc = vellum_format::parse(&markdown).document;
        let page = self.renderer.render_offline(
            &doc,
            &PublicResolver,
            &RenderOptions {
                doc_key: Some(node.id.to_string()),
            },
        );
        let title = page.title.unwrap_or(node.title);
        Ok(PublicPage::Html {
            html: document(&title, &page.html),
            max_age: 0,
        })
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The shared page's document. `body` is the renderer's sanitised HTML.
fn document(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <meta name=\"robots\" content=\"noindex\"><title>{}</title></head>\
         <body><main>{body}</main><footer><p>Shared with Vellum</p></footer></body></html>",
        escape(title)
    )
}

impl PublicSite for SharedPages {
    fn shared(
        &self,
        token: ShareToken,
        password: Option<String>,
    ) -> BoxFuture<'_, Result<PublicPage, Error>> {
        Box::pin(self.open(token, password))
    }

    fn published<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<PublicPage, Error>> {
        Box::pin(async { Err(Error::new(ErrorKind::NotFound, "page not found")) })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use axum::body::Body;
    use http::{Request, StatusCode, header};
    use http_body_util::BodyExt as _;
    use serde_json::{Value, json};
    use tower::ServiceExt as _;
    use vellum_api::config::GatewayConfig;
    use vellum_api::gateway::GatewayBuilder;
    use vellum_authz::{Authz, MemoryStore};
    use vellum_contracts::{
        CredentialKind, Operation, Principal, RequestContext, ScopeSet, Service,
    };
    use vellum_drive::authz::AuthzAccess;
    use vellum_drive::memory::MemoryIndex;
    use vellum_drive::sharing::MemoryShareLinks;
    use vellum_drive::{Drive, DriveService, SpaceKind};
    use vellum_kernel::{RequestId, Timestamp, UserId};
    use vellum_render::figure::{FigureConfig, Figures};
    use vellum_store::LocalStore;

    use super::*;

    fn ctx(who: UserId) -> RequestContext {
        RequestContext {
            request_id: RequestId::generate(),
            principal: Principal::user(
                who,
                CredentialKind::Session,
                ScopeSet::FULL,
                Timestamp::now(),
            ),
            deadline: Instant::now() + Duration::from_secs(5),
            traceparent: None,
        }
    }

    async fn call(ops: &[Operation], who: UserId, op: &str, input: Value) -> Result<Value, Error> {
        let op = ops.iter().find(|o| o.id() == op).expect("operation");
        op.call(ctx(who), input).await
    }

    async fn get(app: &axum::Router, path: &str) -> (StatusCode, http::HeaderMap, String) {
        let request = Request::get(path)
            .header(header::HOST, "pages.localhost")
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        let (parts, body) = response.into_parts();
        let body = body.collect().await.unwrap().to_bytes();
        (
            parts.status,
            parts.headers,
            String::from_utf8_lossy(&body).into_owned(),
        )
    }

    #[tokio::test]
    #[expect(
        clippy::too_many_lines,
        reason = "one story through the API and the public origin"
    )]
    async fn a_share_link_opens_the_page_on_the_public_origin_until_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let store: Arc<dyn vellum_store::BlobStore> =
            Arc::new(LocalStore::open(dir.path()).await.unwrap());
        let index = Arc::new(MemoryIndex::new());
        let authz = Arc::new(Authz::new(MemoryStore::new()));
        let access = Arc::new(AuthzAccess::new(authz, index.clone()));
        let drive = Arc::new(Drive::new(store, index, access.clone()));
        let sharing = Arc::new(
            Sharing::new(
                Arc::clone(&drive),
                access,
                Arc::new(MemoryShareLinks::default()),
                Arc::new(IdentityPasswords),
            )
            .with_public_origin(Some("http://pages.localhost".into())),
        );
        let ops =
            Arc::new(DriveService::new(Arc::clone(&drive)).with_sharing(Arc::clone(&sharing)))
                .operations();
        let mara = UserId::generate();
        let bob = UserId::generate();
        let me = Actor::User { user: mara };
        let space = drive
            .create_space(&me, SpaceKind::SharedDrive, "eng", "Engineering", None)
            .await
            .unwrap();
        let body = "# Launch plan\n\nShip in **May**. See [[Secret page]] and <script>alert(1)</script>.\n";
        let page = drive
            .create_page(&me, space.id, None, "Launch plan", body)
            .await
            .unwrap();

        // Grants through the API: Bob becomes a viewer and is listed.
        let shared = call(
            &ops,
            mara,
            "drive.share",
            json!({"id": page.id, "principal": format!("user:{bob}"), "role": "viewer"}),
        )
        .await
        .unwrap();
        let grants = shared["grants"].as_array().unwrap();
        assert_eq!(grants[0]["role"], "owner", "strongest role first");
        assert!(
            grants
                .iter()
                .any(|g| g["principal"] == format!("user:{bob}") && g["role"] == "viewer")
        );
        assert_eq!(shared["links_allowed"], true);
        // Bob may read, but not see the sharing settings.
        assert!(
            drive
                .read(&Actor::User { user: bob }, page.id)
                .await
                .is_ok()
        );
        let refused = call(&ops, bob, "drive.sharing", json!({"id": page.id}))
            .await
            .unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::PermissionDenied);

        let created = call(&ops, mara, "drive.create_link", json!({"id": page.id}))
            .await
            .unwrap();
        let token = created["token"].as_str().unwrap().to_owned();
        assert_eq!(created["url"], format!("http://pages.localhost/s/{token}"));
        let locked = call(
            &ops,
            mara,
            "drive.create_link",
            json!({"id": page.id, "password": "northstar-2026"}),
        )
        .await
        .unwrap();
        let locked_token = locked["token"].as_str().unwrap().to_owned();

        let figures = Figures::new(FigureConfig::default()).unwrap();
        let site: Arc<dyn PublicSite> = Arc::new(SharedPages::new(
            Arc::clone(&sharing),
            Renderer::new(figures),
        ));
        let mut config = GatewayConfig::new("http://localhost", vec![3; 32]);
        config.public_origin = Some("http://pages.localhost".into());
        let app = GatewayBuilder::new(config, "test")
            .public_site(site)
            .build()
            .unwrap()
            .router();

        let (status, headers, html) = get(&app, &format!("/s/{token}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("<title>Launch plan</title>"), "{html}");
        assert!(html.contains("<strong>May</strong>"), "{html}");
        assert!(
            !html.contains("<script"),
            "scripts never reach the public page: {html}"
        );
        assert!(
            !html.contains("href=\"/"),
            "internal links stay text: {html}"
        );
        assert_eq!(headers[header::REFERRER_POLICY], "no-referrer");
        assert_eq!(headers[header::CACHE_CONTROL], "private, no-store");

        let (status, _, html) = get(&app, &format!("/s/{locked_token}")).await;
        assert_eq!(status, StatusCode::OK);
        assert!(html.contains("This page is protected"), "{html}");

        // Revoked: the same URL is a plain 404 that says nothing about why.
        let link_id = created["link"]["id"].as_str().unwrap().to_owned();
        let _ = call(&ops, mara, "drive.revoke_link", json!({"link_id": link_id}))
            .await
            .unwrap();
        let (status, _, html) = get(&app, &format!("/s/{token}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(html.contains("This link does not work"), "{html}");
        let (status, _, _) = get(&app, "/s/0000000000000000000000").await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let listed = call(&ops, mara, "drive.sharing", json!({"id": page.id}))
            .await
            .unwrap();
        let links = listed["links"].as_array().unwrap();
        assert_eq!(links.len(), 2);
        assert!(
            links
                .iter()
                .any(|l| l["id"] == link_id.as_str() && l["live"] == false)
        );
        assert!(
            links
                .iter()
                .any(|l| l["has_password"] == true && l["live"] == true)
        );
        assert!(
            !listed.to_string().contains(&token),
            "tokens are never listed"
        );

        let removed = call(
            &ops,
            mara,
            "drive.unshare",
            json!({"id": page.id, "principal": format!("user:{bob}")}),
        )
        .await
        .unwrap();
        assert_eq!(
            removed["grants"].as_array().map(Vec::len),
            Some(1),
            "only the owner remains"
        );
    }
}
