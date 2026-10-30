// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Sharing (#95, R-SE-3): grants to people and groups, and share links
//! that expire, can be revoked and can take a password. Runs on the real
//! permission engine, in memory and all in SQLite as `vellum serve` runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use async_trait::async_trait;
use vellum_authz::{Authz, AuthzStore, MemoryStore, SqlStore};
use vellum_db::{Index, IndexConfig};
use vellum_drive::authz::{AuthzAccess, TreeSync};
use vellum_drive::memory::MemoryIndex;
use vellum_drive::sharing::sql::SqlShareLinks;
use vellum_drive::sharing::{
    Grant, Grantee, LinkPasswords, MemoryShareLinks, Opened, Role, ShareLinks, Sharing,
};
use vellum_drive::sql::SqlIndex;
use vellum_drive::{Drive, DriveError, DriveIndex, DriveResult, SpaceKind};
use vellum_kernel::{Actor, ShareToken, Timestamp, UserId};
use vellum_store::LocalStore;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Passwords "hashed" by prefixing; enough to test the flow, not the hash.
#[derive(Debug)]
struct PlainPasswords;

#[async_trait]
impl LinkPasswords for PlainPasswords {
    async fn hash(&self, password: String) -> DriveResult<String> {
        Ok(format!("plain:{password}"))
    }

    async fn verify(&self, password: String, stored: String) -> bool {
        stored == format!("plain:{password}")
    }
}

const DAY_MS: i64 = 86_400_000;

#[tokio::test]
async fn sharing_in_memory() -> TestResult {
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    sharing_with(
        Arc::new(MemoryIndex::new()),
        authz,
        Arc::new(MemoryShareLinks::default()),
    )
    .await
}

#[tokio::test]
async fn sharing_all_in_sqlite() -> TestResult {
    let dir = tempfile::tempdir()?;
    let db = Index::open(&IndexConfig::sqlite(dir.path().join("index.sqlite"), None)).await?;
    let authz = Arc::new(Authz::new(SqlStore::new(db.clone())));
    sharing_with(
        Arc::new(SqlIndex::new(db.clone())),
        authz,
        Arc::new(SqlShareLinks::new(db)),
    )
    .await
}

