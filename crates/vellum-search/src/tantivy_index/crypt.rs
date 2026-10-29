// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A tantivy [`Directory`] that encrypts every file at rest (D63).
//!
//! The index holds document text, so it is encrypted like the files it is
//! built from. Each file is a 20-byte header (`VSX1` plus a random 16-byte
//! salt) followed by 4 KiB plaintext chunks, each sealed with
//! XChaCha20-Poly1305. The nonce is the salt plus the chunk number; the AAD
//! binds the file name, the chunk number and whether it is the last chunk,
//! so chunks cannot be swapped, reordered or truncated undetected. Reads
//! decrypt only the chunks a range touches, so random access stays cheap.

use std::fmt;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use tantivy::HasLen;
use tantivy::directory::error::{DeleteError, LockError, OpenReadError, OpenWriteError};
use tantivy::directory::{
    AntiCallToken, Directory, DirectoryLock, FileHandle, Lock, OwnedBytes, TerminatingWrite,
    WatchCallback, WatchHandle, WritePtr,
};
use zeroize::Zeroizing;

const MAGIC: &[u8; 4] = b"VSX1";
const SALT_LEN: usize = 16;
const HEADER_LEN: usize = MAGIC.len() + SALT_LEN;
const CHUNK: usize = 4096;
const TAG: usize = 16;
const SEALED_CHUNK: usize = CHUNK + TAG;

/// The 256-bit key that encrypts the search index.
///
/// It comes from the same key hierarchy as document keys (a DEK held for
/// the index, wrapped by the KEK); this crate only receives the raw key.
#[derive(Clone)]
pub struct IndexKey(Zeroizing<[u8; 32]>);

impl IndexKey {
    /// Wraps raw key bytes.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(Zeroizing::new(bytes))
    }
}

impl fmt::Debug for IndexKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("IndexKey(…)")
    }
}

#[derive(Clone)]
struct Cipher(Arc<XChaCha20Poly1305>);

impl Cipher {
    fn new(key: &IndexKey) -> Self {
        Self(Arc::new(XChaCha20Poly1305::new((&*key.0).into())))
    }

    fn seal(
        &self,
        path: &Path,
        salt: &[u8; SALT_LEN],
        index: u64,
        last: bool,
        plain: &[u8],
    ) -> io::Result<Vec<u8>> {
        let aad = aad(path, index, last);
        self.0
            .encrypt(
                &nonce(salt, index),
                Payload {
                    msg: plain,
                    aad: &aad,
                },
            )
            .map_err(|_| io::Error::other("index encryption failed"))
    }

    fn open(
        &self,
        path: &Path,
        salt: &[u8; SALT_LEN],
        index: u64,
        last: bool,
        sealed: &[u8],
    ) -> io::Result<Vec<u8>> {
        let aad = aad(path, index, last);
        self.0
            .decrypt(
                &nonce(salt, index),
                Payload {
                    msg: sealed,
                    aad: &aad,
                },
            )
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "index chunk failed authentication",
                )
            })
    }
}

fn nonce(salt: &[u8; SALT_LEN], index: u64) -> XNonce {
    let mut bytes = [0_u8; 24];
    let (head, tail) = bytes.split_at_mut(SALT_LEN);
    head.copy_from_slice(salt);
    tail.copy_from_slice(&index.to_le_bytes());
    XNonce::from(bytes)
}

fn aad(path: &Path, index: u64, last: bool) -> Vec<u8> {
    let name = path.to_string_lossy();
    let mut aad = Vec::with_capacity(name.len() + 9);
    aad.extend_from_slice(name.as_bytes());
    aad.extend_from_slice(&index.to_le_bytes());
    aad.push(u8::from(last));
    aad
}

fn fresh_salt() -> io::Result<[u8; SALT_LEN]> {
    let mut salt = [0_u8; SALT_LEN];
    getrandom::fill(&mut salt).map_err(|_| io::Error::other("no randomness for the index salt"))?;
    Ok(salt)
}

