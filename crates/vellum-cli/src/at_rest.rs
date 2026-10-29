// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Encryption at rest for the workspace files (brief §11, ADR 0025 D260).
//!
//! Every object `vellum serve` writes is sealed in an envelope (ADR 0002)
//! unless the operator turns it off. The key-encryption key comes from the
//! first of:
//!
//! | Setting | KEK |
//! | --- | --- |
//! | `VELLUM_FILES_PLAINTEXT=1` | none: files are stored as they are |
//! | `VELLUM_KMS_KEY` | a key service: `aws-kms:<region>:<key>`, `gcp-kms:<key name>` or `vault:<mount>/<key>` |
//! | `VELLUM_KEK_PASSPHRASE` or `VELLUM_KEK_PASSPHRASE_FILE` | Argon2id from the passphrase |
//! | (nothing, data folder only) | a random key in `.vellum/keys/files.kek` |
//!
//! A shared bucket needs one of the first three, because replicas cannot
//! share a key file. Reads open sealed and plaintext objects alike, so an
//! existing workspace keeps working and its files are sealed as they are
//! next written.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use vellum_crypto::{
    CryptoError, KekId, KeyProvider, LocalKeyring, PassphraseKdf, SecretKey, WrappedDek,
};
use vellum_kernel::{Config, Error, ErrorKind};
use vellum_kms::RemoteKeys;
use vellum_sealed_store::{PrefixPolicy, Sealed};
use vellum_store::{BlobPath, BlobStore, Precondition, StoreError};
use zeroize::Zeroizing;

/// `1` stores the workspace files without encryption.
pub const PLAINTEXT_VAR: &str = "VELLUM_FILES_PLAINTEXT";

/// The KEK id of a key service.
pub const KMS_VAR: &str = "VELLUM_KMS_KEY";

/// The KEK passphrase itself.
const PASSPHRASE_VAR: &str = vellum_crypto::PASSPHRASE_ENV;

/// A file holding the KEK passphrase (a Docker or Kubernetes secret).
const PASSPHRASE_FILE_VAR: &str = vellum_crypto::PASSPHRASE_FILE_ENV;

/// The key file under `.vellum/keys` for a data folder with no other KEK.
pub const KEY_FILE: &str = "files.kek";

/// The KEK id prefix of [`KEY_FILE`]; a fingerprint of the key follows.
const KEY_FILE_KEK: &str = "local:files.kek";

/// Where the key file's KEK id is recorded in the store.
const KEY_FILE_MARKER: &str = ".vellum/kek-file";

/// Where the passphrase KEK's id (its salt and costs, nothing secret) is
/// kept, so every replica and every restart wraps under the same KEK.
const PASSPHRASE_KEK_PATH: &str = ".vellum/kek";

/// Passphrase KEKs derived and kept in memory, at most. Each distinct
/// salt in the store costs one Argon2id derivation.
const DERIVED_MAX: usize = 8;

/// The most Argon2id memory a KEK id read from a stored object may ask for
/// (four times the default), so a crafted header cannot exhaust memory.
const MAX_M_KIB: u32 = PassphraseKdf::DEFAULT_M_KIB * 4;

fn invalid(what: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::InvalidInput, what.to_string())
}

fn unavailable(what: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, what.to_string())
}

fn on(value: Option<String>) -> bool {
    value.is_some_and(|v| matches!(v.trim(), "1" | "true" | "yes"))
}

/// The key that seals the workspace files.
#[derive(Debug)]
pub enum AtRestKeys {
    /// A key service (`VELLUM_KMS_KEY`).
    Remote(RemoteKeys),
    /// A passphrase (`VELLUM_KEK_PASSPHRASE`).
    Passphrase(PassphraseKeys),
    /// The data folder's key file.
    KeyFile(LocalKeyring),
}

impl AtRestKeys {
    /// Where the KEK lives, for logs: `aws-kms`, `gcp-kms`,
    /// `vault-transit`, `passphrase` or `key-file`.
    #[must_use]
    pub const fn source(&self) -> &'static str {
        match self {
            Self::Remote(remote) => remote.backend(),
            Self::Passphrase(_) => "passphrase",
            Self::KeyFile(_) => "key-file",
        }
    }
}

