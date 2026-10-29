// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Google Docs through the Drive API.
//!
//! Export writes the flow as Word with PNG figures and formula images
//! (Google Docs reads neither SVG nor OMML) and uploads it with conversion
//! to a Google Doc. Import downloads a Google Doc as Word and hands it to
//! the Word importer. Calls use the person's own OAuth access token, so
//! Drive applies their permissions; the token is never logged.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use serde::Deserialize;
use url::Url;

use crate::docx::{self, DocxOptions, MathMode, rasterize_svg};
use crate::error::{ExportError, ImportError};
use crate::flow::{Asset, Flow, Item, Media, Node};
use crate::metrics;
use crate::pdf::PdfEngine;

/// The Google Docs MIME type.
pub const GOOGLE_DOC: &str = "application/vnd.google-apps.document";
/// The Word MIME type.
pub const DOCX: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

/// Largest Word file downloaded for import.
const MAX_IMPORT_BYTES: usize = 100 * 1024 * 1024;

/// An OAuth 2.0 access token with the `drive.file` scope (export) or
/// `drive.readonly` (import). Its `Debug` output is redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct AccessToken(String);

impl AccessToken {
    /// Wraps a token.
    pub fn new(token: impl Into<String>) -> Self {
        Self(token.into())
    }
}

impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessToken(…)")
    }
}

/// A file created in Drive.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    /// The Drive file ID.
    pub id: String,
    /// The file name.
    #[serde(default)]
    pub name: String,
    /// Link to open the document.
    #[serde(default)]
    pub web_view_link: String,
}

/// A Drive API client.
#[derive(Debug, Clone)]
pub struct Drive {
    client: reqwest::Client,
    api: Url,
    upload: Url,
}

impl Drive {
    /// A client for Google's Drive API.
    ///
    /// # Errors
    ///
    /// [`ExportError::NotConfigured`] if the HTTP client cannot be built.
    pub fn new(timeout: Duration) -> Result<Self, ExportError> {
        let api = Url::parse("https://www.googleapis.com/drive/v3/")
            .map_err(|_| ExportError::NotConfigured("drive"))?;
        let upload = Url::parse("https://www.googleapis.com/upload/drive/v3/")
            .map_err(|_| ExportError::NotConfigured("drive"))?;
        Self::with_endpoints(api, upload, timeout)
    }