/// Encrypts a whole buffer into the file format.
fn seal_all(cipher: &Cipher, path: &Path, data: &[u8]) -> io::Result<Vec<u8>> {
    let salt = fresh_salt()?;
    let mut out = Vec::with_capacity(HEADER_LEN + data.len() + (data.len() / CHUNK + 1) * TAG);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&salt);
    let mut chunks: Vec<&[u8]> = data.chunks(CHUNK).collect();
    if chunks.is_empty() {
        chunks.push(&[]);
    }
    let count = chunks.len();
    for (index, chunk) in chunks.into_iter().enumerate() {
        let sealed = cipher.seal(path, &salt, index as u64, index + 1 == count, chunk)?;
        out.extend_from_slice(&sealed);
    }
    Ok(out)
}

/// The layout of an encrypted file, from its length.
#[derive(Debug, Clone, Copy)]
struct Layout {
    chunks: usize,
    plain_len: usize,
}

fn layout(file_len: usize) -> io::Result<Layout> {
    let corrupt = || io::Error::new(io::ErrorKind::InvalidData, "truncated encrypted index file");
    let body = file_len.checked_sub(HEADER_LEN).ok_or_else(corrupt)?;
    if body < TAG {
        return Err(corrupt());
    }
    let chunks = body.div_ceil(SEALED_CHUNK);
    let last = body - (chunks - 1) * SEALED_CHUNK;
    if last < TAG {
        return Err(corrupt());
    }
    Ok(Layout {
        chunks,
        plain_len: (chunks - 1) * CHUNK + (last - TAG),
    })
}

fn read_header(bytes: &[u8]) -> io::Result<[u8; SALT_LEN]> {
    let header = bytes
        .get(..HEADER_LEN)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing index header"))?;
    let (magic, salt) = header.split_at(MAGIC.len());
    if magic != MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "index file is not encrypted with this format (wrong key mode?)",
        ));
    }
    let mut out = [0_u8; SALT_LEN];
    out.copy_from_slice(salt);
    Ok(out)
}

/// Decrypts a whole buffer in the file format.
fn open_all(cipher: &Cipher, path: &Path, data: &[u8]) -> io::Result<Vec<u8>> {
    let salt = read_header(data)?;
    let layout = layout(data.len())?;
    let mut out = Vec::with_capacity(layout.plain_len);
    let body = data.get(HEADER_LEN..).unwrap_or_default();
    for (index, sealed) in body.chunks(SEALED_CHUNK).enumerate() {
        out.extend(cipher.open(
            path,
            &salt,
            index as u64,
            index + 1 == layout.chunks,
            sealed,
        )?);
    }
    Ok(out)
}

/// Wraps another directory and encrypts everything written through it.
#[derive(Clone)]
pub struct EncryptedDirectory {
    inner: Box<dyn Directory>,
    cipher: Cipher,
}

impl EncryptedDirectory {
    /// Encrypts `inner` with `key`.
    #[must_use]
    pub fn new(inner: Box<dyn Directory>, key: &IndexKey) -> Self {
        Self {
            inner,
            cipher: Cipher::new(key),
        }
    }
}

impl fmt::Debug for EncryptedDirectory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedDirectory")
            .field("inner", &self.inner)
            .finish_non_exhaustive()
    }
}

