// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The optional screen profile: the page's own HTML printed by Chromium
//! through a Gotenberg sidecar.
//!
//! Print PDF (Typst) is the default and needs nothing outside the binary.
//! Deployments that run Gotenberg can also offer a "looks like the screen"
//! PDF: the render engine's sanitised HTML and its assets are posted to
//! Gotenberg's Chromium route. Run Gotenberg with outbound network denied
//! (`--chromium-deny-list=.*` plus the allow-list for the sidecar itself),
//! so the HTML can only use the files sent with it.

use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use url::Url;

use crate::error::ExportError;
use crate::metrics;
use crate::pdf::Paper;

/// Largest PDF accepted back from Gotenberg.
const MAX_PDF_BYTES: usize = 200 * 1024 * 1024;

/// A page to print: its HTML and the files it references by name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScreenPage {
    /// A full HTML document from the render engine.
    pub html: String,
    /// Stylesheets, fonts and images, referenced by these names.
    pub files: Vec<(String, Vec<u8>)>,
    /// Paper size.
    pub paper: Paper,
    /// Landscape orientation.
    pub landscape: bool,
}

/// A Gotenberg client.
#[derive(Debug, Clone)]
pub struct Gotenberg {
    client: reqwest::Client,
    endpoint: Url,
}

impl Gotenberg {
    /// A client for the Gotenberg at `base` (such as `http://gotenberg:3000`).
    ///
    /// # Errors
    ///
    /// [`ExportError::NotConfigured`] when the URL cannot be used.
    pub fn new(base: &Url, timeout: Duration) -> Result<Self, ExportError> {
        let endpoint = base
            .join("forms/chromium/convert/html")
            .map_err(|_| ExportError::NotConfigured("gotenberg"))?;
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ExportError::NotConfigured("gotenberg"))?;
        Ok(Self { client, endpoint })
    }

    /// Prints `page` to PDF.
    ///
    /// # Errors
    ///
    /// [`ExportError::Remote`] when Gotenberg fails, times out or returns
    /// something that is not a PDF.
    #[tracing::instrument(
        name = "export.pdf_screen",
        skip_all,
        fields(files = page.files.len())
    )]
    pub async fn print(
        &self,
        page: &ScreenPage,
    ) -> Result<Vec<u8>, ExportError> {
        let started = Instant::now();
        let result = self.print_inner(page).await;
        metrics::export_done(
            "pdf_screen",
            started,
            result.as_ref().map(Vec::len),
        );
        result
    }

    async fn print_inner(
        &self,
        page: &ScreenPage,
    ) -> Result<Vec<u8>, ExportError> {
        let (width, height) = match page.paper {
            Paper::A4 => ("8.27", "11.7"),
            Paper::Letter => ("8.5", "11"),
        };
        let mut form = Form::new()
            .part(
                "files",
                Part::bytes(page.html.clone().into_bytes())
                    .file_name("index.html")
                    .mime_str("text/html")
                    .map_err(remote)?,
            )
            .text("paperWidth", width)
            .text("paperHeight", height)
            .text("marginTop", "0.6")
            .text("marginBottom", "0.6")
            .text("marginLeft", "0.6")
            .text("marginRight", "0.6")
            .text("printBackground", "true")
            .text("landscape", if page.landscape { "true" } else { "false" });
        for (name, bytes) in &page.files {
            if name == "index.html" || name.contains('/') || name.contains("..")
            {
                return Err(ExportError::Invalid(format!(
                    "bad screen asset name {name}"
                )));
            }
            form = form.part(
                "files",
                Part::bytes(bytes.clone()).file_name(name.clone()),
            );
        }
        let response = self
            .client
            .post(self.endpoint.clone())
            .multipart(form)
            .send()
            .await
            .map_err(remote)?;
        let status = response.status();
        if !status.is_success() {
            return Err(ExportError::Remote {
                service: "gotenberg",
                message: format!("HTTP {status}"),
                retryable: status.is_server_error() || status.as_u16() == 429,
            });
        }
        let bytes = response.bytes().await.map_err(remote)?;
        if bytes.len() > MAX_PDF_BYTES {
            return Err(ExportError::TooLarge(
                "the screen PDF is over 200 MB".into(),
            ));
        }
        if !bytes.starts_with(b"%PDF-") {
            return Err(ExportError::Remote {
                service: "gotenberg",
                message: "the response is not a PDF".into(),
                retryable: false,
            });
        }
        Ok(bytes.to_vec())
    }
}

fn remote(error: reqwest::Error) -> ExportError {
    let retryable = error.is_timeout() || error.is_connect();
    ExportError::Remote {
        service: "gotenberg",
        message: error.without_url().to_string(),
        retryable,
    }
}