impl KeyProvider for AtRestKeys {
    fn current_kek(&self) -> KekId {
        match self {
            Self::Remote(p) => p.current_kek(),
            Self::Passphrase(p) => p.current.clone(),
            Self::KeyFile(p) => p.current_kek(),
        }
    }

    async fn wrap(&self, dek: &SecretKey) -> Result<WrappedDek, CryptoError> {
        match self {
            Self::Remote(p) => p.wrap(dek).await,
            Self::Passphrase(p) => {
                let kek = p.kek(&p.current).await?;
                LocalKeyring::new(p.current.clone(), kek).wrap(dek).await
            }
            Self::KeyFile(p) => p.wrap(dek).await,
        }
    }

    async fn unwrap(&self, wrapped: &WrappedDek) -> Result<SecretKey, CryptoError> {
        match self {
            Self::Remote(p) => p.unwrap(wrapped).await,
            Self::Passphrase(p) => {
                let kek = p.kek(&wrapped.kek_id).await?;
                LocalKeyring::new(wrapped.kek_id.clone(), kek)
                    .unwrap(wrapped)
                    .await
            }
            Self::KeyFile(p) => p.unwrap(wrapped).await,
        }
    }
}

/// KEKs derived from one passphrase. The current KEK's salt is fixed for
/// the workspace; any other passphrase KEK id an envelope names is derived
/// on first use, so envelopes written under an old salt still open.
pub struct PassphraseKeys {
    passphrase: Arc<Zeroizing<Vec<u8>>>,
    current: KekId,
    derived: Mutex<BTreeMap<KekId, SecretKey>>,
}

impl std::fmt::Debug for PassphraseKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassphraseKeys")
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl PassphraseKeys {
    /// Keys from `passphrase`, wrapping new DEKs under `current`.
    #[must_use]
    pub fn new(passphrase: Zeroizing<Vec<u8>>, current: KekId) -> Self {
        Self {
            passphrase: Arc::new(passphrase),
            current,
            derived: Mutex::new(BTreeMap::new()),
        }
    }

    async fn kek(&self, id: &KekId) -> Result<SecretKey, CryptoError> {
        if let Some(kek) = self.derived.lock().ok().and_then(|d| d.get(id).cloned()) {
            return Ok(kek);
        }
        let kdf =
            PassphraseKdf::from_kek_id(id).map_err(|_| CryptoError::UnknownKek(id.clone()))?;
        if kdf.m_kib() > MAX_M_KIB {
            return Err(CryptoError::UnknownKek(id.clone()));
        }
        let passphrase = Arc::clone(&self.passphrase);
        // Argon2id takes a noticeable fraction of a second; keep it off the
        // async workers.
        let kek = tokio::task::spawn_blocking(move || kdf.derive(passphrase.as_slice()))
            .await
            .map_err(|_| CryptoError::Kdf)??;
        if let Ok(mut derived) = self.derived.lock()
            && (derived.len() < DERIVED_MAX || derived.contains_key(id))
        {
            let _ = derived.insert(id.clone(), kek.clone());
        }
        Ok(kek)
    }
}

/// The passphrase from `VELLUM_KEK_PASSPHRASE` or the file
/// `VELLUM_KEK_PASSPHRASE_FILE` names (one trailing newline dropped).
fn passphrase(var: &impl Fn(&str) -> Option<String>) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
    let secret = match (var(PASSPHRASE_VAR), var(PASSPHRASE_FILE_VAR)) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => {
            return Err(invalid(format!(
                "set only one of {PASSPHRASE_VAR} and {PASSPHRASE_FILE_VAR}"
            )));
        }
        (Some(value), None) => Zeroizing::new(value.into_bytes()),
        (None, Some(path)) => {
            let mut bytes = Zeroizing::new(
                std::fs::read(&path)
                    .map_err(|e| invalid(format!("cannot read {PASSPHRASE_FILE_VAR}: {e}")))?,
            );
            if bytes.ends_with(b"\n") {
                let _ = bytes.pop();
                if bytes.ends_with(b"\r") {
                    let _ = bytes.pop();
                }
            }
            bytes
        }
    };
    if secret.is_empty() {
        return Err(invalid("the KEK passphrase is empty"));
    }
    Ok(Some(secret))
}