impl Directory for EncryptedDirectory {
    fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
        let inner = self.inner.get_file_handle(path)?;
        let wrap = |error: io::Error| OpenReadError::wrap_io_error(error, path.to_path_buf());
        let header = inner
            .read_bytes(0..HEADER_LEN.min(inner.len()))
            .map_err(wrap)?;
        let salt = read_header(header.as_slice()).map_err(wrap)?;
        let layout = layout(inner.len()).map_err(wrap)?;
        Ok(Arc::new(EncryptedFile {
            inner,
            cipher: self.cipher.clone(),
            path: path.to_path_buf(),
            salt,
            layout,
        }))
    }

    fn delete(&self, path: &Path) -> Result<(), DeleteError> {
        self.inner.delete(path)
    }

    fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
        self.inner.exists(path)
    }

    fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
        let wrap = |error: io::Error| OpenWriteError::wrap_io_error(error, path.to_path_buf());
        let salt = fresh_salt().map_err(wrap)?;
        let mut inner = self.inner.open_write(path)?;
        inner.write_all(MAGIC).map_err(wrap)?;
        inner.write_all(&salt).map_err(wrap)?;
        let writer = EncryptingWriter {
            inner: Some(inner),
            cipher: self.cipher.clone(),
            path: path.to_path_buf(),
            salt,
            buffer: Vec::with_capacity(CHUNK),
            index: 0,
        };
        Ok(io::BufWriter::new(Box::new(writer)))
    }

    fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
        let sealed = self.inner.atomic_read(path)?;
        open_all(&self.cipher, path, &sealed)
            .map_err(|error| OpenReadError::wrap_io_error(error, path.to_path_buf()))
    }

    fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
        let sealed = seal_all(&self.cipher, path, data)?;
        self.inner.atomic_write(path, &sealed)
    }

    fn sync_directory(&self) -> io::Result<()> {
        self.inner.sync_directory()
    }

    fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
        // Lock files hold no data; the inner directory's own locking applies.
        self.inner.acquire_lock(lock)
    }

    fn watch(&self, watch_callback: WatchCallback) -> tantivy::Result<WatchHandle> {
        self.inner.watch(watch_callback)
    }
}

/// A read handle that decrypts the chunks a range touches.
#[derive(Debug)]
struct EncryptedFile {
    inner: Arc<dyn FileHandle>,
    cipher: Cipher,
    path: PathBuf,
    salt: [u8; SALT_LEN],
    layout: Layout,
}

impl fmt::Debug for Cipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Cipher(XChaCha20-Poly1305)")
    }
}

impl HasLen for EncryptedFile {
    fn len(&self) -> usize {
        self.layout.plain_len
    }
}

impl FileHandle for EncryptedFile {
    fn read_bytes(&self, range: Range<usize>) -> io::Result<OwnedBytes> {
        if range.start > range.end || range.end > self.layout.plain_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past the end of an encrypted index file",
            ));
        }
        if range.is_empty() {
            return Ok(OwnedBytes::empty());
        }
        let first = range.start / CHUNK;
        let last = (range.end - 1) / CHUNK;
        let mut plain = Vec::with_capacity((last - first + 1) * CHUNK);
        for index in first..=last {
            let from = HEADER_LEN + index * SEALED_CHUNK;
            let to = (from + SEALED_CHUNK).min(self.inner.len());
            let sealed = self.inner.read_bytes(from..to)?;
            let is_last = index + 1 == self.layout.chunks;
            plain.extend(self.cipher.open(
                &self.path,
                &self.salt,
                index as u64,
                is_last,
                sealed.as_slice(),
            )?);
        }
        let offset = range.start - first * CHUNK;
        let wanted = plain
            .get(offset..offset + range.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "short encrypted chunk"))?
            .to_vec();
        Ok(OwnedBytes::new(wanted))
    }
}

/// Buffers plaintext and writes sealed chunks to the inner writer.
struct EncryptingWriter {
    inner: Option<WritePtr>,
    cipher: Cipher,
    path: PathBuf,
    salt: [u8; SALT_LEN],
    buffer: Vec<u8>,
    index: u64,
}

impl EncryptingWriter {
    fn emit(&mut self, last: bool) -> io::Result<()> {
        let sealed = self
            .cipher
            .seal(&self.path, &self.salt, self.index, last, &self.buffer)?;
        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| io::Error::other("write after terminate"))?;
        inner.write_all(&sealed)?;
        self.buffer.clear();
        self.index += 1;
        Ok(())
    }
}

impl Write for EncryptingWriter {
    fn write(&mut self, mut data: &[u8]) -> io::Result<usize> {
        let written = data.len();
        while !data.is_empty() {
            // A full chunk is sealed only once more data arrives, so the
            // final chunk is always known when it is sealed.
            if self.buffer.len() == CHUNK {
                self.emit(false)?;
            }
            let room = CHUNK - self.buffer.len();
            let (now, later) = data.split_at(room.min(data.len()));
            self.buffer.extend_from_slice(now);
            data = later;
        }
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        // Partial chunks stay buffered until terminate; flushing them early
        // would need a re-seal. Tantivy only relies on terminate.
        match self.inner.as_mut() {
            Some(inner) => inner.flush(),
            None => Ok(()),
        }
    }
}

