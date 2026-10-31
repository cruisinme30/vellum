// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Encrypted backups and the restore drill (brief §11, §12).
//!
//! A backup is one `.vbk` file holding every file under the data directory
//! (documents, blobs, the index and the audit log), except the key
//! directory: a backup must never carry the keys that decrypt it. Files are
//! split into chunks of at most [`CHUNK`] bytes and every chunk is sealed
//! with its own data key through `vellum-crypto`, so a backup is safe to
//! copy to any storage. A sealed manifest at the end lists every file with
//! its size and SHA-256.
//!
//! ```text
//! "VBK1"  then frames:  kind (u8) · length (u64 BE) · sealed payload
//!   kind 1  chunk      path length (u16 BE) · path · chunk index (u32 BE) · data
//!   kind 2  manifest   JSON (Manifest), always last
//! ```
//!
//! The restore drill restores the newest backup into a scratch directory,
//! checks every file against the manifest and reports how long it took
//! against the 15-minute RTO; it runs monthly and is audited.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use opentelemetry::KeyValue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, BufReader, BufWriter};
use vellum_crypto::KeyProvider;
use vellum_kernel::{Error, ErrorKind, Result, Timestamp};

/// The file signature.
pub const MAGIC: &[u8; 4] = b"VBK1";
/// The largest chunk sealed at once.
pub const CHUNK: usize = 4 * 1024 * 1024;
/// The file extension.
pub const EXTENSION: &str = "vbk";
/// The recovery time objective the drill is measured against, in seconds.
pub const RTO_SECONDS: u64 = 15 * 60;

const KIND_CHUNK: u8 = 1;
const KIND_MANIFEST: u8 = 2;
/// Sealed frames are never larger than a chunk plus headers and envelope.
const MAX_FRAME: u64 = CHUNK as u64 + 64 * 1024;

/// One file in a backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ManifestEntry {
    /// Path relative to the data directory, with `/` separators.
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 of the content, hex.
    pub sha256: String,
    /// Number of chunks.
    pub chunks: u32,
}

/// What a backup holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Manifest {
    /// Format version (1).
    pub format: u32,
    /// The Vellum version that wrote it.
    pub vellum_version: String,
    /// When the backup started.
    pub created_at: Timestamp,
    /// Every file.
    pub files: Vec<ManifestEntry>,
    /// Total plaintext bytes.
    pub bytes: u64,
}

/// A finished backup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupInfo {
    /// The backup file name.
    pub name: String,
    /// When it started.
    pub created_at: Timestamp,
    /// Files included.
    pub files: u64,
    /// Plaintext bytes.
    pub bytes: u64,
    /// Size of the backup file.
    pub size: u64,
    /// How long it took, in milliseconds.
    pub duration_ms: u64,
    /// Symbolic links skipped (never followed).
    pub skipped_links: u64,
}

/// A finished restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RestoreReport {
    /// Files written and verified.
    pub files: u64,
    /// Bytes written.
    pub bytes: u64,
    /// How long it took, in milliseconds.
    pub duration_ms: u64,
}

/// The outcome of a restore drill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DrillReport {
    /// The backup drilled.
    pub backup: String,
    /// When the drill ran.
    pub ran_at: Timestamp,
    /// Whether every file restored and matched the manifest within the RTO.
    pub passed: bool,
    /// Files verified.
    pub files: u64,
    /// Bytes verified.
    pub bytes: u64,
    /// Restore time in milliseconds.
    pub duration_ms: u64,
    /// Whether the restore finished within [`RTO_SECONDS`].
    pub within_rto: bool,
    /// Why the drill failed, when it did.
    pub error: Option<String>,
}

/// A backup file on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BackupFile {
    /// The file name.
    pub name: String,
    /// Its size in bytes.
    pub size: u64,
}

fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> Error {
    move |e| Error::new(ErrorKind::Unavailable, what).with_source(e)
}

fn corrupt(what: &'static str) -> Error {
    Error::new(ErrorKind::Conflict, what)
}

fn crypto(
    what: &'static str,
) -> impl FnOnce(vellum_crypto::CryptoError) -> Error {
    move |e| Error::new(ErrorKind::Conflict, what).with_source(e)
}