    /// A client for Drive-compatible endpoints (tests and proxies). Both
    /// URLs end with `/`.
    ///
    /// # Errors
    ///
    /// [`ExportError::NotConfigured`] if the HTTP client cannot be built.
    pub fn with_endpoints(api: Url, upload: Url, timeout: Duration) -> Result<Self, ExportError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::limited(3))
            .build()
            .map_err(|_| ExportError::NotConfigured("drive"))?;
        Ok(Self {
            client,
            api,
            upload,
        })
    }

    /// Exports `flow` as a new Google Doc called `name`, in `folder` when
    /// given (a Drive folder ID), else in the person's My Drive.
    ///
    /// # Errors
    ///
    /// [`ExportError::Remote`] when Drive refuses or fails, and the Word
    /// writer's errors.
    #[tracing::instrument(name = "export.gdocs", skip_all, fields(blocks = flow.blocks.len()))]
    pub async fn export(
        &self,
        flow: &Flow,
        engine: &PdfEngine,
        token: &AccessToken,
        name: &str,
        folder: Option<&str>,
    ) -> Result<DriveFile, ExportError> {
        let started = Instant::now();
        let result = self.export_inner(flow, engine, token, name, folder).await;
        metrics::export_done("gdocs", started, result.as_ref().map(|_| 0));
        result
    }

    async fn export_inner(
        &self,
        flow: &Flow,
        engine: &PdfEngine,
        token: &AccessToken,
        name: &str,
        folder: Option<&str>,
    ) -> Result<DriveFile, ExportError> {
        let prepared = for_google_docs(flow, engine);
        let bytes = docx::write(
            &prepared,
            &DocxOptions {
                svg: false,
                math: MathMode::Source,
                ..DocxOptions::default()
            },
        )?;
        let mut metadata = serde_json::json!({ "name": name, "mimeType": GOOGLE_DOC });
        if let Some(folder) = folder {
            check_id(folder).map_err(|m| ExportError::Invalid(m.into()))?;
            metadata["parents"] = serde_json::json!([folder]);
        }
        let boundary = "vellum-export-boundary-7c1e";
        let mut body = Vec::with_capacity(bytes.len() + 512);
        let mut head = String::new();
        let _ = write!(
            head,
            "--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{metadata}\r\n--{boundary}\r\nContent-Type: {DOCX}\r\n\r\n"
        );
        body.extend_from_slice(head.as_bytes());
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let mut url = self
            .upload
            .join("files")
            .map_err(|_| ExportError::NotConfigured("drive"))?;
        let _ = url
            .query_pairs_mut()
            .append_pair("uploadType", "multipart")
            .append_pair("fields", "id,name,webViewLink")
            .append_pair("supportsAllDrives", "true");
        let response = self
            .client
            .post(url)
            .bearer_auth(&token.0)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(export_remote)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ExportError::Remote {
                service: "drive",
                message: drive_message(status, response.text().await.unwrap_or_default()),
                retryable: status.is_server_error() || status.as_u16() == 429,
            });
        }
        response
            .json::<DriveFile>()
            .await
            .map_err(export_remote)
    }

    /// Downloads the Google Doc `file_id` as Word, for the Word importer.
    ///
    /// # Errors
    ///
    /// [`ImportError::Remote`] when Drive refuses or fails, and
    /// [`ImportError::TooLarge`] past 100 MB.
    #[tracing::instrument(name = "import.gdocs.download", skip_all)]
    pub async fn download_docx(
        &self,
        token: &AccessToken,
        file_id: &str,
    ) -> Result<(String, Vec<u8>), ImportError> {
        check_id(file_id).map_err(|m| ImportError::format("gdocs", m))?;
        let meta_url = self
            .api
            .join(&format!("files/{file_id}?fields=name,mimeType&supportsAllDrives=true"))
            .map_err(|e| ImportError::format("gdocs", e.to_string()))?;
        let meta: serde_json::Value = self
            .get(meta_url, token)
            .await?
            .json()
            .await
            .map_err(import_remote)?;
        let name = meta
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Untitled")
            .to_owned();
        let mime = meta
            .get("mimeType")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let url = if mime == GOOGLE_DOC {
            let mut url = self
                .api
                .join(&format!("files/{file_id}/export"))
                .map_err(|e| ImportError::format("gdocs", e.to_string()))?;
            let _ = url.query_pairs_mut().append_pair("mimeType", DOCX);
            url
        } else if mime == DOCX {
            let mut url = self
                .api
                .join(&format!("files/{file_id}"))
                .map_err(|e| ImportError::format("gdocs", e.to_string()))?;
            let _ = url
                .query_pairs_mut()
                .append_pair("alt", "media")
                .append_pair("supportsAllDrives", "true");
            url
        } else {
            return Err(ImportError::format(
                "gdocs",
                format!("the Drive file is {mime}, not a Google Doc or Word file"),
            ));
        };
        let response = self.get(url, token).await?;
        if response
            .content_length()
            .is_some_and(|n| n > MAX_IMPORT_BYTES as u64)
        {
            return Err(ImportError::TooLarge("the document is over 100 MB".into()));
        }
        let bytes = response.bytes().await.map_err(import_remote)?;
        if bytes.len() > MAX_IMPORT_BYTES {
            return Err(ImportError::TooLarge("the document is over 100 MB".into()));
        }
        Ok((name, bytes.to_vec()))
    }

    async fn get(&self, url: Url, token: &AccessToken) -> Result<reqwest::Response, ImportError> {
        let response = self
            .client
            .get(url)
            .bearer_auth(&token.0)
            .send()
            .await
            .map_err(import_remote)?;
        let status = response.status();
        if status.is_success() {
            Ok(response)
        } else {
            Err(ImportError::Remote {
                service: "drive",
                message: drive_message(status, response.text().await.unwrap_or_default()),
                retryable: status.is_server_error() || status.as_u16() == 429,
            })
        }
    }
}

/// Drive IDs are URL-safe base64-like strings.
fn check_id(id: &str) -> Result<(), &'static str> {
    if !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(())
    } else {
        Err("not a Drive file ID")
    }
}