/// The passphrase KEK id kept in `store`, created on the first start. All
/// replicas agree on it: the first write wins and the others read it.
async fn passphrase_kek(store: &dyn BlobStore) -> Result<KekId, Error> {
    let path = BlobPath::new(PASSPHRASE_KEK_PATH).map_err(unavailable)?;
    for _ in 0..3 {
        match store.get(&path).await {
            Ok(blob) => {
                let text = std::str::from_utf8(&blob.bytes)
                    .map_err(|_| unavailable(format!("{PASSPHRASE_KEK_PATH} is not text")))?;
                let id = KekId::new(text.trim())
                    .map_err(|_| unavailable(format!("{PASSPHRASE_KEK_PATH} is damaged")))?;
                let _ = PassphraseKdf::from_kek_id(&id)
                    .map_err(|_| unavailable(format!("{PASSPHRASE_KEK_PATH} is damaged")))?;
                return Ok(id);
            }
            Err(StoreError::NotFound(_)) => {
                let id = PassphraseKdf::generate()
                    .and_then(|kdf| kdf.kek_id())
                    .map_err(|e| unavailable(format!("cannot make the passphrase KEK: {e}")))?;
                match store
                    .put(
                        &path,
                        Bytes::from(id.as_str().to_owned()),
                        Precondition::IfAbsent,
                    )
                    .await
                {
                    Ok(_) => return Ok(id),
                    // Another replica wrote it first; read theirs.
                    Err(StoreError::PreconditionFailed { .. } | StoreError::AlreadyExists(_)) => {}
                    Err(e) => {
                        return Err(unavailable(format!(
                            "cannot save {PASSPHRASE_KEK_PATH}: {e}"
                        )));
                    }
                }
            }
            Err(e) => {
                return Err(unavailable(format!(
                    "cannot read {PASSPHRASE_KEK_PATH}: {e}"
                )));
            }
        }
    }
    Err(unavailable(format!(
        "cannot agree on {PASSPHRASE_KEK_PATH}"
    )))
}

/// Chooses the KEK, or `None` when the files are stored in plaintext.
/// `files` is the store before sealing; `shared` is true for a bucket that
/// several replicas use.
///
/// # Errors
///
/// A malformed or half-configured setting, an unreachable store, or a
/// bucket with no KEK configured.
pub async fn keys(
    config: &Config,
    files: &dyn BlobStore,
    shared: bool,
    var: impl Fn(&str) -> Option<String> + Sync,
) -> Result<Option<AtRestKeys>, Error> {
    let var = |name: &str| var(name).filter(|v| !v.trim().is_empty());
    if on(var(PLAINTEXT_VAR)) {
        tracing::warn!("workspace files are stored without encryption ({PLAINTEXT_VAR})");
        // Files written now are plaintext, so a later switch back on must
        // run the upgrade pass again.
        if let Ok(record) = BlobPath::new(UPGRADED_PATH) {
            match files.delete(&record, Precondition::None).await {
                Ok(()) | Err(StoreError::NotFound(_)) => {}
                // Best effort: an unreachable store fails later, clearly.
                Err(error) => tracing::warn!(%error, "cannot clear {UPGRADED_PATH}"),
            }
        }
        return Ok(None);
    }
    if let Some(kek) = var(KMS_VAR) {
        let remote = RemoteKeys::from_kek_id(kek.trim(), var)
            .map_err(|e| invalid(format!("{KMS_VAR}: {e}")))?;
        return Ok(Some(AtRestKeys::Remote(remote)));
    }
    if let Some(secret) = passphrase(&var)? {
        let current = passphrase_kek(files).await?;
        return Ok(Some(AtRestKeys::Passphrase(PassphraseKeys::new(
            secret, current,
        ))));
    }
    if shared {
        return Err(invalid(format!(
            "files in a shared bucket are encrypted at rest: set {KMS_VAR}, or \
             {PASSPHRASE_VAR} or {PASSPHRASE_FILE_VAR} (the same on every replica), \
             or {PLAINTEXT_VAR}=1 to store them in plaintext"
        )));
    }
    key_file_keys(config, files).await.map(Some)
}