fn millis(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn record_run(op: &'static str, ok: bool, start: Instant) {
    let meter = opentelemetry::global::meter("vellum-admin");
    meter
        .f64_histogram("vellum.backup.duration")
        .with_unit("s")
        .with_description("Backup, restore and drill durations")
        .build()
        .record(
            start.elapsed().as_secs_f64(),
            &[
                KeyValue::new("operation", op),
                KeyValue::new(
                    "outcome",
                    if ok { "success" } else { "failure" },
                ),
            ],
        );
}

/// Turns a relative path into the portable `/`-separated form, refusing
/// anything that could escape the target directory.
fn portable(rel: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(p) => parts.push(p.to_str()?.to_owned()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// The safe on-disk path for a portable path under `root`.
fn safe_join(root: &Path, portable: &str) -> Option<PathBuf> {
    let mut out = root.to_path_buf();
    for part in portable.split('/') {
        let p = Path::new(part);
        let mut comps = p.components();
        match (comps.next(), comps.next()) {
            (Some(Component::Normal(n)), None) if !part.contains('\\') => {
                out.push(n);
            }
            _ => return None,
        }
    }
    (out != root).then_some(out)
}

/// Lists every regular file under `root`, skipping excluded directories
/// and symbolic links. Returns sorted relative paths and the links skipped.
async fn walk(root: &Path, exclude: &[PathBuf]) -> Result<(Vec<PathBuf>, u64)> {
    let mut files = Vec::new();
    let mut links = 0_u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .map_err(io("cannot list the data directory"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(io("cannot list the data directory"))?
        {
            let path = entry.path();
            if exclude.iter().any(|x| path.starts_with(x)) {
                continue;
            }
            let kind = entry
                .file_type()
                .await
                .map_err(io("cannot read a file type"))?;
            if kind.is_symlink() {
                links += 1;
            } else if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file() {
                let is_temp = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.') && n.contains(".tmp-"));
                if !is_temp && let Ok(rel) = path.strip_prefix(root) {
                    files.push(rel.to_path_buf());
                }
            }
        }
    }
    files.sort();
    Ok((files, links))
}

async fn write_frame<K: KeyProvider, W: tokio::io::AsyncWrite + Unpin>(
    keys: &K,
    out: &mut W,
    kind: u8,
    payload: &[u8],
) -> Result<()> {
    let sealed = vellum_crypto::seal(keys, payload)
        .await
        .map_err(crypto("cannot seal a backup frame"))?;
    out.write_all(&[kind])
        .await
        .map_err(io("cannot write the backup"))?;
    out.write_all(&(sealed.len() as u64).to_be_bytes())
        .await
        .map_err(io("cannot write the backup"))?;
    out.write_all(&sealed)
        .await
        .map_err(io("cannot write the backup"))
}

fn chunk_payload(path: &str, index: u32, data: &[u8]) -> Result<Vec<u8>> {
    let len = u16::try_from(path.len()).map_err(|_| {
        Error::new(ErrorKind::InvalidInput, "path too long for a backup")
    })?;
    let mut payload = Vec::with_capacity(2 + path.len() + 4 + data.len());
    payload.extend_from_slice(&len.to_be_bytes());
    payload.extend_from_slice(path.as_bytes());
    payload.extend_from_slice(&index.to_be_bytes());
    payload.extend_from_slice(data);
    Ok(payload)
}

/// Writes a backup of `source` into `dest_dir` and returns what it holds.
///
/// `exclude` lists absolute directories to leave out; the key directory
/// (`source/.vellum/keys`) and `dest_dir` are always excluded.
///
/// # Errors
///
/// Returns an error when a file cannot be read or the backup cannot be
/// written; a partial backup file is removed.
#[tracing::instrument(
    name = "backup.create",
    skip_all,
    fields(source = %source.display())
)]
pub async fn create<K: KeyProvider>(
    keys: &K,
    source: &Path,
    dest_dir: &Path,
    exclude: &[PathBuf],
) -> Result<BackupInfo> {
    let start = Instant::now();
    let result = create_inner(keys, source, dest_dir, exclude, start).await;
    record_run("create", result.is_ok(), start);
    result
}

async fn create_inner<K: KeyProvider>(
    keys: &K,
    source: &Path,
    dest_dir: &Path,
    exclude: &[PathBuf],
    start: Instant,
) -> Result<BackupInfo> {
    tokio::fs::create_dir_all(dest_dir)
        .await
        .map_err(io("cannot create the backup directory"))?;
    let created_at = Timestamp::now();
    let stamp: String = created_at
        .to_string()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    let name = format!("vellum-{stamp}.{EXTENSION}");
    let final_path = dest_dir.join(&name);
    let partial = dest_dir.join(format!(".{name}.partial"));
    let mut skip: Vec<PathBuf> = exclude.to_vec();
    skip.push(source.join(".vellum").join("keys"));
    skip.push(dest_dir.to_path_buf());
    if let Ok(canonical) = tokio::fs::canonicalize(dest_dir).await {
        skip.push(canonical);
    }
    let written = async {
        let (files, skipped_links) = walk(source, &skip).await?;
        let file = tokio::fs::File::create(&partial)
            .await
            .map_err(io("cannot create the backup file"))?;
        let mut out = BufWriter::new(file);
        out.write_all(MAGIC)
            .await
            .map_err(io("cannot write the backup"))?;
        let mut manifest = Manifest {
            format: 1,
            vellum_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_at,
            files: Vec::with_capacity(files.len()),
            bytes: 0,
        };
        let mut buf = vec![0_u8; CHUNK];
        for rel in files {
            let Some(path) = portable(&rel) else { continue };
            let entry = back_up_file(
                keys,
                &mut out,
                &source.join(&rel),
                path,
                &mut buf,
            )
            .await?;
            manifest.bytes += entry.size;
            manifest.files.push(entry);
        }
        let json = serde_json::to_vec(&manifest).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot encode the manifest")
                .with_source(e)
        })?;
        write_frame(keys, &mut out, KIND_MANIFEST, &json).await?;
        out.flush().await.map_err(io("cannot write the backup"))?;
        out.into_inner()
            .sync_all()
            .await
            .map_err(io("cannot sync the backup"))?;
        Ok::<_, Error>((manifest, skipped_links))
    }
    .await;
    let (manifest, skipped_links) = match written {
        Ok(v) => v,
        Err(e) => {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(e);
        }
    };
    tokio::fs::rename(&partial, &final_path)
        .await
        .map_err(io("cannot finish the backup file"))?;
    let size = tokio::fs::metadata(&final_path)
        .await
        .map_err(io("cannot read the backup size"))?
        .len();
    Ok(BackupInfo {
        name,
        created_at,
        files: manifest.files.len() as u64,
        bytes: manifest.bytes,
        size,
        duration_ms: millis(start),
        skipped_links,
    })
}

