// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Live editing in `vellum serve`: the collaboration hub over the real
//! page files, the index database and the permission engine.
//!
//! `vellum-collab` and `vellum-drive` sit on the same layer and may not
//! depend on each other, so the adapters that join them live here:
//!
//! - [`DriveFiles`] reads a page's Markdown through the drive when a room
//!   opens and saves each version back through the drive's three-way
//!   merge, so edits made to the file outside Vellum are merged, not
//!   overwritten.
//! - [`DriveAuthorizer`] admits a WebSocket upgrade only for a signed-in
//!   user who may view the page, from the app's own origin, and lets them
//!   edit only with edit permission and a token scope that allows it.
//!
//! The change log is [`vellum_collab::sql::SqlUpdateStore`], sealed with a
//! key kept in the data folder.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::Router;
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse as _;
use bytes::Bytes;
use vellum_api::error::ErrorResponse;
use vellum_collab::sql::{Envelope, SqlUpdateStore};
use vellum_collab::ws::Authorizer;
use vellum_collab::{
    Access, CollabConfig, DocumentFiles, FormatCodec, Hub, MarkdownVersion, RoomKey,
};
use vellum_contracts::{CredentialKind, Principal, Scope};
use vellum_crypto::{KekId, LocalKeyring, SecretKey};
use vellum_db::Index;
use vellum_drive::{Access as DriveAccess, Drive, DriveError, NodeKind, Permission, Saved};
use vellum_kernel::{Actor, Error, ErrorKind, ObjectId, RequestId, Result, UserId};
use vellum_store::Base;

/// Name of the key, in the data folder, that seals the change log.
const LOG_KEY_FILE: &str = "collab-log.key";

/// The live-editing hub and the access check in front of it.
#[derive(Clone)]
pub struct Collab {
    hub: Hub,
    authorizer: Arc<dyn Authorizer>,
}

impl std::fmt::Debug for Collab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Collab")
            .field("open_rooms", &self.hub.open_rooms())
            .finish_non_exhaustive()
    }
}

impl Collab {
    /// Builds the hub over `drive` and the `index` database.
    ///
    /// # Errors
    ///
    /// The change-log key cannot be read or created.
    pub fn open(
        drive: Arc<Drive>,
        access: Arc<dyn DriveAccess>,
        index: Index,
        data_dir: &Path,
        app_origin: &str,
    ) -> Result<Self> {
        let keyring = log_keyring(data_dir)?;
        let hub = Hub::new(
            Arc::new(SqlUpdateStore::new(index, Envelope(keyring))),
            Arc::new(DriveFiles::new(Arc::clone(&drive))),
            Arc::new(FormatCodec),
            CollabConfig::default(),
        );
        let authorizer = Arc::new(DriveAuthorizer {
            drive,
            access,
            origin: app_origin.trim_end_matches('/').to_owned(),
        });
        Ok(Self { hub, authorizer })
    }

    /// The router to mount at `/collab`.
    pub fn router(&self) -> Router {
        vellum_collab::ws::router(self.hub.clone(), Arc::clone(&self.authorizer))
    }

    /// Saves every open page and closes its connections.
    pub async fn shutdown(&self) {
        self.hub.shutdown().await;
    }
}

/// The router served at `/collab` when no workspace index is configured.
pub fn unavailable_router() -> Router {
    Router::new().fallback(|request: axum::extract::Request| async move {
        let id = request
            .extensions()
            .get::<RequestId>()
            .copied()
            .unwrap_or_else(RequestId::generate);
        ErrorResponse::of(
            ErrorKind::Unavailable,
            "live editing needs the workspace index",
            id,
        )
        .into_response()
    })
}

/// The KEK that seals the change log, created on first start.
fn log_keyring(data_dir: &Path) -> Result<LocalKeyring> {
    let bytes = crate::keys::key_file(data_dir, LOG_KEY_FILE)?;
    let array: [u8; 32] = bytes
        .get(..32)
        .and_then(|b| b.try_into().ok())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "the change-log key is too short"))?;
    let kek =
        KekId::new("collab-log-1").map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    Ok(LocalKeyring::new(kek, SecretKey::from_bytes(array)))
}

/// The page as the room last saw it: what three-way merges start from.
#[derive(Debug, Clone)]
struct Seen {
    base: Base,
}

/// Page files through the drive.
#[derive(Debug)]
pub struct DriveFiles {
    drive: Arc<Drive>,
    seen: Mutex<HashMap<RoomKey, Seen>>,
}

