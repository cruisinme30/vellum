// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Device enrolment and ciphertext over a real local store.

#![expect(clippy::unwrap_used, clippy::expect_used, reason = "tests")]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use bytes::Bytes;
use vellum_crypto::recipients::{RecipientSecret, open_as, seal_for};
use vellum_e2e::{
    DeleteObject, E2e, EnrolDevice, GetObject, ListObjects, Need, PutObject, RevokeDevice,
    SpaceAccess,
};
use vellum_kernel::{Error, ErrorKind, SpaceId, UserId};
use vellum_store::LocalStore;
use vellum_store::{BlobPath, BlobStore, Precondition};

const OWNER: UserId = UserId::from_u128(1);
const MEMBER: UserId = UserId::from_u128(2);
const READER: UserId = UserId::from_u128(3);
const OUTSIDER: UserId = UserId::from_u128(4);

const fn space() -> SpaceId {
    SpaceId::from_u128(77)
}

#[derive(Debug)]
struct Access(BTreeMap<UserId, Need>);

#[async_trait]
impl SpaceAccess for Access {
    async fn root(&self, space_id: SpaceId) -> Result<Option<BlobPath>, Error> {
        Ok((space_id == space()).then(|| BlobPath::new("drives/board").unwrap()))
    }

    async fn allowed(&self, user: UserId, _: SpaceId, need: Need) -> Result<bool, Error> {
        Ok(self.0.get(&user).is_some_and(|held| *held >= need))
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    store: Arc<LocalStore>,
    e2e: E2e,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(LocalStore::open(dir.path()).await.unwrap());
    let access = Access(BTreeMap::from([
        (OWNER, Need::Manage),
        (MEMBER, Need::Edit),
        (READER, Need::View),
    ]));
    let e2e = E2e::new(Arc::clone(&store) as Arc<dyn BlobStore>, Arc::new(access));
    Fixture {
        _dir: dir,
        store,
        e2e,
    }
}

fn device() -> RecipientSecret {
    RecipientSecret::generate().unwrap()
}

fn hex_of(secret: &RecipientSecret) -> String {
    hex::encode(secret.public().to_bytes())
}

fn enrol(secret: &RecipientSecret, label: &str) -> EnrolDevice {
    EnrolDevice {
        space_id: space(),
        public_key: hex_of(secret),
        label: label.to_owned(),
    }
}

fn kind<T: std::fmt::Debug>(result: Result<T, Error>) -> ErrorKind {
    result.expect_err("should fail").kind()
}

fn put(path: &str, envelope: &[u8], if_match: Option<String>) -> PutObject {
    PutObject {
        space_id: space(),
        path: path.to_owned(),
        envelope: STANDARD.encode(envelope),
        if_match,
    }
}

#[tokio::test]
async fn the_first_key_needs_a_manager_then_members_enrol_their_own() {
    let f = fixture().await;
    let (laptop, phone) = (device(), device());
    assert_eq!(
        kind(f.e2e.enrol(MEMBER, enrol(&phone, "phone")).await),
        ErrorKind::PermissionDenied
    );
    let first = f
        .e2e
        .enrol(OWNER, enrol(&laptop, "Owner's laptop"))
        .await
        .unwrap();
    assert!(first.mine);
    let _ = f
        .e2e
        .enrol(READER, enrol(&phone, "Reader's phone"))
        .await
        .unwrap();

    let keys = f.e2e.keys(READER, space()).await.unwrap();
    assert!(!keys.version.is_empty());
    let labels: Vec<_> = keys
        .keys
        .iter()
        .map(|k| (k.label.as_str(), k.mine))
        .collect();
    assert_eq!(
        labels,
        [("Owner's laptop", false), ("Reader's phone", true)]
    );
    assert_eq!(keys.keys[0].fingerprint.len(), 32);

    // The list is a system file: plain YAML holding public keys only.
    let stored = f
        .store
        .get(&vellum_e2e::keys_path(space()).unwrap())
        .await
        .unwrap();
    let text = String::from_utf8(stored.bytes.to_vec()).unwrap();
    assert!(text.contains(&hex_of(&laptop)));
}

#[tokio::test]
async fn bad_keys_labels_and_duplicates_are_refused() {
    let f = fixture().await;
    let laptop = device();
    let mut weak = enrol(&laptop, "x");
    weak.public_key = "00".repeat(32);
    assert_eq!(
        kind(f.e2e.enrol(OWNER, weak).await),
        ErrorKind::InvalidInput
    );
    let mut short = enrol(&laptop, "x");
    short.public_key = "abcd".into();
    assert_eq!(
        kind(f.e2e.enrol(OWNER, short).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.e2e.enrol(OWNER, enrol(&laptop, " \n ")).await),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        kind(f.e2e.enrol(OWNER, enrol(&laptop, &"x".repeat(81))).await),
        ErrorKind::InvalidInput
    );
    let _ = f.e2e.enrol(OWNER, enrol(&laptop, "laptop")).await.unwrap();
    assert_eq!(
        kind(f.e2e.enrol(MEMBER, enrol(&laptop, "same key")).await),
        ErrorKind::Conflict
    );
}

#[tokio::test]
async fn outsiders_see_nothing_and_readers_cannot_write() {
    let f = fixture().await;
    let laptop = device();
    let _ = f.e2e.enrol(OWNER, enrol(&laptop, "laptop")).await.unwrap();
    assert_eq!(
        kind(f.e2e.keys(OUTSIDER, space()).await),
        ErrorKind::NotFound
    );
    assert_eq!(
        kind(f.e2e.keys(OWNER, SpaceId::from_u128(5)).await),
        ErrorKind::NotFound
    );
    let sealed = seal_for(&[laptop.public()], b"# Minutes").unwrap();
    assert_eq!(
        kind(f.e2e.put(READER, put("minutes.md", &sealed, None)).await),
        ErrorKind::PermissionDenied
    );
    assert_eq!(
        kind(f.e2e.put(OUTSIDER, put("minutes.md", &sealed, None)).await),
        ErrorKind::NotFound
    );
}

#[tokio::test]
async fn uploads_must_be_sealed_to_exactly_the_current_keys() {
    let f = fixture().await;
    let (laptop, phone, stranger) = (device(), device(), device());
    let early = seal_for(&[laptop.public()], b"x").unwrap();
    assert_eq!(
        kind(f.e2e.put(MEMBER, put("a.md", &early, None)).await),
        ErrorKind::Conflict,
        "a space with no keys is not private"
    );
    let _ = f.e2e.enrol(OWNER, enrol(&laptop, "laptop")).await.unwrap();
    let _ = f.e2e.enrol(MEMBER, enrol(&phone, "phone")).await.unwrap();

    let missing = seal_for(&[laptop.public()], b"x").unwrap();
    assert_eq!(
        kind(f.e2e.put(MEMBER, put("a.md", &missing, None)).await),
        ErrorKind::Conflict
    );
    let extra = seal_for(&[laptop.public(), phone.public(), stranger.public()], b"x").unwrap();
    assert_eq!(
        kind(f.e2e.put(MEMBER, put("a.md", &extra, None)).await),
        ErrorKind::Conflict
    );
    assert_eq!(
        kind(f.e2e.put(MEMBER, put("a.md", b"# plaintext", None)).await),
        ErrorKind::InvalidInput
    );
    let mut not_base64 = put("a.md", b"", None);
    not_base64.envelope = "%%%".into();
    assert_eq!(
        kind(f.e2e.put(MEMBER, not_base64).await),
        ErrorKind::InvalidInput
    );

    let good = seal_for(&[phone.public(), laptop.public()], b"# Board minutes").unwrap();
    let stored = f
        .e2e
        .put(MEMBER, put("notes/a.md", &good, None))
        .await
        .unwrap();
    assert_eq!(
        kind(f.e2e.put(MEMBER, put("notes/a.md", &good, None)).await),
        ErrorKind::Conflict,
        "creating over an existing object needs if_match"
    );
    let got = f
        .e2e
        .get(
            READER,
            GetObject {
                space_id: space(),
                path: "notes/a.md".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(got.etag, stored.etag);
    let bytes = STANDARD.decode(got.envelope).unwrap();
    assert_eq!(open_as(&laptop, &bytes).unwrap(), b"# Board minutes");

    let newer = seal_for(&[phone.public(), laptop.public()], b"# v2").unwrap();
    let v2 = f
        .e2e
        .put(MEMBER, put("notes/a.md", &newer, Some(stored.etag.clone())))
        .await
        .unwrap();
    assert_eq!(
        kind(
            f.e2e
                .put(MEMBER, put("notes/a.md", &newer, Some(stored.etag)))
                .await
        ),
        ErrorKind::Conflict,
        "a stale version is refused"
    );
    let _ = f
        .e2e
        .delete(
            MEMBER,
            DeleteObject {
                space_id: space(),
                path: "notes/a.md".into(),
                if_match: v2.etag,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn plaintext_and_system_files_are_never_served() {
    let f = fixture().await;
    let laptop = device();
    let _ = f.e2e.enrol(OWNER, enrol(&laptop, "laptop")).await.unwrap();
    for (path, body) in [
        ("drives/board/.vellum-folder.yaml", "id: x\n"),
        ("drives/board/readme.md", "# readable"),
        ("drives/board/.vellum/state.json", "{}"),
    ] {
        let _ = f
            .store
            .put(
                &BlobPath::new(path).unwrap(),
                Bytes::from(body),
                Precondition::None,
            )
            .await
            .unwrap();
    }
    let get = |path: &str| GetObject {
        space_id: space(),
        path: path.into(),
    };
    let manifest = f.e2e.get(READER, get(".vellum-folder.yaml")).await.unwrap();
    assert_eq!(STANDARD.decode(manifest.envelope).unwrap(), b"id: x\n");
    assert_eq!(
        kind(f.e2e.get(READER, get("readme.md")).await),
        ErrorKind::Conflict
    );
    for bad in [
        "../other/a.md",
        ".vellum/state.json",
        "a/../../x.md",
        "/etc/passwd/../..",
    ] {
        assert_ne!(
            f.e2e
                .get(READER, get(bad))
                .await
                .map(|_| ())
                .map_err(|e| e.kind()),
            Ok(()),
            "{bad}"
        );
    }
    let sealed = seal_for(&[laptop.public()], b"x").unwrap();
    assert_eq!(
        kind(
            f.e2e
                .put(MEMBER, put(".vellum/e2e/evil.yaml", &sealed, None))
                .await
        ),
        ErrorKind::InvalidInput
    );

    let listed = f
        .e2e
        .objects(
            READER,
            ListObjects {
                space_id: space(),
                after: None,
                limit: Some(1),
            },
        )
        .await
        .unwrap();
    assert_eq!(listed.objects.len(), 1);
    assert_eq!(listed.objects[0].path, ".vellum-folder.yaml");
    let rest = f
        .e2e
        .objects(
            READER,
            ListObjects {
                space_id: space(),
                after: listed.next,
                limit: None,
            },
        )
        .await
        .unwrap();
    let paths: Vec<_> = rest.objects.iter().map(|o| o.path.as_str()).collect();
    assert_eq!(paths, ["readme.md"], "system files are left out");
    assert!(rest.next.is_none());
}

#[tokio::test]
async fn revocation_rules() {
    let f = fixture().await;
    let (laptop, phone, tablet) = (device(), device(), device());
    let owner_key = f.e2e.enrol(OWNER, enrol(&laptop, "laptop")).await.unwrap();
    let revoke = |fingerprint: &str| RevokeDevice {
        space_id: space(),
        fingerprint: fingerprint.into(),
    };
    assert_eq!(
        kind(f.e2e.revoke(OWNER, revoke(&owner_key.fingerprint)).await),
        ErrorKind::Conflict,
        "the last key stays"
    );
    let phone_key = f.e2e.enrol(MEMBER, enrol(&phone, "phone")).await.unwrap();
    let tablet_key = f.e2e.enrol(READER, enrol(&tablet, "tablet")).await.unwrap();
    assert_eq!(
        kind(f.e2e.revoke(READER, revoke(&phone_key.fingerprint)).await),
        ErrorKind::PermissionDenied,
        "only managers revoke other people's devices"
    );
    assert_eq!(
        kind(f.e2e.revoke(READER, revoke(&"0".repeat(32))).await),
        ErrorKind::NotFound
    );
    let gone = f
        .e2e
        .revoke(MEMBER, revoke(&phone_key.fingerprint.to_uppercase()))
        .await
        .unwrap();
    assert_eq!(gone.removed[0].public_key, hex_of(&phone));
    let _ = f
        .e2e
        .revoke(OWNER, revoke(&tablet_key.fingerprint))
        .await
        .unwrap();
    let keys = f.e2e.keys(OWNER, space()).await.unwrap();
    assert_eq!(keys.keys.len(), 1);
}

#[tokio::test]
async fn concurrent_enrolments_all_land() {
    let f = Arc::new(fixture().await);
    let _ = f.e2e.enrol(OWNER, enrol(&device(), "first")).await.unwrap();
    let tasks: Vec<_> = (0..8)
        .map(|i| {
            let f = Arc::clone(&f);
            tokio::spawn(async move {
                f.e2e
                    .enrol(READER, enrol(&device(), &format!("d{i}")))
                    .await
            })
        })
        .collect();
    let mut ok = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => ok += 1,
            Err(e) => assert_eq!(e.kind(), ErrorKind::Unavailable),
        }
    }
    let keys = f.e2e.keys(OWNER, space()).await.unwrap();
    assert_eq!(keys.keys.len(), ok + 1, "no enrolment is lost");
    assert!(ok >= 1);
}