/// The data folder's own key. Its id carries a fingerprint, recorded in
/// the store on first use, so a lost or replaced key file stops startup
/// instead of making a new key that cannot open the existing files.
async fn key_file_keys(config: &Config, files: &dyn BlobStore) -> Result<AtRestKeys, Error> {
    let marker = BlobPath::new(KEY_FILE_MARKER).map_err(unavailable)?;
    let recorded = match files.get(&marker).await {
        Ok(blob) => Some(String::from_utf8_lossy(&blob.bytes).trim().to_owned()),
        Err(StoreError::NotFound(_)) => None,
        Err(e) => return Err(unavailable(format!("cannot read {KEY_FILE_MARKER}: {e}"))),
    };
    let path = config.data_dir.join(".vellum").join("keys").join(KEY_FILE);
    if recorded.is_some() && !path.exists() {
        return Err(invalid(format!(
            "the files in {} are encrypted with a key that is missing: restore {}, \
             or set the key service or passphrase they were encrypted with",
            config.data_dir.display(),
            path.display()
        )));
    }
    let bytes = Zeroizing::new(crate::keys::key_file(&config.data_dir, KEY_FILE)?);
    let key = bytes
        .get(..32)
        .and_then(|k| <[u8; 32]>::try_from(k).ok())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "the files key is too short"))?;
    let digest =
        <sha2::Sha256 as sha2::Digest>::digest([b"vellum/files-kek/v1:".as_slice(), &key].concat());
    let fingerprint = hex::encode(digest.get(..8).unwrap_or_default());
    let id = KekId::new(format!("{KEY_FILE_KEK}:{fingerprint}"))
        .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?;
    match recorded {
        Some(recorded) if recorded != id.as_str() => {
            return Err(invalid(format!(
                "{} is not the key the files were encrypted with ({recorded}); restore the \
                 original key file",
                path.display()
            )));
        }
        Some(_) => {}
        None => {
            let _ = files
                .put(
                    &marker,
                    Bytes::from(id.as_str().to_owned()),
                    Precondition::IfAbsent,
                )
                .await
                .map_err(|e| unavailable(format!("cannot save {KEY_FILE_MARKER}: {e}")))?;
        }
    }
    Ok(AtRestKeys::KeyFile(LocalKeyring::new(
        id,
        SecretKey::from_bytes(key),
    )))
}

/// Reads workspace files from disk for `vellum reindex`, opening sealed
/// ones. E2E ciphertext and plaintext files are returned as they are.
#[derive(Clone)]
pub struct OpeningReader {
    keys: Option<Arc<AtRestKeys>>,
    runtime: tokio::runtime::Handle,
}

impl vellum_db::FileReader for OpeningReader {
    fn read(&self, path: &std::path::Path) -> std::io::Result<Vec<u8>> {
        let bytes = std::fs::read(path)?;
        let Some(keys) = &self.keys else {
            return Ok(bytes);
        };
        if !vellum_crypto::is_envelope(&bytes)
            || vellum_crypto::recipients::is_recipient_envelope(&bytes)
        {
            return Ok(bytes);
        }
        // Called on the reindex's blocking thread, never on a worker.
        self.runtime
            .block_on(vellum_crypto::open(keys.as_ref(), &bytes))
            .map_err(std::io::Error::other)
    }
}

/// The reader for the data folder in `config`, with the keys the server
/// would use. Must be called inside the runtime that runs the reindex.
///
/// # Errors
///
/// As [`keys`].
pub async fn reader(
    config: &Config,
    var: impl Fn(&str) -> Option<String> + Sync,
) -> Result<OpeningReader, Error> {
    let store = vellum_store::LocalStore::open(&config.data_dir)
        .await
        .map_err(|e| unavailable(format!("cannot open the data folder: {e}")))?;
    let keys = keys(config, &store, false, var).await?;
    Ok(OpeningReader {
        keys: keys.map(Arc::new),
        runtime: tokio::runtime::Handle::current(),
    })
}

