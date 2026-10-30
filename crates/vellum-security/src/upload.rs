// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Upload hygiene: size limits, content sniffing, image re-encoding and a
//! virus-scanner hook.
//!
//! An upload is accepted only when its bytes, not its name or the
//! client's `Content-Type`, identify an allowed kind, and the declared
//! extension agrees with them. Raster images are decoded under pixel and
//! memory limits and re-encoded, which drops metadata (EXIF location,
//! comments) and any payload hidden after the image data. SVG is accepted
//! but only ever served as a download; the renderer inlines SVG only after
//! its own sanitiser. ZIP containers (Office files included) are inflated
//! under size, ratio and depth limits before anything scans them. A
//! [`Scanner`] (`ClamAV` via [`Clamd`]) runs last; [`check`] runs the whole
//! pipeline and is the one entry point an upload route should use.

use std::io::{Cursor, Read};
use std::sync::Arc;
use std::time::Duration;

use image::{ImageFormat, ImageReader, Limits};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Semaphore;

/// A kind of file Vellum accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Kind {
    /// PNG image.
    Png,
    /// JPEG image.
    Jpeg,
    /// GIF image.
    Gif,
    /// `WebP` image.
    Webp,
    /// SVG drawing; served only as a download.
    Svg,
    /// PDF document.
    Pdf,
    /// Office Open XML (`.docx`, `.xlsx`, `.pptx`) or other ZIP container.
    Zip,
    /// UTF-8 text (`.md`, `.txt`, `.csv`, `.json`, `.yaml`).
    Text,
}

impl Kind {
    /// The media type to store and serve.
    #[must_use]
    pub const fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Svg => "image/svg+xml",
            Self::Pdf => "application/pdf",
            Self::Zip => "application/zip",
            Self::Text => "text/plain; charset=utf-8",
        }
    }

    /// Whether the file may be shown inline; everything else is served
    /// with `Content-Disposition: attachment`.
    #[must_use]
    pub const fn inline(self) -> bool {
        matches!(
            self,
            Self::Png | Self::Jpeg | Self::Gif | Self::Webp | Self::Pdf
        )
    }

    const fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg"],
            Self::Gif => &["gif"],
            Self::Webp => &["webp"],
            Self::Svg => &["svg"],
            Self::Pdf => &["pdf"],
            Self::Zip => &["zip", "docx", "xlsx", "pptx", "odt", "ods"],
            Self::Text => &["md", "txt", "csv", "tsv", "json", "yaml", "yml"],
        }
    }

    const fn raster(self) -> Option<ImageFormat> {
        match self {
            Self::Png => Some(ImageFormat::Png),
            Self::Jpeg => Some(ImageFormat::Jpeg),
            Self::Gif => Some(ImageFormat::Gif),
            Self::Webp => Some(ImageFormat::WebP),
            _ => None,
        }
    }
}

/// Identifies a file from its first bytes.
#[must_use]
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    let starts = |magic: &[u8]| bytes.starts_with(magic);
    if starts(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else if starts(&[0xff, 0xd8, 0xff]) {
        Some(Kind::Jpeg)
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some(Kind::Gif)
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some(Kind::Webp)
    } else if starts(b"%PDF-") {
        Some(Kind::Pdf)
    } else if starts(b"PK\x03\x04") {
        Some(Kind::Zip)
    } else {
        let text = std::str::from_utf8(bytes).ok()?;
        if text.contains('\0') {
            return None;
        }
        let head = text.trim_start_matches('\u{feff}').trim_start();
        let svg = (head.starts_with("<?xml") || head.starts_with("<svg"))
            && head.get(..1024).unwrap_or(head).contains("<svg");
        Some(if svg { Kind::Svg } else { Kind::Text })
    }
}

/// Why an upload was refused.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum UploadError {
    /// The file is larger than the limit.
    #[error("file is larger than {0} bytes")]
    TooLarge(u64),
    /// The content is not a kind Vellum accepts.
    #[error("file type is not allowed")]
    Unknown,
    /// The extension does not match the content.
    #[error("file extension does not match its content ({0:?})")]
    Mismatch(Kind),
    /// The image could not be decoded within the limits.
    #[error("image is invalid or too large: {0}")]
    Image(String),
    /// The scanner found malware.
    #[error("file was rejected by the virus scanner: {0}")]
    Infected(String),
    /// The scanner could not be reached; uploads fail closed.
    #[error("virus scanner unavailable: {0}")]
    ScannerUnavailable(String),
    /// An archive is encrypted, unreadable, nested too deep or expands
    /// past the limits: a zip bomb, or a file the scanner could not see
    /// into (red team round 4).
    #[error("archive refused: {0}")]
    Archive(String),
}