#[expect(clippy::too_many_lines, reason = "one story, start to end")]
async fn sharing_with<S: AuthzStore + TreeSync + 'static>(
    index: Arc<dyn DriveIndex>,
    authz: Arc<Authz<S>>,
    links: Arc<dyn ShareLinks>,
) -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let access = Arc::new(AuthzAccess::new(authz.clone(), index.clone()));
    let drive = Arc::new(Drive::new(store, index, access.clone()));
    let clock = Arc::new(AtomicI64::new(Timestamp::now().unix_ms()));
    let now = Arc::clone(&clock);
    let sharing = Sharing::new(
        Arc::clone(&drive),
        access,
        Arc::clone(&links),
        Arc::new(PlainPasswords),
    )
    .with_public_origin(Some("https://pages.northstar.dev/".into()))
    .with_clock(Arc::new(move || {
        Timestamp::from_unix_ms(now.load(Ordering::SeqCst)).unwrap_or_else(|_| Timestamp::now())
    }));
    assert_eq!(sharing.public_origin(), Some("https://pages.northstar.dev"));
    let at = |offset: i64| Timestamp::from_unix_ms(clock.load(Ordering::SeqCst) + offset);

    let (alice_id, bob_id, carol_id) = (UserId::generate(), UserId::generate(), UserId::generate());
    let (alice, bob) = (Actor::User { user: alice_id }, Actor::User { user: bob_id });
    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let folder = drive.create_folder(&alice, eng.id, None, "Plans").await?;
    let page = drive
        .create_page(&alice, eng.id, Some(folder.id), "Roadmap", "Ship it.")
        .await?;

    // Nobody but the owner may see who has access, or share.
    assert!(matches!(
        sharing.state(&bob, page.id).await,
        Err(DriveError::PermissionDenied | DriveError::NotFound(_))
    ));
    let state = sharing.state(&alice, page.id).await?;
    assert!(state.links.is_empty());

    // Alice makes Bob an editor; Bob can now edit but not share.
    let edit = Grant {
        grantee: Grantee::User(bob_id),
        role: Role::Editor,
        expires_at: None,
    };
    sharing.share(&alice, page.id, &edit).await?;
    assert!(drive.read(&bob, page.id).await.is_ok());
    assert!(matches!(
        sharing.state(&bob, page.id).await,
        Err(DriveError::PermissionDenied)
    ));
    let carol_view = Grant {
        grantee: Grantee::User(carol_id),
        role: Role::Viewer,
        expires_at: None,
    };
    assert!(matches!(
        sharing.share(&bob, page.id, &carol_view).await,
        Err(DriveError::PermissionDenied)
    ));
    let grants = sharing.state(&alice, page.id).await?.grants;
    assert!(
        grants
            .iter()
            .any(|g| g.grantee == Grantee::User(bob_id) && g.role == Role::Editor)
    );

    // A new role replaces the old one rather than stacking.
    let view = Grant {
        role: Role::Viewer,
        ..edit.clone()
    };
    sharing.share(&alice, page.id, &view).await?;
    let bobs: Vec<Role> = sharing
        .state(&alice, page.id)
        .await?
        .grants
        .iter()
        .filter(|g| g.grantee == Grantee::User(bob_id))
        .map(|g| g.role)
        .collect();
    assert_eq!(bobs, vec![Role::Viewer]);

    // Ownership and one's own access cannot be changed here; nor can a
    // grant end in the past.
    let own = Grant {
        grantee: Grantee::User(carol_id),
        role: Role::Owner,
        expires_at: None,
    };
    assert!(matches!(
        sharing.share(&alice, page.id, &own).await,
        Err(DriveError::Invalid(_))
    ));
    let me = Grant {
        grantee: Grantee::User(alice_id),
        ..carol_view.clone()
    };
    assert!(matches!(
        sharing.share(&alice, page.id, &me).await,
        Err(DriveError::Invalid(_))
    ));
    let past = Grant {
        expires_at: Some(at(-1)?),
        ..carol_view.clone()
    };
    assert!(matches!(
        sharing.share(&alice, page.id, &past).await,
        Err(DriveError::Invalid(_))
    ));

    // Removing Bob takes his access away.
    sharing
        .unshare(&alice, page.id, Grantee::User(bob_id))
        .await?;
    assert!(drive.read(&bob, page.id).await.is_err());

    // A link opens the page; the token is stored only as its hash.
    let link = sharing.create_link(&alice, page.id, None, None).await?;
    assert_eq!(link.token.expose().len(), ShareToken::LEN);
    let record = links.get(link.info.id).await?.ok_or("no record")?;
    assert_eq!(record.token_hash, link.token.hash());
    assert!(matches!(sharing.open(&link.token, None).await?, Opened::Page(n) if n.id == page.id));

    // A made-up token, or one a character off, finds nothing.
    let made_up = ShareToken::generate()?;
    assert!(matches!(
        sharing.open(&made_up, None).await,
        Err(DriveError::NotFound(_))
    ));

    // Folders have no links; viewers cannot make them.
    assert!(matches!(
        sharing.create_link(&alice, folder.id, None, None).await,
        Err(DriveError::Invalid(_))
    ));
    assert!(
        sharing
            .create_link(&bob, page.id, None, None)
            .await
            .is_err()
    );

    // Revoked: never opens again, and the list says so.
    sharing.revoke_link(&alice, link.info.id).await?;
    assert!(matches!(
        sharing.open(&link.token, None).await,
        Err(DriveError::NotFound(_))
    ));
    let listed = sharing.state(&alice, page.id).await?.links;
    assert!(listed.iter().any(|l| l.id == link.info.id && !l.live));

    // Password: asked for, refused when wrong, opens when right.
    assert!(matches!(
        sharing
            .create_link(&alice, page.id, None, Some("short"))
            .await,
        Err(DriveError::Invalid(_))
    ));
    let locked = sharing
        .create_link(&alice, page.id, None, Some("northstar-2026"))
        .await?;
    assert!(locked.info.has_password);
    assert_eq!(
        sharing.open(&locked.token, None).await?,
        Opened::PasswordRequired { retry: false }
    );
    assert_eq!(
        sharing.open(&locked.token, Some("guess")).await?,
        Opened::PasswordRequired { retry: true }
    );
    assert!(matches!(
        sharing.open(&locked.token, Some("northstar-2026")).await?,
        Opened::Page(_)
    ));

    // Expiry: works until the moment, then is gone, by the record and by
    // the engine both.
    let day = sharing
        .create_link(&alice, page.id, Some(at(DAY_MS)?), None)
        .await?;
    assert!(matches!(
        sharing.open(&day.token, None).await?,
        Opened::Page(_)
    ));
    let _ = clock.fetch_add(DAY_MS + 1, Ordering::SeqCst);
    assert!(matches!(
        sharing.open(&day.token, None).await,
        Err(DriveError::NotFound(_))
    ));

    // A trashed page closes its links.
    let open = sharing.create_link(&alice, page.id, None, None).await?;
    let _ = drive.trash(&alice, page.id).await?;
    assert!(matches!(
        sharing.open(&open.token, None).await,
        Err(DriveError::NotFound(_))
    ));
    Ok(())
}