impl DriveFiles {
    /// Reads and saves pages through `drive`.
    #[must_use]
    pub fn new(drive: Arc<Drive>) -> Self {
        Self {
            drive,
            seen: Mutex::default(),
        }
    }

    fn remember(&self, room: RoomKey, base: Base) {
        if let Ok(mut seen) = self.seen.lock() {
            let _ = seen.insert(room, Seen { base });
        }
    }

    fn recall(&self, room: RoomKey) -> Option<Base> {
        self.seen
            .lock()
            .ok()
            .and_then(|seen| seen.get(&room).map(|s| s.base.clone()))
    }

    /// The page's current bytes and etag, checked to be a page of the room's
    /// space.
    async fn current(&self, room: RoomKey) -> Result<Option<Base>> {
        match self
            .drive
            .read(&Actor::System, ObjectId::from(room.doc))
            .await
        {
            Ok((node, blob)) if node.space == room.space && node.kind == NodeKind::Page => {
                Ok(Some(Base {
                    bytes: blob.bytes,
                    etag: Some(blob.meta.etag),
                }))
            }
            Ok(_) | Err(DriveError::NotFound(_)) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }
}

#[async_trait]
impl DocumentFiles for DriveFiles {
    async fn read_markdown(&self, room: RoomKey) -> Result<Option<String>> {
        let Some(base) = self.current(room).await? else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&base.bytes).into_owned();
        self.remember(room, base);
        Ok(Some(text))
    }

    async fn write_version(&self, room: RoomKey, version: MarkdownVersion) -> Result<()> {
        // A room rebuilt from the change log never read the file: start the
        // merge from the file as it is now.
        let base = match self.recall(room) {
            Some(base) => base,
            None => self
                .current(room)
                .await?
                .ok_or_else(|| Error::new(ErrorKind::NotFound, "the page no longer exists"))?,
        };
        let content = Bytes::from(version.markdown.into_bytes());
        let actor = version
            .editors
            .first()
            .map_or(Actor::System, |user| Actor::User { user: *user });
        let id = ObjectId::from(room.doc);
        let saved = match self
            .drive
            .save(&actor, id, base.clone(), content.clone())
            .await
        {
            // Edit rights can be revoked while a room is open. The edits
            // were accepted under the rights held at the time, so they are
            // saved as Vellum's own write.
            Err(DriveError::PermissionDenied) if actor != Actor::System => {
                tracing::warn!(%room, "editor lost edit access; saving the version as Vellum");
                self.drive
                    .save(&Actor::System, id, base.clone(), content.clone())
                    .await
            }
            other => other,
        }
        .map_err(Error::from)?;
        let next = match saved.saved {
            // The file holds exactly what the room wrote: merge from it.
            Saved::Written(_) => match self.current(room).await? {
                Some(now) if now.bytes == content => now,
                _ => Base {
                    bytes: content,
                    etag: base.etag,
                },
            },
            // The file also holds changes made outside the room. Keep the
            // room's text as the base and the old etag, so the next save
            // merges again instead of dropping those changes.
            Saved::Merged(_) => Base {
                bytes: content,
                etag: base.etag,
            },
            // The room's text went to a conflict copy and the page kept
            // the other change. Carry on from the page as stored.
            Saved::Conflicted { copy } => {
                tracing::warn!(%room, copy = %copy.id, "live edit conflicted; saved as a copy");
                self.current(room).await?.unwrap_or(base)
            }
        };
        self.remember(room, next);
        Ok(())
    }
}

/// Who may join a page's room, and whether they may edit.
#[derive(Debug)]
pub struct DriveAuthorizer {
    drive: Arc<Drive>,
    access: Arc<dyn DriveAccess>,
    origin: String,
}

impl DriveAuthorizer {
    /// Browsers send cookies with cross-site WebSocket handshakes that
    /// carry no CSRF token, so a cookie session must come from the app.
    fn check_origin(&self, request: &Parts, principal: &Principal) -> Result<()> {
        if principal.kind() != CredentialKind::Session {
            return Ok(());
        }
        let origin = request
            .headers
            .get(header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(|o| o.trim_end_matches('/'));
        // Hosts are case-insensitive; the API's CSRF check compares the
        // same way, so a dev proxy that keeps the browser's Origin works
        // for both.
        if origin.is_some_and(|o| o.eq_ignore_ascii_case(&self.origin)) {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "live editing is only open to the Vellum app",
            ))
        }
    }
}

