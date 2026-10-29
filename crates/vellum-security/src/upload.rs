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
//! its own sanitiser. A [`Scanner`] (`ClamAV` via [`Clamd`]) runs last.

use std::io::Cursor;
use std::time::Duration;

use image::{ImageFormat, ImageReader, Limits};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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
}

impl Default for UploadPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 100 * 1024 * 1024,
            max_dimension: 16_384,
            max_decoded_bytes: 512 * 1024 * 1024,
        }
    }
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
    pub fn clean(&self, file_name: &str, bytes: Vec<u8>) -> Result<Cleaned, UploadError> {
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

    fn reencode(&self, bytes: &[u8], format: ImageFormat) -> Result<Vec<u8>, UploadError> {
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

/// Keeps the last path segment, drops control, formatting and reserved characters and
/// leading dots, and caps the length at 200 bytes.
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
    /// [`UploadError::Infected`] for malware, [`UploadError::ScannerUnavailable`]
    /// when the scan could not run.
    fn scan(&self, bytes: &[u8]) -> impl Future<Output = Result<(), UploadError>> + Send;
}

/// `ClamAV`'s `clamd` over TCP, using the `INSTREAM` command.
#[derive(Debug, Clone)]
pub struct Clamd {
    address: String,
    timeout: Duration,
}

impl Clamd {
    /// A scanner at `address` (for example `clamav:3310`).
    #[must_use]
    pub const fn new(address: String, timeout: Duration) -> Self {
        Self { address, timeout }
    }

    async fn instream(&self, bytes: &[u8]) -> std::io::Result<String> {
        let mut stream = TcpStream::connect(&self.address).await?;
        stream.write_all(b"zINSTREAM\0").await?;
        for chunk in bytes.chunks(64 * 1024) {
            let len = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
            stream.write_all(&len.to_be_bytes()).await?;
            stream.write_all(chunk).await?;
        }
        stream.write_all(&0_u32.to_be_bytes()).await?;
        let mut reply = String::new();
        let _read = stream.take(4096).read_to_string(&mut reply).await?;
        Ok(reply.trim_end_matches(['\0', '\n']).to_owned())
    }
}

impl Scanner for Clamd {
    async fn scan(&self, bytes: &[u8]) -> Result<(), UploadError> {
        let span = tracing::info_span!("upload.scan", bytes = bytes.len());
        let reply = tracing::Instrument::instrument(
            tokio::time::timeout(self.timeout, self.instream(bytes)),
            span,
        )
        .await
        .map_err(|_| UploadError::ScannerUnavailable("timed out".to_owned()))?
        .map_err(|e| UploadError::ScannerUnavailable(e.to_string()))?;
        match reply.strip_suffix(" FOUND") {
            Some(found) => Err(UploadError::Infected(
                found.trim_start_matches("stream: ").to_owned(),
            )),
            None if reply.ends_with("OK") => Ok(()),
            None => Err(UploadError::ScannerUnavailable(reply)),
        }
    }
}
