// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Private spaces on a server, end to end over HTTP: two devices enrol
//! through the real gateway, one seals a page, the other reseals after a
//! new device joins, and the private agent reads the space remotely.

use std::sync::Arc;

use bytes::Bytes;
use vellum_api::auth::{Authenticated, Authenticator, RequestMeta};
use vellum_api::config::GatewayConfig;
use vellum_api::gateway::GatewayBuilder;
use vellum_contracts::{BoxFuture, CredentialKind, Principal, ScopeSet, Service};
use vellum_crypto::recipients::{RecipientSecret, seal_for};
use vellum_drive::memory::{MemoryAccess, MemoryIndex, Role};
use vellum_drive::{DriveIndex as _, Space, SpaceKind, SpaceSettings};
use vellum_kernel::{Error, ErrorKind, SpaceId, Timestamp, UserId};
use vellum_private::{DeviceIdentity, PrivateAgent, PrivateSpace};
use vellum_store::{BlobPath, BlobStore, LocalStore, Precondition};

use crate::pins::Pins;
use crate::private_remote::{E2eClient, RemoteSpaces, UNTRUSTED, enrol, reseal};

const OWNER: UserId = UserId::from_u128(1);
const MEMBER: UserId = UserId::from_u128(2);
const SPACE: &str = "01JDB0ARD00000000000000000";
const PAGE: &str = "---\nid: 01JDB0ARDP1AN0000000000000\ntitle: Q4 plan\nvellum: 1\n---\n\n# Q4 ^blk-q4000001\n\nRaise the seed round in December.\n";

/// `Bearer owner` and `Bearer member`, both freshly signed in.
struct Tokens;

impl Authenticator for Tokens {
    fn authenticate<'a>(
        &'a self,
        request: &'a RequestMeta,
    ) -> BoxFuture<'a, Result<Authenticated, Error>> {
        let header = request
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        let user = |id| {
            Ok(Authenticated {
                principal: Principal::user(
                    id,
                    CredentialKind::ApiToken,
                    ScopeSet::FULL,
                    Timestamp::now(),
                ),
                csrf_binding: None,
            })
        };
        let result = match header {
            "Bearer owner" => user(OWNER),
            "Bearer member" => user(MEMBER),
            "" => Ok(Authenticated::anonymous()),
            _ => Err(Error::new(ErrorKind::Unauthenticated, "invalid token")),
        };
        Box::pin(async move { result })
    }
}

struct Server {
    _dir: tempfile::TempDir,
    url: String,
    store: Arc<LocalStore>,
}

async fn server() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let space: SpaceId = SPACE.parse().unwrap();
    let root = BlobPath::new("drives/board").unwrap();
    let index = Arc::new(MemoryIndex::new());
    index
        .put_space(&Space {
            id: space,
            kind: SpaceKind::SharedDrive,
            slug: "board".into(),
            name: "Board".into(),
            owner: None,
            settings: SpaceSettings::default(),
            root: root.clone(),
        })
        .await
        .unwrap();
    let _ = store
        .put(
            &root.join(".vellum-folder.yaml").unwrap(),
            Bytes::from(format!("id: {SPACE}\n")),
            Precondition::None,
        )
        .await
        .unwrap();
    let access = Arc::new(MemoryAccess::new(index.clone()));
    access.grant(OWNER, space, Role::Manager);
    access.grant(MEMBER, space, Role::Editor);
    let blob: Arc<dyn BlobStore> = store.clone();
    let index: Arc<dyn vellum_drive::DriveIndex> = index;
    let e2e = crate::e2e::service(&blob, &index, access);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let gateway = GatewayBuilder::new(GatewayConfig::new(url.clone(), vec![7; 32]), "1.0.0-test")
        .service(Arc::new(e2e) as Arc<dyn Service>)
        .authenticator(Arc::new(Tokens))
        .build()
        .unwrap();
    let router = gateway.router();
    drop(tokio::spawn(
        async move { axum::serve(listener, router).await },
    ));
    Server {
        _dir: dir,
        url,
        store,
    }
}

fn device() -> (RecipientSecret, DeviceIdentity) {
    let secret = RecipientSecret::generate().unwrap();
    (secret.clone(), DeviceIdentity::from_secret(secret))
}