/// The sealed store type `vellum serve` uses.
pub type SealedFiles<S> = Sealed<S, AtRestKeys, PrefixPolicy>;

/// `store` sealed with `keys`: every object is encrypted, except the
/// store's own key markers, and E2E private-space ciphertext is kept as
/// the clients sealed it. A background pass then seals what an older
/// version stored in plaintext (see [`upgrade`]).
pub fn seal<S: BlobStore + 'static>(store: S, keys: AtRestKeys) -> Arc<dyn BlobStore> {
    tracing::info!(kek = keys.source(), "workspace files are encrypted at rest");
    let sealed = Arc::new(sealed(store, keys));
    let pass = Arc::clone(&sealed);
    drop(tokio::spawn(async move {
        match upgrade(pass.as_ref()).await {
            Ok(report) if report.done => tracing::info!(?report, "workspace files sealed"),
            Ok(report) => tracing::warn!(
                ?report,
                "some files are not sealed yet; the next start retries"
            ),
            Err(error) => {
                tracing::warn!(%error, "sealing existing files failed; the next start retries");
            }
        }
    }));
    sealed
}

/// The sealed store without the background pass.
pub fn sealed<S: BlobStore>(store: S, keys: AtRestKeys) -> SealedFiles<S> {
    let mut policy = PrefixPolicy::encrypt_all();
    for marker in [PASSPHRASE_KEK_PATH, KEY_FILE_MARKER, UPGRADED_PATH] {
        if let Ok(path) = BlobPath::new(marker) {
            // Read raw before the keys exist, so never sealed.
            policy.set(path, true);
        }
    }
    Sealed::new(store, Arc::new(keys), policy)
}

/// Where the KEK id of the last complete [`upgrade`] is recorded.
const UPGRADED_PATH: &str = ".vellum/sealed";

/// The folders whose files Vellum reads only through the store: the
/// drives (pages, books, collections), and the version history and trash
/// that hold their content. Everything else under `.vellum` is Vellum's
/// own state, some of it read straight from disk, so it is left as is.
const UPGRADE_ROOTS: [&str; 4] = ["drives", "spaces", ".vellum/versions", ".vellum/trash"];

/// What an [`upgrade`] pass did.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Upgrade {
    /// Objects looked at.
    pub seen: usize,
    /// Objects rewritten: plaintext sealed, or envelopes re-wrapped under
    /// the current KEK.
    pub rewritten: usize,
    /// Objects that could not be rewritten this time.
    pub failed: usize,
    /// True when nothing was left to do, now or on an earlier start.
    pub done: bool,
}