impl TerminatingWrite for EncryptingWriter {
    fn terminate_ref(&mut self, _: AntiCallToken) -> io::Result<()> {
        self.emit(true)?;
        match self.inner.take() {
            Some(inner) => inner.terminate(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tantivy::directory::RamDirectory;

    use super::*;

    fn dir() -> (RamDirectory, EncryptedDirectory) {
        let ram = RamDirectory::create();
        let enc = EncryptedDirectory::new(Box::new(ram.clone()), &IndexKey::from_bytes([7; 32]));
        (ram, enc)
    }

    fn write(dir: &EncryptedDirectory, path: &str, data: &[u8]) {
        let mut w = dir.open_write(Path::new(path)).unwrap();
        // Write in odd-sized pieces to cross chunk boundaries.
        for piece in data.chunks(1000) {
            w.write_all(piece).unwrap();
        }
        w.terminate().unwrap();
    }

    #[test]
    fn streamed_files_round_trip_at_every_size_and_range() {
        let (_, enc) = dir();
        for size in [0, 1, CHUNK - 1, CHUNK, CHUNK + 1, 3 * CHUNK, 3 * CHUNK + 17] {
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let name = format!("f{size}");
            write(&enc, &name, &data);
            let handle = enc.get_file_handle(Path::new(&name)).unwrap();
            assert_eq!(handle.len(), size);
            assert_eq!(handle.read_bytes(0..size).unwrap().as_slice(), &data[..]);
            if size > 10 {
                let mid = size / 2;
                let r = (mid - 5)..(mid + 5).min(size);
                assert_eq!(handle.read_bytes(r.clone()).unwrap().as_slice(), &data[r]);
            }
            assert!(handle.read_bytes(0..size + 1).is_err());
        }
    }

    #[test]
    fn plaintext_never_reaches_the_inner_directory() {
        let (ram, enc) = dir();
        let secret = b"northstar launch codename".repeat(300);
        write(&enc, "seg", &secret);
        enc.atomic_write(Path::new("meta.json"), b"{\"northstar\":1}")
            .unwrap();
        let raw = ram.atomic_read(Path::new("seg")).unwrap();
        assert!(!raw.windows(9).any(|w| w == b"northstar"));
        let meta = ram.atomic_read(Path::new("meta.json")).unwrap();
        assert!(!meta.windows(9).any(|w| w == b"northstar"));
        assert_eq!(
            enc.atomic_read(Path::new("meta.json")).unwrap(),
            b"{\"northstar\":1}"
        );
    }

    #[test]
    fn tampering_renaming_and_wrong_keys_fail() {
        let (ram, enc) = dir();
        write(&enc, "a", &[1; 3 * CHUNK]);
        let mut raw = ram.atomic_read(Path::new("a")).unwrap();
        // A file copied under another name does not authenticate.
        ram.atomic_write(Path::new("b"), &raw).unwrap();
        assert!(
            enc.get_file_handle(Path::new("b"))
                .unwrap()
                .read_bytes(0..1)
                .is_err()
        );
        // Truncating the last chunk is detected.
        raw.truncate(raw.len() - SEALED_CHUNK);
        ram.atomic_write(Path::new("a"), &raw).unwrap();
        let handle = enc.get_file_handle(Path::new("a")).unwrap();
        assert!(handle.read_bytes(0..handle.len()).is_err());
        // Another key cannot read.
        let other = EncryptedDirectory::new(Box::new(ram.clone()), &IndexKey::from_bytes([8; 32]));
        enc.atomic_write(Path::new("m"), b"x").unwrap();
        assert!(other.atomic_read(Path::new("m")).is_err());
        // A plaintext file is refused.
        ram.atomic_write(Path::new("p"), b"plain text file here")
            .unwrap();
        assert!(enc.atomic_read(Path::new("p")).is_err());
    }

    #[test]
    fn the_key_is_not_printed() {
        assert_eq!(
            format!("{:?}", IndexKey::from_bytes([1; 32])),
            "IndexKey(…)"
        );
    }
}