/// Drive's error message, without echoing anything that could hold a token.
fn drive_message(status: reqwest::StatusCode, body: String) -> String {
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| {
            v.pointer("/error/message")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let message: String = message.chars().take(300).collect();
    if message.is_empty() {
        format!("HTTP {status}")
    } else {
        format!("HTTP {status}: {message}")
    }
}

fn export_remote(error: reqwest::Error) -> ExportError {
    let retryable = error.is_timeout() || error.is_connect();
    ExportError::Remote {
        service: "drive",
        message: error.without_url().to_string(),
        retryable,
    }
}

fn import_remote(error: reqwest::Error) -> ImportError {
    let retryable = error.is_timeout() || error.is_connect();
    ImportError::Remote {
        service: "drive",
        message: error.without_url().to_string(),
        retryable,
    }
}

/// The flow as Google Docs should receive it: SVG figures replaced by PNG
/// (Docs drops SVG) and formulas typeset to images (Docs drops OMML).
#[must_use]
pub fn for_google_docs(flow: &Flow, engine: &PdfEngine) -> Flow {
    let mut out = flow.clone();
    let mut extra: Vec<Asset> = Vec::new();
    let next = out.assets.len();
    let mut convert = |node: &mut Node| {
        if let Node::Math {
            tex,
            typst: Some(typst),
        } = node
        {
            let image = engine
                .math_svg(typst)
                .ok()
                .and_then(|svg| rasterize_svg(svg.as_bytes(), 3.0).map(|png| (svg, png)));
            if let Some((svg, png)) = image {
                let (w, h) = svg_size(&svg);
                let name = format!("a{}.png", next + extra.len());
                extra.push(Asset {
                    name: name.clone(),
                    media: Media::Png,
                    bytes: png,
                    width: None,
                    height: None,
                });
                *node = Node::Figure {
                    asset: name,
                    alt: tex.clone(),
                    caption: String::new(),
                    width: w,
                    height: h,
                };
            }
        }
    };
    walk_mut(&mut out.blocks, &mut convert);
    out.assets.extend(extra);
    out
}

/// Width and height in points from an SVG's root `width`/`height`.
fn svg_size(svg: &str) -> (f64, f64) {
    let attr = |name: &str| -> f64 {
        let key = format!("{name}=\"");
        svg.find(&key)
            .and_then(|at| {
                let rest = svg.get(at + key.len()..)?;
                let end = rest.find('"')?;
                rest.get(..end)?
                    .trim_end_matches("pt")
                    .trim_end_matches("px")
                    .parse::<f64>()
                    .ok()
            })
            .unwrap_or(0.0)
    };
    (attr("width"), attr("height"))
}

fn walk_mut(nodes: &mut [Node], f: &mut impl FnMut(&mut Node)) {
    for node in nodes {
        f(node);
        match node {
            Node::Quote { blocks } | Node::Callout { blocks, .. } | Node::Titled { blocks, .. } => {
                walk_mut(blocks, f);
            }
            Node::List { items, .. } => {
                for Item { blocks, .. } in items {
                    walk_mut(blocks, f);
                }
            }
            Node::Columns { columns } => {
                for column in columns {
                    walk_mut(column, f);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_ids_are_checked() {
        assert!(check_id("1AbC-d_9").is_ok());
        assert!(check_id("../etc").is_err());
        assert!(check_id("a?b=c").is_err());
        assert!(check_id("").is_err());
    }

    #[test]
    fn tokens_are_redacted_in_debug_output() {
        let token = AccessToken::new("ya29.secret");
        assert!(!format!("{token:?}").contains("secret"));
    }

    #[test]
    fn drive_errors_keep_only_the_message() {
        let body = r#"{"error":{"code":403,"message":"Insufficient permissions"}}"#;
        assert_eq!(
            drive_message(reqwest::StatusCode::FORBIDDEN, body.into()),
            "HTTP 403 Forbidden: Insufficient permissions"
        );
        assert_eq!(
            drive_message(reqwest::StatusCode::BAD_GATEWAY, "<html>".into()),
            "HTTP 502 Bad Gateway"
        );
    }

    #[test]
    fn svg_sizes_are_read_from_the_root() {
        assert_eq!(
            svg_size(r#"<svg class="typst-doc" width="42.5pt" height="20pt">"#),
            (42.5, 20.0)
        );
    }
}