/// Writes one file as sealed chunks and returns its manifest entry.
async fn back_up_file<K: KeyProvider, W: tokio::io::AsyncWrite + Unpin>(
    keys: &K,
    out: &mut W,
    file: &Path,
    path: String,
    buf: &mut [u8],
) -> Result<ManifestEntry> {
    let mut input = BufReader::new(
        tokio::fs::File::open(file)
            .await
            .map_err(io("cannot open a file to back up"))?,
    );
    let mut hasher = Sha256::new();
    let mut size = 0_u64;
    let mut index = 0_u32;
    loop {
        let n = read_full(&mut input, buf).await?;
        if n == 0 && index > 0 {
            break;
        }
        let data = buf.get(..n).unwrap_or_default();
        hasher.update(data);
        size += n as u64;
        write_frame(keys, out, KIND_CHUNK, &chunk_payload(&path, index, data)?)
            .await?;
        index += 1;
        if n < buf.len() {
            break;
        }
    }
    Ok(ManifestEntry {
        path,
        size,
        sha256: hex::encode(hasher.finalize()),
        chunks: index,
    })
}

async fn read_full<R: tokio::io::AsyncRead + Unpin>(
    input: &mut R,
    buf: &mut [u8],
) -> Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        let Some(rest) = buf.get_mut(filled..) else {
            break;
        };
        let n = input
            .read(rest)
            .await
            .map_err(io("cannot read a file to back up"))?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    Ok(filled)
}