#[tokio::test]
async fn devices_enrol_seal_reseal_and_search_through_the_server() {
    let server = server().await;
    let space: SpaceId = SPACE.parse().unwrap();
    let owner = E2eClient::new(&server.url, "owner").unwrap();
    let member = E2eClient::new(&server.url, "member").unwrap();
    let (laptop_secret, laptop) = device();
    let (_, phone) = device();

    // The owner makes the space private by enrolling their laptop, then
    // uploads a page sealed to it.
    let (laptop_dir, phone_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let laptop_pins = Pins::new(laptop_dir.path().to_owned());
    let phone_pins = Pins::new(phone_dir.path().to_owned());
    let first = enrol(&owner, &laptop, space, "Owner's laptop", &laptop_pins)
        .await
        .unwrap();
    assert_eq!(first.fingerprint, laptop.fingerprint_hex());
    let sealed = seal_for(&[laptop_secret.public()], PAGE.as_bytes()).unwrap();
    let _ = owner.put(space, "plan.md", &sealed, None).await.unwrap();

    // A plaintext page is never handed out by the e2e API.
    let _ = server
        .store
        .put(
            &BlobPath::new("drives/board/readme.md").unwrap(),
            Bytes::from_static(b"# readable"),
            Precondition::None,
        )
        .await
        .unwrap();
    assert_eq!(
        member.get(space, "readme.md").await.unwrap_err().status,
        409
    );

    // The member's phone joins. Until a device that can open the page
    // reseals it, the phone cannot read it.
    let phone_key = enrol(&member, &phone, space, "Member's phone", &phone_pins)
        .await
        .unwrap();
    // The phone cannot open the page yet, but it seals the readable page
    // left from before the space turned private.
    let report = reseal(&member, &phone, space, &phone_pins, &[])
        .await
        .unwrap();
    assert_eq!(
        (
            report.resealed,
            report.converted,
            report.not_for_this_device
        ),
        (0, 1, 1)
    );
    assert!(!report.first_use, "enrolling pinned the keys it found");
    let stale = seal_for(&[laptop_secret.public()], b"x").unwrap();
    let (_, etag) = owner.get(space, "plan.md").await.unwrap();
    assert_eq!(
        owner
            .put(space, "plan.md", &stale, Some(&etag))
            .await
            .unwrap_err()
            .status,
        409,
        "an upload that leaves out the new device is refused"
    );

    // The laptop has not seen the phone's key before: it refuses to seal
    // to it until the owner trusts its fingerprint.
    let refused = reseal(&owner, &laptop, space, &laptop_pins, &[])
        .await
        .unwrap_err();
    assert!(refused.starts_with(UNTRUSTED), "{refused}");
    assert!(refused.contains(&phone_key.fingerprint), "{refused}");
    let report = reseal(
        &owner,
        &laptop,
        space,
        &laptop_pins,
        std::slice::from_ref(&phone_key.fingerprint),
    )
    .await
    .unwrap();
    assert_eq!((report.resealed, report.current), (1, 1));
    let (bytes, _) = member.get(space, "plan.md").await.unwrap();
    assert_eq!(phone.open(&bytes).unwrap(), PAGE.as_bytes());
    let again = reseal(&owner, &laptop, space, &laptop_pins, &[])
        .await
        .unwrap();
    assert_eq!(
        (again.resealed, again.current),
        (0, 2),
        "trusted keys stay trusted"
    );

    // The phone's private agent reads the space from the server.
    let store: Arc<dyn BlobStore> = Arc::new(RemoteSpaces::new(member));
    let root = BlobPath::new(SPACE).unwrap();
    let opened = PrivateSpace::open(store.as_ref(), &phone, "board", root)
        .await
        .unwrap();
    assert_eq!(opened.id, space);
    let agent = PrivateAgent::new(phone, store, vec![opened]).unwrap();
    let pass = agent.refresh().await.unwrap();
    assert_eq!(pass.opened, 1);
    assert_eq!(pass.unreadable, 1, "the converted readme has no page ID");
}

#[tokio::test]
async fn bad_server_urls_and_tokens_are_refused_before_any_call() {
    assert!(E2eClient::new("http://vellum.northstar.dev", "t").is_err());
    assert!(E2eClient::new("ftp://x", "t").is_err());
    assert!(E2eClient::new("https://vellum.northstar.dev", " ").is_err());
    assert!(E2eClient::new("http://127.0.0.1:9", "t").is_ok());
    let server = server().await;
    let stranger = E2eClient::new(&server.url, "nobody").unwrap();
    let failure = stranger.keys(SPACE.parse().unwrap()).await.unwrap_err();
    assert_eq!(failure.status, 401);
}