#[async_trait]
impl Authorizer for DriveAuthorizer {
    async fn authorize(&self, request: &Parts, room: RoomKey) -> Result<(UserId, Access)> {
        let principal = request
            .extensions
            .get::<Principal>()
            .cloned()
            .unwrap_or_else(Principal::anonymous);
        if principal.kind() == CredentialKind::ShareLink {
            return Err(Error::new(
                ErrorKind::PermissionDenied,
                "share links cannot join live editing",
            ));
        }
        let user = principal.require_user()?;
        principal.require_scope(Scope::Read)?;
        self.check_origin(request, &principal)?;
        let id = ObjectId::from(room.doc);
        let node = self
            .drive
            .get(&Actor::User { user }, id)
            .await
            .map_err(Error::from)?;
        if node.space != room.space || node.kind != NodeKind::Page {
            return Err(Error::new(ErrorKind::NotFound, "page not found"));
        }
        let may_edit = principal.scopes().allows(Scope::WriteDirect)
            && self
                .access
                .check(user, room.space, id, Permission::Edit)
                .await
                .map_err(Error::from)?;
        Ok((
            user,
            if may_edit {
                Access::Write
            } else {
                Access::Read
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request;
    use vellum_collab::VersionReason;
    use vellum_contracts::ScopeSet;
    use vellum_drive::memory::{MemoryAccess, MemoryIndex, Role};
    use vellum_drive::{Node, SpaceKind};
    use vellum_kernel::{DocId, SpaceId, Timestamp};
    use vellum_store::{BlobStore, LocalStore, Precondition};

    use super::*;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    const ORIGIN: &str = "https://vellum.northstar.dev";

    struct World {
        _dir: tempfile::TempDir,
        store: Arc<LocalStore>,
        drive: Arc<Drive>,
        access: Arc<MemoryAccess>,
        owner: UserId,
        page: Node,
    }

    impl World {
        async fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
            let dir = tempfile::tempdir()?;
            let store = Arc::new(LocalStore::open(dir.path()).await?);
            let index = Arc::new(MemoryIndex::new());
            let access = Arc::new(MemoryAccess::new(index.clone()));
            let drive = Arc::new(Drive::new(store.clone(), index, access.clone()));
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
                store,
                drive,
                access,
                owner,
                page,
            })
        }

        fn room(&self) -> RoomKey {
            RoomKey {
                space: self.page.space,
                doc: DocId::from(self.page.id),
            }
        }

        fn authorizer(&self) -> DriveAuthorizer {
            DriveAuthorizer {
                drive: Arc::clone(&self.drive),
                access: self.access.clone(),
                origin: ORIGIN.to_owned(),
            }
        }

        async fn file(&self) -> std::result::Result<String, Box<dyn std::error::Error>> {
            let (_, blob) = self.drive.read(&Actor::System, self.page.id).await?;
            Ok(String::from_utf8(blob.bytes.to_vec())?)
        }
    }

    fn version(markdown: &str, editor: UserId) -> MarkdownVersion {
        MarkdownVersion {
            markdown: markdown.to_owned(),
            reason: VersionReason::Idle,
            editors: vec![editor],
        }
    }

    fn upgrade(principal: Option<Principal>, origin: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri("/collab/x/y");
        if let Some(origin) = origin {
            builder = builder.header(header::ORIGIN, origin);
        }
        let (mut parts, ()) = builder.body(()).unwrap_or_default().into_parts();
        if let Some(principal) = principal {
            let _ = parts.extensions.insert(principal);
        }
        parts
    }

    fn session(user: UserId) -> Principal {
        Principal::user(
            user,
            CredentialKind::Session,
            ScopeSet::from_scopes(Scope::ALL),
            Timestamp::now(),
        )
    }

    #[tokio::test]
    async fn a_room_reads_and_saves_the_page_file() -> TestResult {
        let world = World::new().await?;
        let files = DriveFiles::new(Arc::clone(&world.drive));
        let opened = files.read_markdown(world.room()).await?.ok_or("no file")?;
        assert!(opened.contains("Ship it."), "{opened}");

        let edited = opened.replace("Ship it.", "Ship it on Monday.");
        files
            .write_version(world.room(), version(&edited, world.owner))
            .await?;
        assert_eq!(world.file().await?, edited);

        // A second version still writes straight through.
        let again = edited.replace("Monday", "Tuesday");
        files
            .write_version(world.room(), version(&again, world.owner))
            .await?;
        assert_eq!(world.file().await?, again);
        Ok(())
    }

    #[tokio::test]
    async fn edits_made_outside_the_room_are_merged_not_overwritten() -> TestResult {
        let world = World::new().await?;
        let files = DriveFiles::new(Arc::clone(&world.drive));
        let opened = files.read_markdown(world.room()).await?.ok_or("no file")?;

        // Someone edits the file directly while the room is open.
        let outside = format!("{opened}\nAdded in a text editor.\n");
        let _meta = world
            .store
            .put(&world.page.path, Bytes::from(outside), Precondition::None)
            .await?;

        let edited = opened.replace("Ship it.", "Ship it on Monday.");
        files
            .write_version(world.room(), version(&edited, world.owner))
            .await?;
        let merged = world.file().await?;
        assert!(merged.contains("Ship it on Monday."), "{merged}");
        assert!(merged.contains("Added in a text editor."), "{merged}");

        // The next version keeps the outside edit too.
        let again = edited.replace("Monday", "Tuesday");
        files
            .write_version(world.room(), version(&again, world.owner))
            .await?;
        let merged = world.file().await?;
        assert!(merged.contains("Ship it on Tuesday."), "{merged}");
        assert!(merged.contains("Added in a text editor."), "{merged}");
        Ok(())
    }

    #[tokio::test]
    async fn a_page_from_another_space_has_no_file() -> TestResult {
        let world = World::new().await?;
        let files = DriveFiles::new(Arc::clone(&world.drive));
        let elsewhere = RoomKey {
            space: SpaceId::generate(),
            doc: DocId::from(world.page.id),
        };
        assert_eq!(files.read_markdown(elsewhere).await?, None);
        Ok(())
    }

    #[tokio::test]
    async fn editors_write_viewers_read_and_strangers_are_refused() -> TestResult {
        let world = World::new().await?;
        let auth = world.authorizer();
        let room = world.room();

        let (user, access) = auth
            .authorize(&upgrade(Some(session(world.owner)), Some(ORIGIN)), room)
            .await?;
        assert_eq!((user, access), (world.owner, Access::Write));

        let viewer = UserId::generate();
        world.access.grant(viewer, world.page.space, Role::Viewer);
        let (_, access) = auth
            .authorize(&upgrade(Some(session(viewer)), Some(ORIGIN)), room)
            .await?;
        assert_eq!(access, Access::Read);

        let stranger = UserId::generate();
        let refused = auth
            .authorize(&upgrade(Some(session(stranger)), Some(ORIGIN)), room)
            .await;
        assert!(refused.is_err());

        let anonymous = auth.authorize(&upgrade(None, Some(ORIGIN)), room).await;
        assert_eq!(
            anonymous.map(|_| ()).map_err(|e| e.kind()),
            Err(ErrorKind::Unauthenticated)
        );
        Ok(())
    }

    #[tokio::test]
    async fn a_session_from_another_site_is_refused() -> TestResult {
        let world = World::new().await?;
        let auth = world.authorizer();
        for origin in [Some("https://evil.example"), None] {
            let refused = auth
                .authorize(&upgrade(Some(session(world.owner)), origin), world.room())
                .await;
            assert_eq!(
                refused.map(|_| ()).map_err(|e| e.kind()),
                Err(ErrorKind::PermissionDenied),
                "{origin:?}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn the_app_origin_is_matched_like_the_api_matches_it() -> TestResult {
        let world = World::new().await?;
        let auth = world.authorizer();
        for origin in [
            ORIGIN,
            "https://vellum.northstar.dev/",
            "https://Vellum.Northstar.dev",
        ] {
            let (user, _) = auth
                .authorize(
                    &upgrade(Some(session(world.owner)), Some(origin)),
                    world.room(),
                )
                .await?;
            assert_eq!(user, world.owner, "{origin}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_read_only_token_cannot_edit() -> TestResult {
        let world = World::new().await?;
        let token = Principal::user(
            world.owner,
            CredentialKind::ApiToken,
            ScopeSet::from_scopes([Scope::Read]),
            Timestamp::now(),
        );
        // Tokens carry no ambient authority, so no Origin is needed.
        let (_, access) = world
            .authorizer()
            .authorize(&upgrade(Some(token), None), world.room())
            .await?;
        assert_eq!(access, Access::Read);
        Ok(())
    }

    #[tokio::test]
    async fn a_page_under_the_wrong_space_is_not_found() -> TestResult {
        let world = World::new().await?;
        let room = RoomKey {
            space: SpaceId::generate(),
            doc: DocId::from(world.page.id),
        };
        let refused = world
            .authorizer()
            .authorize(&upgrade(Some(session(world.owner)), Some(ORIGIN)), room)
            .await;
        assert_eq!(
            refused.map(|_| ()).map_err(|e| e.kind()),
            Err(ErrorKind::NotFound)
        );
        Ok(())
    }
}