/// A decoded frame.
enum Frame {
    Chunk {
        path: String,
        index: u32,
        data: Vec<u8>,
    },
    Manifest(Manifest),
}

/// Reads frames one at a time from a backup file.
struct Reader<'k, K: KeyProvider> {
    keys: &'k K,
    input: BufReader<tokio::fs::File>,
}

impl<'k, K: KeyProvider> Reader<'k, K> {
    async fn open(keys: &'k K, file: &Path) -> Result<Self> {
        let mut input = BufReader::new(
            tokio::fs::File::open(file)
                .await
                .map_err(io("cannot open the backup"))?,
        );
        let mut magic = [0_u8; 4];
        let _ = input
            .read_exact(&mut magic)
            .await
            .map_err(|_| corrupt("not a Vellum backup"))?;
        if &magic != MAGIC {
            return Err(corrupt("not a Vellum backup"));
        }
        Ok(Self { keys, input })
    }

    async fn next(&mut self) -> Result<Option<Frame>> {
        let mut kind = [0_u8; 1];
        match self.input.read_exact(&mut kind).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Ok(None);
            }
            Err(e) => return Err(io("cannot read the backup")(e)),
        }
        let mut len = [0_u8; 8];
        let _ = self
            .input
            .read_exact(&mut len)
            .await
            .map_err(|_| corrupt("backup is truncated"))?;
        let len = u64::from_be_bytes(len);
        if len > MAX_FRAME {
            return Err(corrupt("backup frame is too large"));
        }
        let mut sealed = vec![0_u8; usize::try_from(len).unwrap_or(0)];
        let _ = self
            .input
            .read_exact(&mut sealed)
            .await
            .map_err(|_| corrupt("backup is truncated"))?;
        let plain =
            vellum_crypto::open(self.keys, &sealed)
                .await
                .map_err(crypto(
                    "cannot decrypt the backup (wrong key or damaged file)",
                ))?;
        match kind {
            [KIND_CHUNK] => decode_chunk(&plain).map(Some),
            [KIND_MANIFEST] => serde_json::from_slice(&plain)
                .map(|m| Some(Frame::Manifest(m)))
                .map_err(|_| corrupt("backup manifest is damaged")),
            _ => Err(corrupt("unknown backup frame")),
        }
    }
}

fn decode_chunk(plain: &[u8]) -> Result<Frame> {
    let bad = || corrupt("backup chunk is damaged");
    let len = plain.get(..2).ok_or_else(bad)?;
    let len = usize::from(u16::from_be_bytes([
        len.first().copied().unwrap_or(0),
        len.get(1).copied().unwrap_or(0),
    ]));
    let path = plain.get(2..2 + len).ok_or_else(bad)?;
    let path = String::from_utf8(path.to_vec()).map_err(|_| bad())?;
    let idx = plain.get(2 + len..6 + len).ok_or_else(bad)?;
    let mut idx_bytes = [0_u8; 4];
    idx_bytes.copy_from_slice(idx);
    let data = plain.get(6 + len..).ok_or_else(bad)?.to_vec();
    Ok(Frame::Chunk {
        path,
        index: u32::from_be_bytes(idx_bytes),
        data,
    })
}

/// Reads only the manifest of a backup.
///
/// # Errors
///
/// Returns a conflict error when the file is damaged or the key is wrong.
pub async fn read_manifest<K: KeyProvider>(
    keys: &K,
    file: &Path,
) -> Result<Manifest> {
    let mut reader = Reader::open(keys, file).await?;
    while let Some(frame) = reader.next().await? {
        if let Frame::Manifest(m) = frame {
            return Ok(m);
        }
    }
    Err(corrupt("backup has no manifest"))
}

struct Pending {
    file: BufWriter<tokio::fs::File>,
    hasher: Sha256,
    size: u64,
    next_index: u32,
}

/// Restores `file` into `target`, which must be empty or not exist yet,
/// and checks every file against the manifest.
///
/// # Errors
///
/// Returns invalid input when `target` is not empty, and a conflict error
/// when the backup is damaged, the key is wrong, or a file does not match
/// the manifest.
#[tracing::instrument(
    name = "backup.restore",
    skip_all,
    fields(file = %file.display())
)]
pub async fn restore<K: KeyProvider>(
    keys: &K,
    file: &Path,
    target: &Path,
) -> Result<RestoreReport> {
    let start = Instant::now();
    let result = restore_inner(keys, file, target, start).await;
    record_run("restore", result.is_ok(), start);
    result
}

