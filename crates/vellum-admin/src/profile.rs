// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum profile`: a flame graph of where a running server spends time.
//!
//! The command asks a server for the folded span stacks of its recent
//! traces (`GET /api/v1/admin/profile`, admin scope) and renders them as a
//! self-contained SVG, or reads a folded-stacks file saved earlier.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use vellum_kernel::{Error, ErrorKind, Result};
use vellum_observe::flame;

/// The profile endpoint, relative to the server URL.
pub const PROFILE_PATH: &str = "/api/v1/admin/profile";

/// Arguments of `vellum profile`.
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct ProfileArgs {
    /// The server, such as `https://vellum.northstar.dev`.
    #[arg(long, env = "VELLUM_URL", default_value = "http://127.0.0.1:8080")]
    pub server: String,
    /// An API token with the `admin` scope.
    #[arg(long, env = "VELLUM_TOKEN", hide_env_values = true)]
    pub token: Option<String>,
    /// How far back to look, in seconds (the ring buffer holds the last
    /// 10,000 requests).
    #[arg(long, default_value_t = 60)]
    pub seconds: u64,
    /// Only requests to this route template.
    #[arg(long)]
    pub route: Option<String>,
    /// Render a folded-stacks file instead of asking a server.
    #[arg(long)]
    pub input: Option<PathBuf>,
    /// Where to write the result.
    #[arg(long, short, default_value = "vellum-profile.svg")]
    pub out: PathBuf,
    /// Write folded stacks (for other flame graph tools) instead of SVG.
    #[arg(long)]
    pub folded: bool,
}

/// The profile endpoint's response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProfileResponse {
    /// Folded stacks, one `frame;frame;frame microseconds` line each.
    pub folded: String,
    /// Traces included.
    pub traces: usize,
}

impl ProfileResponse {
    /// The folded stacks, parsed.
    #[must_use]
    pub fn folded_stacks(&self) -> flame::Folded {
        flame::from_text(&self.folded)
    }
}

fn fetch(args: &ProfileArgs) -> Result<String> {
    let seconds = args.seconds.to_string();
    let mut query = vec![("seconds", seconds.as_str())];
    if let Some(route) = &args.route {
        query.push(("route", route));
    }
    let body: ProfileResponse =
        crate::http::get_json(&args.server, args.token.as_deref(), PROFILE_PATH, &query)?;
    Ok(body.folded)
}

/// Runs the command and returns the file written.
///
/// # Errors
///
/// Returns an error when the input cannot be read, the server cannot be
/// reached or refuses, or the output cannot be written.
pub fn run(args: &ProfileArgs) -> Result<PathBuf> {
    let text = match &args.input {
        Some(path) => std::fs::read_to_string(path).map_err(|e| {
            Error::new(ErrorKind::InvalidInput, "cannot read the input file").with_source(e)
        })?,
        None => fetch(args)?,
    };
    let folded = flame::from_text(&text);
    let output = if args.folded {
        flame::to_text(&folded)
    } else {
        let title = args.route.as_deref().map_or_else(
            || format!("Vellum, last {} s", args.seconds),
            |r| format!("Vellum {r}, last {} s", args.seconds),
        );
        flame::svg(&folded, &title)
    };
    std::fs::write(&args.out, output).map_err(|e| {
        Error::new(ErrorKind::Unavailable, "cannot write the output file").with_source(e)
    })?;
    Ok(args.out.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read as _, Write as _};

    fn args(dir: &std::path::Path) -> ProfileArgs {
        ProfileArgs {
            server: "http://127.0.0.1:9".into(),
            token: Some("t".into()),
            seconds: 30,
            route: Some("/api/v1/x".into()),
            input: None,
            out: dir.join("out.svg"),
            folded: false,
        }
    }

    #[test]
    fn renders_a_folded_file() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("in.folded");
        std::fs::write(&input, "http.request;db.query 900\nhttp.request 100\n").unwrap();
        let mut a = args(dir.path());
        a.input = Some(input);
        let out = run(&a).unwrap();
        let svg = std::fs::read_to_string(out).unwrap();
        assert!(svg.contains("Vellum /api/v1/x, last 30 s"));
        assert!(svg.contains("db.query"));
        a.folded = true;
        a.route = None;
        a.out = dir.path().join("out.folded");
        let _ = run(&a).unwrap();
        assert_eq!(
            std::fs::read_to_string(&a.out).unwrap(),
            "http.request 100\nhttp.request;db.query 900\n"
        );
        a.input = Some(dir.path().join("missing"));
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::InvalidInput);
    }

    /// A one-shot HTTP server answering `status` with `body`.
    fn serve(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (format!("http://{addr}"), handle)
    }

    #[test]
    fn asks_the_server_with_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let (url, handle) = serve(200, r#"{"folded":"a;b 5\n","traces":1}"#);
        let mut a = args(dir.path());
        a.server = format!("{url}/");
        let _ = run(&a).unwrap();
        let request = handle.join().unwrap();
        assert!(request.starts_with("GET /api/v1/admin/profile?seconds=30&route=%2Fapi%2Fv1%2Fx "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer t")
        );
        assert!(std::fs::read_to_string(&a.out).unwrap().contains("<svg"));
    }

    #[test]
    fn maps_refusals_and_failures() {
        let dir = tempfile::tempdir().unwrap();
        for (status, kind) in [
            (401, ErrorKind::Unauthenticated),
            (403, ErrorKind::PermissionDenied),
            (500, ErrorKind::Unavailable),
        ] {
            let (url, handle) = serve(status, "{}");
            let mut a = args(dir.path());
            a.server = url;
            a.token = None;
            assert_eq!(run(&a).unwrap_err().kind(), kind);
            let _ = handle.join();
        }
        let (url, handle) = serve(200, "not json");
        let mut a = args(dir.path());
        a.server = url;
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::Unavailable);
        let _ = handle.join();
        a.server = "not a url".into();
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::InvalidInput);
        a.server = "http://127.0.0.1:9".into();
        assert_eq!(run(&a).unwrap_err().kind(), ErrorKind::Unavailable);
    }
}