/// Limits applied to every upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UploadPolicy {
    /// Largest accepted file.
    pub max_bytes: u64,
    /// Largest image width or height in pixels.
    pub max_dimension: u32,
    /// Largest decoded image in bytes (guards against decompression bombs).
    pub max_decoded_bytes: u64,
    /// Largest total an archive may inflate to, nested archives included.
    pub max_expanded_bytes: u64,
}

impl Default for UploadPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 100 * 1024 * 1024,
            max_dimension: 16_384,
            max_decoded_bytes: 512 * 1024 * 1024,
            max_expanded_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Most entries in one archive, nested archives included.
pub const MAX_ARCHIVE_ENTRIES: usize = 20_000;
/// Most an archive entry may expand over its packed size. Office XML and
/// text expand 5 to 20 times; a zip bomb about 1000 times.
pub const MAX_ARCHIVE_RATIO: u64 = 200;
/// Entries up to this size may expand any amount.
const RATIO_FREE_BYTES: u64 = 8 * 1024 * 1024;
/// Archives inside archives are followed this deep; deeper is refused
/// (`ClamAV` gives up past its `MaxRecursion` and answers "OK").
pub const MAX_ARCHIVE_DEPTH: u32 = 4;

/// Inflates every entry of a ZIP archive without keeping it, so that what
/// the scanner is asked to read is bounded and fully readable.
struct ArchiveWalk {
    max_total: u64,
    total: u64,
    entries: usize,
}

impl ArchiveWalk {
    fn walk(&mut self, bytes: &[u8], depth: u32) -> Result<(), UploadError> {
        let refuse = |why: String| UploadError::Archive(why);
        if depth > MAX_ARCHIVE_DEPTH {
            return Err(refuse(format!(
                "archives are nested more than {MAX_ARCHIVE_DEPTH} deep"
            )));
        }
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| refuse(format!("not a readable zip ({e})")))?;
        for index in 0..zip.len() {
            self.entries += 1;
            if self.entries > MAX_ARCHIVE_ENTRIES {
                return Err(refuse(format!(
                    "more than {MAX_ARCHIVE_ENTRIES} files"
                )));
            }
            // Encrypted entries and unknown compression fail here, so the
            // scanner never gets an archive it cannot see into.
            let mut file = zip.by_index(index).map_err(|e| {
                refuse(format!("entry {index} cannot be read ({e})"))
            })?;
            if file.is_dir() {
                continue;
            }
            let expands = file
                .compressed_size()
                .saturating_mul(MAX_ARCHIVE_RATIO)
                .max(RATIO_FREE_BYTES);
            let room = self.max_total.saturating_sub(self.total);
            let cap = expands.min(room);
            let mut head = [0_u8; 4];
            let mut limited = file.by_ref().take(cap.saturating_add(1));
            let got = read_up_to(&mut limited, &mut head).map_err(|e| {
                refuse(format!("entry {index} is corrupt ({e})"))
            })?;
            let nested = got == 4 && head == *b"PK\x03\x04";
            let corrupt = |e: std::io::Error| {
                refuse(format!("entry {index} is corrupt ({e})"))
            };
            let (size, inner) = if nested {
                let mut inner = head.to_vec();
                let _ = limited.read_to_end(&mut inner).map_err(corrupt)?;
                (inner.len() as u64, Some(inner))
            } else {
                let rest = std::io::copy(&mut limited, &mut std::io::sink())
                    .map_err(corrupt)?;
                (rest + got as u64, None)
            };
            if size > expands {
                return Err(refuse(format!(
                    "an entry expands more than {MAX_ARCHIVE_RATIO} times its \
                        packed size"
                )));
            }
            self.total += size;
            if self.total > self.max_total {
                return Err(refuse(format!(
                    "the archive expands past {} MiB",
                    self.max_total / 1024 / 1024
                )));
            }
            if let Some(inner) = inner {
                self.walk(&inner, depth + 1)?;
            }
        }
        Ok(())
    }
}

/// Reads until `buf` is full or the reader ends.
fn read_up_to(
    reader: &mut impl Read,
    buf: &mut [u8],
) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(buf.get_mut(filled..).unwrap_or_default())? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}