async fn restore_inner<K: KeyProvider>(
    keys: &K,
    file: &Path,
    target: &Path,
    start: Instant,
) -> Result<RestoreReport> {
    tokio::fs::create_dir_all(target)
        .await
        .map_err(io("cannot create the restore directory"))?;
    let mut entries = tokio::fs::read_dir(target)
        .await
        .map_err(io("cannot list the restore directory"))?;
    if entries
        .next_entry()
        .await
        .map_err(io("cannot list the restore directory"))?
        .is_some()
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the restore directory must be empty",
        ));
    }
    let mut reader = Reader::open(keys, file).await?;
    let mut open: BTreeMap<String, Pending> = BTreeMap::new();
    let mut done: BTreeMap<String, (u64, String, u32)> = BTreeMap::new();
    let mut manifest = None;
    while let Some(frame) = reader.next().await? {
        match frame {
            Frame::Chunk { path, index, data } => {
                let dest = safe_join(target, &path)
                    .ok_or_else(|| corrupt("backup contains an unsafe path"))?;
                if done.contains_key(&path) {
                    return Err(corrupt("backup repeats a file"));
                }
                if !open.contains_key(&path) {
                    if index != 0 {
                        return Err(corrupt("backup chunks are out of order"));
                    }
                    if let Some(parent) = dest.parent() {
                        tokio::fs::create_dir_all(parent).await.map_err(io(
                            "cannot create a restored directory",
                        ))?;
                    }
                    let f = tokio::fs::File::create(&dest)
                        .await
                        .map_err(io("cannot create a restored file"))?;
                    // Only the file being written is open; finish any other.
                    let finished: Vec<String> = open.keys().cloned().collect();
                    for p in finished {
                        if let Some(pending) = open.remove(&p) {
                            let _ = done.insert(p, finish(pending).await?);
                        }
                    }
                    let _ = open.insert(
                        path.clone(),
                        Pending {
                            file: BufWriter::new(f),
                            hasher: Sha256::new(),
                            size: 0,
                            next_index: 0,
                        },
                    );
                }
                let pending = open
                    .get_mut(&path)
                    .ok_or_else(|| corrupt("backup chunks are out of order"))?;
                if pending.next_index != index {
                    return Err(corrupt("backup chunks are out of order"));
                }
                pending.hasher.update(&data);
                pending.size += data.len() as u64;
                pending.next_index += 1;
                pending
                    .file
                    .write_all(&data)
                    .await
                    .map_err(io("cannot write a restored file"))?;
            }
            Frame::Manifest(m) => {
                manifest = Some(m);
                break;
            }
        }
    }
    for (p, pending) in std::mem::take(&mut open) {
        let _ = done.insert(p, finish(pending).await?);
    }
    let manifest = manifest.ok_or_else(|| corrupt("backup has no manifest"))?;
    if manifest.files.len() != done.len() {
        return Err(corrupt("backup files do not match the manifest"));
    }
    for entry in &manifest.files {
        let Some((size, sha, chunks)) = done.get(&entry.path) else {
            return Err(corrupt("a file in the manifest is missing"));
        };
        if *size != entry.size
            || *sha != entry.sha256
            || *chunks != entry.chunks
        {
            return Err(corrupt("a restored file does not match the manifest"));
        }
    }
    Ok(RestoreReport {
        files: manifest.files.len() as u64,
        bytes: manifest.bytes,
        duration_ms: millis(start),
    })
}

async fn finish(mut pending: Pending) -> Result<(u64, String, u32)> {
    pending
        .file
        .flush()
        .await
        .map_err(io("cannot write a restored file"))?;
    pending
        .file
        .into_inner()
        .sync_all()
        .await
        .map_err(io("cannot sync a restored file"))?;
    Ok((
        pending.size,
        hex::encode(pending.hasher.finalize()),
        pending.next_index,
    ))
}