#[test]
fn grantees_and_roles_parse_strictly() {
    let user = UserId::generate();
    assert_eq!(
        Grantee::parse(&format!("user:{user}")).ok(),
        Some(Grantee::User(user))
    );
    assert_eq!(Grantee::User(user).to_string(), format!("user:{user}"));
    assert!(Grantee::parse("user:nope").is_err());
    assert!(Grantee::parse("share_link:01M64VQD838T5RR5F3SYBKTGJH").is_err());
    assert!(Grantee::parse("").is_err());
    assert_eq!(Role::parse("editor").ok(), Some(Role::Editor));
    assert!(Role::parse("owner").is_err());
}

/// Counts how many passwords were checked.
#[derive(Debug, Default)]
struct CountingPasswords(std::sync::atomic::AtomicUsize);

#[async_trait]
impl LinkPasswords for CountingPasswords {
    async fn hash(&self, password: String) -> DriveResult<String> {
        Ok(format!("plain:{password}"))
    }

    async fn verify(&self, password: String, stored: String) -> bool {
        let _ = self.0.fetch_add(1, Ordering::SeqCst);
        stored == format!("plain:{password}")
    }
}

/// RT-042: the public origin has no rate limit, so a link's password could
/// be guessed without end, each guess a full Argon2 hash. After ten wrong
/// guesses in 15 minutes a link stops checking passwords, even the right
/// one, until the window passes.
#[tokio::test]
async fn link_passwords_lock_after_repeated_failures() -> TestResult {
    let dir = tempfile::tempdir()?;
    let store = Arc::new(LocalStore::open(dir.path()).await?);
    let index: Arc<dyn DriveIndex> = Arc::new(MemoryIndex::new());
    let authz = Arc::new(Authz::new(MemoryStore::new()));
    let access = Arc::new(AuthzAccess::new(authz, index.clone()));
    let drive = Arc::new(Drive::new(store, index, access.clone()));
    let clock = Arc::new(AtomicI64::new(Timestamp::now().unix_ms()));
    let now = Arc::clone(&clock);
    let passwords = Arc::new(CountingPasswords::default());
    let sharing = Sharing::new(
        Arc::clone(&drive),
        access,
        Arc::new(MemoryShareLinks::default()),
        passwords.clone(),
    )
    .with_public_origin(Some("https://pages.northstar.dev".into()))
    .with_clock(Arc::new(move || {
        Timestamp::from_unix_ms(now.load(Ordering::SeqCst)).unwrap_or_else(|_| Timestamp::now())
    }));
    let alice = Actor::User {
        user: UserId::generate(),
    };
    let eng = drive
        .create_space(&alice, SpaceKind::SharedDrive, "eng", "Engineering", None)
        .await?;
    let page = drive
        .create_page(&alice, eng.id, None, "Roadmap", "Ship it.")
        .await?;
    let link = sharing
        .create_link(&alice, page.id, None, Some("northstar-2026"))
        .await?;
    let other = sharing
        .create_link(&alice, page.id, None, Some("northstar-2026"))
        .await?;

    for n in 0..100 {
        assert_eq!(
            sharing
                .open(&link.token, Some(&format!("guess-{n}")))
                .await?,
            Opened::PasswordRequired { retry: true }
        );
    }
    assert_eq!(
        passwords.0.load(Ordering::SeqCst),
        10,
        "only ten guesses were hashed"
    );
    assert_eq!(
        sharing.open(&link.token, Some("northstar-2026")).await?,
        Opened::PasswordRequired { retry: true },
        "locked, even for the right password"
    );
    // Other links are not affected.
    assert!(matches!(
        sharing.open(&other.token, Some("northstar-2026")).await?,
        Opened::Page(_)
    ));
    // After the window the right password opens it again.
    let _ = clock.fetch_add(15 * 60 * 1000, Ordering::SeqCst);
    assert!(matches!(
        sharing.open(&link.token, Some("northstar-2026")).await?,
        Opened::Page(_)
    ));
    Ok(())
}