/// A checked upload, ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cleaned {
    /// What the file is.
    pub kind: Kind,
    /// A safe file name: no directories, control characters or leading dots.
    pub file_name: String,
    /// The bytes to store: re-encoded for raster images, else unchanged.
    pub bytes: Vec<u8>,
}

impl UploadPolicy {
    /// Checks size, sniffs the type, matches it to the extension and
    /// re-encodes raster images.
    ///
    /// # Errors
    ///
    /// Returns the first check the upload fails.
    pub fn clean(
        &self,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Cleaned, UploadError> {
        if bytes.len() as u64 > self.max_bytes {
            return Err(UploadError::TooLarge(self.max_bytes));
        }
        let kind = sniff(&bytes).ok_or(UploadError::Unknown)?;
        let file_name = safe_file_name(file_name);
        let ext = file_name
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if !kind.extensions().contains(&ext.as_str()) {
            return Err(UploadError::Mismatch(kind));
        }
        if kind == Kind::Zip {
            ArchiveWalk {
                max_total: self.max_expanded_bytes,
                total: 0,
                entries: 0,
            }
            .walk(&bytes, 1)?;
        }
        let bytes = match kind.raster() {
            Some(format) => self.reencode(&bytes, format)?,
            None => bytes,
        };
        Ok(Cleaned {
            kind,
            file_name,
            bytes,
        })
    }

    fn reencode(
        &self,
        bytes: &[u8],
        format: ImageFormat,
    ) -> Result<Vec<u8>, UploadError> {
        let mut limits = Limits::default();
        limits.max_image_width = Some(self.max_dimension);
        limits.max_image_height = Some(self.max_dimension);
        limits.max_alloc = Some(self.max_decoded_bytes);
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits);
        let image = reader
            .decode()
            .map_err(|e| UploadError::Image(e.to_string()))?;
        let mut out = Cursor::new(Vec::new());
        image
            .write_to(&mut out, format)
            .map_err(|e| UploadError::Image(e.to_string()))?;
        Ok(out.into_inner())
    }
}

/// Invisible formatting characters that can disguise a name: bidi
/// overrides and isolates (`invoice\u{202e}fdp.txt` shows as `invoicetxt.pdf`,
/// #273), zero-width marks and the byte order mark.
const fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
            | '\u{061c}'
    )
}

/// Keeps the last path segment, drops control, formatting and reserved
/// characters and leading dots, and caps the length at 200 bytes.
#[must_use]
pub fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base
        .chars()
        .filter(|c| {
            !c.is_control()
                && !is_format_char(*c)
                && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
        })
        .collect();
    let mut cleaned = cleaned.trim().trim_start_matches('.').to_owned();
    while cleaned.len() > 200 {
        let _ = cleaned.pop();
    }
    if cleaned.is_empty() {
        "upload".to_owned()
    } else {
        cleaned
    }
}

/// A virus scanner.
pub trait Scanner: Send + Sync {
    /// Scans `bytes`.
    ///
    /// # Errors
    ///
    /// [`UploadError::Infected`] for malware,
    /// [`UploadError::ScannerUnavailable`] when the scan could not run.
    fn scan(
        &self,
        bytes: &[u8],
    ) -> impl Future<Output = Result<(), UploadError>> + Send;
}

/// Scans one replica sends to clamd at once. clamd runs 10 scanner threads
/// by default (`MaxThreads`) and queues the rest, so more in flight only
/// holds more uploads in memory while they wait.
pub const CLAMD_IN_FLIGHT: usize = 8;

/// `ClamAV`'s `clamd` over TCP, using the `INSTREAM` command.
///
/// At most [`CLAMD_IN_FLIGHT`] scans run at once. A scan that cannot start
/// and finish within the timeout fails closed as
/// [`UploadError::ScannerUnavailable`], so a flood of uploads is refused
/// quickly instead of queueing without bound.
#[derive(Debug, Clone)]
pub struct Clamd {
    address: String,
    timeout: Duration,
    slots: Arc<Semaphore>,
}

impl Clamd {
    /// A scanner at `address` (for example `clamav:3310`).
    #[must_use]
    pub fn new(address: String, timeout: Duration) -> Self {
        Self::with_in_flight(address, timeout, CLAMD_IN_FLIGHT)
    }

    /// Like [`Clamd::new`], with at most `in_flight` scans at once.
    #[must_use]
    pub fn with_in_flight(
        address: String,
        timeout: Duration,
        in_flight: usize,
    ) -> Self {
        Self {
            address,
            timeout,
            slots: Arc::new(Semaphore::new(in_flight.max(1))),
        }
    }