/// Seals every plaintext file and re-wraps every envelope whose KEK is
/// not the current one, so an upgraded workspace, or one whose key moved,
/// ends up wholly under the current KEK. A concurrent write wins (the
/// rewrite is conditional), and is sealed by that write anyway.
///
/// The KEK id is recorded when a pass completes, so later starts skip the
/// listing until the KEK changes. Every replica may run it; the work is
/// idempotent.
///
/// # Errors
///
/// The store cannot be listed or the record cannot be written.
pub async fn upgrade<S: BlobStore>(sealed: &SealedFiles<S>) -> Result<Upgrade, StoreError> {
    use futures::StreamExt as _;

    let kek = sealed.keys().current_kek();
    let record = BlobPath::new(UPGRADED_PATH)?;
    match sealed.inner().get(&record).await {
        Ok(blob) if blob.bytes.as_ref() == kek.as_str().as_bytes() => {
            return Ok(Upgrade {
                done: true,
                ..Upgrade::default()
            });
        }
        Ok(_) | Err(StoreError::NotFound(_)) => {}
        Err(e) => return Err(e),
    }
    let mut report = Upgrade::default();
    for root in UPGRADE_ROOTS {
        let root = BlobPath::new(root)?;
        let mut entries = sealed.inner().list(Some(&root));
        while let Some(entry) = entries.next().await {
            let entry = match entry {
                Ok(entry) => entry,
                Err(StoreError::NotFound(_)) => break,
                Err(e) => return Err(e),
            };
            report.seen += 1;
            let mut rewritten = Err(());
            for _ in 0..3 {
                match sealed.reseal(&entry.path).await {
                    Ok(changed) => {
                        rewritten = Ok(changed);
                        break;
                    }
                    // Written meanwhile, and sealed by that write; look again.
                    Err(StoreError::PreconditionFailed { .. }) => {}
                    // Deleted meanwhile: nothing to seal.
                    Err(StoreError::NotFound(_)) => {
                        rewritten = Ok(false);
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(path = %entry.path, %error, "cannot seal a file yet");
                        break;
                    }
                }
            }
            match rewritten {
                Ok(true) => report.rewritten += 1,
                Ok(false) => {}
                Err(()) => report.failed += 1,
            }
        }
    }
    if report.failed == 0 {
        let _ = sealed
            .inner()
            .put(
                &record,
                Bytes::from(kek.as_str().to_owned()),
                Precondition::None,
            )
            .await?;
        report.done = true;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vellum_store::LocalStore;

    fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    fn config(dir: &std::path::Path) -> Config {
        let mut config = Config::default();
        config.data_dir = dir.to_path_buf();
        config
    }

    fn path(p: &str) -> BlobPath {
        BlobPath::new(p).unwrap()
    }

    #[tokio::test]
    async fn a_data_folder_is_sealed_by_default_with_its_own_key() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let keys = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(keys.source(), "key-file");
        assert!(dir.path().join(".vellum/keys").join(KEY_FILE).exists());
        let sealed = seal(raw, keys);
        let page = path("spaces/eng/notes.md");
        let _ = sealed
            .put(
                &page,
                Bytes::from_static(b"# Secret plans\n"),
                Precondition::None,
            )
            .await
            .unwrap();
        let on_disk = std::fs::read(dir.path().join("spaces/eng/notes.md")).unwrap();
        assert!(vellum_crypto::is_envelope(&on_disk));
        assert!(!on_disk.windows(6).any(|w| w == b"Secret"));
        assert_eq!(
            &sealed.get(&page).await.unwrap().bytes[..],
            b"# Secret plans\n"
        );

        // A file written before encryption was on still reads.
        std::fs::write(dir.path().join("spaces/eng/old.md"), "# Old\n").unwrap();
        assert_eq!(
            &sealed.get(&path("spaces/eng/old.md")).await.unwrap().bytes[..],
            b"# Old\n"
        );
    }

    #[tokio::test]
    async fn a_lost_or_replaced_key_file_stops_startup() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let first = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .unwrap()
            .unwrap();
        let again = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.current_kek(), again.current_kek());

        let key = dir.path().join(".vellum/keys").join(KEY_FILE);
        std::fs::remove_file(&key).unwrap();
        let lost = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .err()
            .unwrap();
        assert!(lost.to_string().contains("missing"), "{lost}");
        assert!(!key.exists(), "no new key is made over sealed files");

        std::fs::create_dir_all(key.parent().unwrap()).unwrap();
        std::fs::write(&key, [9_u8; 32]).unwrap();
        let replaced = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .err()
            .unwrap();
        assert!(replaced.to_string().contains("not the key"), "{replaced}");
    }

    /// A data folder as an older Vellum left it: plaintext pages and
    /// history, a private-space page sealed by a device, and server state
    /// read straight from disk.
    fn old_workspace(dir: &std::path::Path) -> Vec<u8> {
        let write = |rel: &str, bytes: &[u8]| {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        };
        write(
            "drives/shared/eng/Runbook.md",
            b"# Runbook\nRestart the pump.\n",
        );
        write("drives/shared/eng/.vellum-folder.yaml", b"id: eng\n");
        write(".vellum/versions/doc/v1.md", b"# Runbook\nOld text.\n");
        write(".vellum/tasks/secrets.json", b"{}");
        let device = vellum_crypto::recipients::RecipientSecret::generate().unwrap();
        let private =
            vellum_crypto::recipients::seal_for(&[device.public()], b"# Diary\n").unwrap();
        write("drives/users/ana/Private/Diary.md", &private);
        private
    }

    #[tokio::test]
    async fn an_upgraded_workspace_is_sealed_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let private = old_workspace(dir.path());
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let keys = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .unwrap()
            .unwrap();
        let store = sealed(raw, keys);

        let report = upgrade(&store).await.unwrap();
        assert_eq!((report.seen, report.rewritten, report.failed), (4, 3, 0));
        assert!(report.done);

        let disk = |rel: &str| std::fs::read(dir.path().join(rel)).unwrap();
        for rel in [
            "drives/shared/eng/Runbook.md",
            "drives/shared/eng/.vellum-folder.yaml",
            ".vellum/versions/doc/v1.md",
        ] {
            assert!(vellum_crypto::is_envelope(&disk(rel)), "{rel} is sealed");
        }
        // Private-space ciphertext stays as the device sealed it, and
        // state the server reads from disk is not touched.
        assert_eq!(disk("drives/users/ana/Private/Diary.md"), private);
        assert_eq!(disk(".vellum/tasks/secrets.json"), b"{}");
        // Everything still reads through the store.
        let page = store
            .get(&path("drives/shared/eng/Runbook.md"))
            .await
            .unwrap();
        assert_eq!(&page.bytes[..], b"# Runbook\nRestart the pump.\n");

        // The next start has nothing to do.
        let again = upgrade(&store).await.unwrap();
        assert_eq!((again.seen, again.done), (0, true));
    }

    #[tokio::test]
    async fn a_new_kek_re_wraps_every_file() {
        let dir = tempfile::tempdir().unwrap();
        let _ = old_workspace(dir.path());
        let old = KekId::new("local:old").unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let first = sealed(
            raw,
            AtRestKeys::KeyFile(LocalKeyring::new(
                old.clone(),
                SecretKey::from_bytes([1; 32]),
            )),
        );
        assert!(upgrade(&first).await.unwrap().done);

        let mut ring = LocalKeyring::new(old, SecretKey::from_bytes([1; 32]));
        ring.rotate_to(
            KekId::new("local:new").unwrap(),
            SecretKey::from_bytes([2; 32]),
        );
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let second = sealed(raw, AtRestKeys::KeyFile(ring));
        let report = upgrade(&second).await.unwrap();
        assert_eq!((report.rewritten, report.done), (3, true));
        let runbook = std::fs::read(dir.path().join("drives/shared/eng/Runbook.md")).unwrap();
        assert_eq!(
            vellum_crypto::kek_of(&runbook).unwrap().as_str(),
            "local:new"
        );
    }

    #[tokio::test]
    async fn new_private_space_writes_pass_through_sealed() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let keys = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .unwrap()
            .unwrap();
        let store = sealed(raw, keys);
        let device = vellum_crypto::recipients::RecipientSecret::generate().unwrap();
        let private =
            vellum_crypto::recipients::seal_for(&[device.public()], b"# Diary\n").unwrap();
        let at = path("drives/users/ana/Private/Diary.md");
        let _ = store
            .put(&at, Bytes::from(private.clone()), Precondition::None)
            .await
            .unwrap();
        // The desktop agent reads the folder or bucket raw and opens it.
        let disk = std::fs::read(dir.path().join("drives/users/ana/Private/Diary.md")).unwrap();
        assert_eq!(disk, private);
        assert_eq!(
            vellum_crypto::recipients::open_as(&device, &disk).unwrap(),
            b"# Diary\n"
        );
        assert_eq!(&store.get(&at).await.unwrap().bytes[..], &private[..]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn reindex_reads_sealed_files_from_disk() {
        use vellum_db::FileReader as _;
        let dir = tempfile::tempdir().unwrap();
        let _ = old_workspace(dir.path());
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let keys = keys(&config(dir.path()), &raw, false, lookup(&[]))
            .await
            .unwrap()
            .unwrap();
        assert!(upgrade(&sealed(raw, keys)).await.unwrap().done);

        let files = reader(&config(dir.path()), lookup(&[])).await.unwrap();
        let runbook = dir.path().join("drives/shared/eng/Runbook.md");
        let text = tokio::task::spawn_blocking(move || files.read(&runbook))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, b"# Runbook\nRestart the pump.\n");
    }

    #[tokio::test]
    async fn turning_encryption_off_and_on_again_reseals() {
        let dir = tempfile::tempdir().unwrap();
        let _ = old_workspace(dir.path());
        let cfg = config(dir.path());
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let keys_on = keys(&cfg, &raw, false, lookup(&[])).await.unwrap().unwrap();
        assert!(upgrade(&sealed(raw, keys_on)).await.unwrap().done);

        let raw = LocalStore::open(dir.path()).await.unwrap();
        assert!(
            keys(&cfg, &raw, false, lookup(&[(PLAINTEXT_VAR, "1")]))
                .await
                .unwrap()
                .is_none()
        );
        std::fs::write(dir.path().join("drives/shared/eng/New.md"), "# New\n").unwrap();

        let raw = LocalStore::open(dir.path()).await.unwrap();
        let keys_on = keys(&cfg, &raw, false, lookup(&[])).await.unwrap().unwrap();
        let report = upgrade(&sealed(raw, keys_on)).await.unwrap();
        assert_eq!(report.rewritten, 1);
        let new = std::fs::read(dir.path().join("drives/shared/eng/New.md")).unwrap();
        assert!(vellum_crypto::is_envelope(&new));
    }

    #[tokio::test]
    async fn plaintext_is_an_explicit_choice() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let none = keys(
            &config(dir.path()),
            &raw,
            true,
            lookup(&[(PLAINTEXT_VAR, "1")]),
        )
        .await
        .unwrap();
        assert!(none.is_none());
    }

    #[tokio::test]
    async fn a_bucket_needs_a_shared_kek() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let error = keys(&config(dir.path()), &raw, true, lookup(&[]))
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(error.to_string().contains(KMS_VAR));
        assert!(!dir.path().join(".vellum/keys").join(KEY_FILE).exists());
    }

    #[tokio::test]
    async fn replicas_with_one_passphrase_share_one_kek() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        let vars = [(PASSPHRASE_VAR, "correct horse battery staple")];
        let first = keys(&config(dir.path()), &raw, true, lookup(&vars))
            .await
            .unwrap()
            .unwrap();
        let second = keys(&config(dir.path()), &raw, true, lookup(&vars))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.source(), "passphrase");
        assert_eq!(first.current_kek(), second.current_kek());
        assert!(first.current_kek().as_str().starts_with("argon2id$"));

        let sealed = vellum_crypto::seal(&first, b"shared").await.unwrap();
        assert_eq!(
            vellum_crypto::open(&second, &sealed).await.unwrap(),
            b"shared"
        );

        let wrong = keys(
            &config(dir.path()),
            &raw,
            true,
            lookup(&[(PASSPHRASE_VAR, "wrong")]),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(vellum_crypto::open(&wrong, &sealed).await.is_err());
    }

    #[tokio::test]
    async fn crafted_kek_ids_cannot_demand_huge_derivations() {
        let keys = PassphraseKeys::new(
            Zeroizing::new(b"pass".to_vec()),
            KekId::new("argon2id$m=65536$t=3$p=4$00112233445566778899aabbccddeeff").unwrap(),
        );
        let huge =
            KekId::new("argon2id$m=1048576$t=3$p=4$00112233445566778899aabbccddeeff").unwrap();
        assert!(matches!(
            keys.kek(&huge).await,
            Err(CryptoError::UnknownKek(_))
        ));
    }

    #[tokio::test]
    async fn a_bad_kms_setting_stops_startup() {
        let dir = tempfile::tempdir().unwrap();
        let raw = LocalStore::open(dir.path()).await.unwrap();
        for value in ["argon2id", "aws-kms:eu-west-1", "vault:transit/k"] {
            let error = keys(
                &config(dir.path()),
                &raw,
                false,
                lookup(&[(KMS_VAR, value)]),
            )
            .await
            .err()
            .unwrap();
            assert!(error.to_string().contains(KMS_VAR), "{value}: {error}");
        }
    }
}