/// Restores `file` into a fresh directory under `scratch`, verifies it,
/// removes it again and reports the result. A failed drill is a report,
/// not an error, so it can be audited and alerted on.
#[tracing::instrument(
    name = "backup.drill",
    skip_all,
    fields(file = %file.display())
)]
pub async fn drill<K: KeyProvider>(
    keys: &K,
    file: &Path,
    scratch: &Path,
) -> DrillReport {
    let start = Instant::now();
    let ran_at = Timestamp::now();
    let target =
        scratch.join(format!("drill-{}", crate::settings::new_install_id()));
    let result = restore(keys, file, &target).await;
    let _ = tokio::fs::remove_dir_all(&target).await;
    let duration_ms = millis(start);
    let within_rto = duration_ms <= RTO_SECONDS * 1000;
    let backup = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned();
    let report = match result {
        Ok(r) => DrillReport {
            backup,
            ran_at,
            passed: within_rto,
            files: r.files,
            bytes: r.bytes,
            duration_ms,
            within_rto,
            error: (!within_rto)
                .then(|| "restore exceeded the 15-minute RTO".to_owned()),
        },
        Err(e) => DrillReport {
            backup,
            ran_at,
            passed: false,
            files: 0,
            bytes: 0,
            duration_ms,
            within_rto,
            error: Some(e.message().to_owned()),
        },
    };
    record_run("drill", report.passed, start);
    report
}

/// Backups in `dir`, newest first.
///
/// # Errors
///
/// Returns an unavailable error when the directory cannot be read.
pub async fn list(dir: &Path) -> Result<Vec<BackupFile>> {
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(io("cannot list backups")(e)),
    };
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(io("cannot list backups"))?
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("vellum-")
            && name.ends_with(&format!(".{EXTENSION}"))
        {
            let size = entry
                .metadata()
                .await
                .map_err(io("cannot read a backup"))?
                .len();
            out.push(BackupFile { name, size });
        }
    }
    // Names embed a sortable UTC timestamp.
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}

/// Deletes all but the newest `keep` backups and returns the names removed.
///
/// # Errors
///
/// Returns an unavailable error when a backup cannot be removed.
pub async fn prune(dir: &Path, keep: usize) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for old in list(dir).await?.into_iter().skip(keep.max(1)) {
        tokio::fs::remove_file(dir.join(&old.name))
            .await
            .map_err(io("cannot remove an old backup"))?;
        removed.push(old.name);
    }
    Ok(removed)
}

/// Whether a drill is due: never run, or older than `interval_days`.
#[must_use]
pub fn drill_due(
    last_ms: Option<i64>,
    now_ms: i64,
    interval_days: u32,
) -> bool {
    last_ms.is_none_or(|last| {
        now_ms - last >= i64::from(interval_days) * 86_400_000
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_portable_and_safe() {
        assert_eq!(portable(Path::new("a/b.md")).as_deref(), Some("a/b.md"));
        assert_eq!(portable(Path::new("../x")), None);
        assert_eq!(portable(Path::new("/abs")), None);
        assert_eq!(portable(Path::new("")), None);
        let root = Path::new("/r");
        assert_eq!(safe_join(root, "a/b"), Some(PathBuf::from("/r/a/b")));
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "",
            "a//b",
            "a\\..\\b",
            ".",
        ] {
            assert_eq!(safe_join(root, bad), None, "{bad}");
        }
    }

    #[test]
    fn drill_schedule() {
        assert!(drill_due(None, 0, 30));
        assert!(!drill_due(Some(0), 29 * 86_400_000, 30));
        assert!(drill_due(Some(0), 30 * 86_400_000, 30));
    }

    #[test]
    fn chunk_round_trip_and_damage() {
        let payload = chunk_payload("a/b.md", 3, b"hello").unwrap();
        match decode_chunk(&payload).unwrap() {
            Frame::Chunk { path, index, data } => {
                assert_eq!(path, "a/b.md");
                assert_eq!(index, 3);
                assert_eq!(data, b"hello");
            }
            Frame::Manifest(_) => unreachable!(),
        }
        assert!(decode_chunk(&[0, 9, b'a']).is_err());
        assert!(chunk_payload(&"x".repeat(70_000), 0, b"").is_err());
    }
}