    async fn instream(&self, bytes: &[u8]) -> std::io::Result<String> {
        let mut stream = TcpStream::connect(&self.address).await?;
        let (mut reader, mut writer) = stream.split();
        // clamd answers and hangs up as soon as a stream passes its
        // StreamMaxLength. Read while writing: once clamd closes with our
        // bytes unread, its kernel may reset the connection, and on macOS
        // that reset discards a reply we have not read yet.
        let (sent, (reply, read)) = tokio::join!(
            Self::send(&mut writer, bytes),
            Self::reply(&mut reader)
        );
        let reply = String::from_utf8_lossy(&reply)
            .trim_end_matches(['\0', '\n'])
            .to_owned();
        match (sent, read) {
            _ if !reply.is_empty() => Ok(reply),
            (Err(e), _) | (Ok(()), Err(e)) => Err(e),
            (Ok(()), Ok(())) => Ok(reply),
        }
    }

    /// Reads clamd's NUL-terminated reply, keeping what arrived before any
    /// error.
    async fn reply<R: AsyncRead + Unpin>(
        reader: &mut R,
    ) -> (Vec<u8>, std::io::Result<()>) {
        let mut reply = Vec::new();
        let mut buf = [0_u8; 512];
        while reply.len() < 4096 && !reply.contains(&0) {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => reply.extend(buf.iter().take(n)),
                Err(e) => return (reply, Err(e)),
            }
        }
        (reply, Ok(()))
    }

    async fn send<W: AsyncWrite + Unpin>(
        stream: &mut W,
        bytes: &[u8],
    ) -> std::io::Result<()> {
        stream.write_all(b"zINSTREAM\0").await?;
        for chunk in bytes.chunks(64 * 1024) {
            let len = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
            stream.write_all(&len.to_be_bytes()).await?;
            stream.write_all(chunk).await?;
        }
        stream.write_all(&0_u32.to_be_bytes()).await
    }
}

impl Scanner for Clamd {
    async fn scan(&self, bytes: &[u8]) -> Result<(), UploadError> {
        let span = tracing::info_span!("upload.scan", bytes = bytes.len());
        // The wait for a slot counts against the same budget as the scan.
        let bounded = async {
            let _slot =
                self.slots.acquire().await.map_err(std::io::Error::other)?;
            self.instream(bytes).await
        };
        let reply = tracing::Instrument::instrument(
            tokio::time::timeout(self.timeout, bounded),
            span,
        )
        .await
        .map_err(|_| {
            UploadError::ScannerUnavailable("timed out or busy".to_owned())
        })?
        .map_err(|e| UploadError::ScannerUnavailable(e.to_string()))?;
        match reply.strip_suffix(" FOUND") {
            Some(found) => Err(UploadError::Infected(
                found.trim_start_matches("stream: ").to_owned(),
            )),
            None if reply.ends_with("OK") => Ok(()),
            None if reply.contains("size limit exceeded") => {
                Err(UploadError::ScannerUnavailable(format!(
                    "clamd refused a {} byte upload as too large; set \
                     StreamMaxLength in clamd.conf to at least the upload \
                     limit ({reply})",
                    bytes.len()
                )))
            }
            None => Err(UploadError::ScannerUnavailable(reply)),
        }
    }
}

/// Checks an upload end to end, then scans it.
///
/// Limits, type, archive and image checks run on a blocking thread, then
/// the scanner when there is one. Upload routes call this rather than
/// [`UploadPolicy::clean`], so no route can skip the scan or decode a
/// large image on an async worker.
///
/// # Errors
///
/// The first check the upload fails, as [`UploadPolicy::clean`] and
/// [`Scanner::scan`] report it.
pub async fn check<S: Scanner>(
    policy: UploadPolicy,
    scanner: Option<&S>,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<Cleaned, UploadError> {
    // Refuse oversized bodies before moving them to another thread.
    if bytes.len() as u64 > policy.max_bytes {
        return Err(UploadError::TooLarge(policy.max_bytes));
    }
    let cleaned =
        tokio::task::spawn_blocking(move || policy.clean(&file_name, bytes))
            .await
            .map_err(|e| {
                UploadError::Image(format!("the upload check stopped: {e}"))
            })??;
    if let Some(scanner) = scanner {
        // What is stored is what is scanned: the bytes as sent, or the
        // re-encoded image that replaces them.
        scanner.scan(&cleaned.bytes).await?;
    }
    Ok(cleaned)
}
