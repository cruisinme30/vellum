// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The busy memory budget's workload (R-LT-1): a signed-in user with ten
//! live-editing rooms open and search warm.
//!
//! Against a fresh `vellum serve`, [`warm_up`] sets up the first admin,
//! signs in, mints an API token, creates a shared drive with
//! [`PAGES`] pages, runs [`SEARCHES`] searches so the index readers and
//! caches are loaded, then opens [`ROOMS`] collaboration sockets, one
//! per page, and completes the Yjs sync handshake on each. The sockets
//! stay open in the returned [`Rooms`] while the caller reads the
//! server's resident memory.
//!
//! HTTP and WebSocket are spoken by hand over `TcpStream`, as in the rest
//! of the budget check, so xtask needs no HTTP client.

use std::fmt::Write as _;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use serde_json::{Value, json};

/// Live-editing rooms held open, from the brief's budget.
pub const ROOMS: usize = 10;
/// Pages created, so search has something to read.
pub const PAGES: usize = 40;
/// Searches run before measuring.
pub const SEARCHES: usize = 20;

const TIMEOUT: Duration = Duration::from_secs(10);
const PASSWORD: &str = "budget check password";

/// The open collaboration sockets.
#[derive(Debug)]
pub struct Rooms(Vec<TcpStream>);

impl Rooms {
    /// Closes every socket.
    pub fn close(self) {
        for socket in self.0 {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Puts the server under the busy workload and returns the open rooms.
pub fn warm_up(address: SocketAddr) -> Result<Rooms, String> {
    let token = sign_in(address)?;
    let auth = format!("Bearer {token}");
    let space = request(
        address,
        "POST",
        "/api/v1/drive/spaces",
        &[("Authorization", &auth)],
        Some(&json!({"kind": "shared_drive", "slug": "northstar", "name": "Northstar"})),
    )?
    .expect_json(200, "create the shared drive")?;
    let space_id = string_field(&space, "id")?;
    let mut pages = Vec::with_capacity(PAGES);
    for i in 0..PAGES {
        let body = format!(
            "# Northstar roadmap {i}\n\nThe Q{} launch plan covers onboarding, billing and the \
             mobile release.\n\n- [ ] Review the budget\n- [x] Ship the release notes\n",
            i % 4 + 1
        );
        let page = request(
            address,
            "POST",
            &format!("/api/v1/drive/spaces/{space_id}/pages"),
            &[("Authorization", &auth)],
            Some(&json!({"space_id": space_id, "title": format!("Roadmap {i}"), "body": body})),
        )?
        .expect_json(200, "create a page")?;
        pages.push(string_field(&page, "id")?);
    }
    for i in 0..SEARCHES {
        let words = ["roadmap", "launch", "billing", "mobile", "release"];
        let word = words.get(i % words.len()).copied().unwrap_or("roadmap");
        let _ = request(
            address,
            "GET",
            &format!("/api/v1/search?q={word}"),
            &[("Authorization", &auth)],
            None,
        )?
        .expect_json(200, "search")?;
    }
    let mut rooms = Vec::with_capacity(ROOMS);
    for page in pages.iter().take(ROOMS) {
        rooms.push(open_room(
            address,
            &format!("/collab/{space_id}/{page}"),
            &auth,
        )?);
    }
    Ok(Rooms(rooms))
}

/// Sets up the first admin, signs in and returns an API token.
fn sign_in(address: SocketAddr) -> Result<String, String> {
    let email = "budget@northstar.dev";
    let _ = request(
        address,
        "POST",
        "/api/v1/setup",
        &[],
        Some(&json!({"email": email, "name": "Budget", "password": PASSWORD})),
    )?
    .expect_json(200, "set up the workspace")?;
    let signed_in = request(
        address,
        "POST",
        "/auth/sign-in",
        &[],
        Some(&json!({"email": email, "password": PASSWORD})),
    )?;
    let cookie = signed_in
        .cookie()
        .ok_or_else(|| format!("sign-in set no cookie (HTTP {})", signed_in.status))?;
    let csrf = request(address, "GET", "/api/csrf", &[("Cookie", &cookie)], None)?
        .expect_json(200, "fetch a CSRF token")?;
    let csrf = string_field(&csrf, "token")?;
    let token = request(
        address,
        "POST",
        "/api/v1/me/tokens",
        &[("Cookie", &cookie), ("X-CSRF-Token", &csrf)],
        Some(&json!({"name": "budgets", "scopes": ["read", "write:direct"]})),
    )?
    .expect_json(200, "create an API token")?;
    string_field(&token, "token")
}

fn string_field(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("no \"{field}\" in {value}"))
}

/// One HTTP response.
#[derive(Debug)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn expect_json(self, status: u16, what: &str) -> Result<Value, String> {
        if self.status != status {
            return Err(format!(
                "{what}: HTTP {} {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            ));
        }
        serde_json::from_slice(&self.body).map_err(|e| format!("{what}: {e}"))
    }

    /// The `name=value` pairs of every `Set-Cookie`, as a `Cookie` header.
    fn cookie(&self) -> Option<String> {
        let pairs: Vec<&str> = self
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("set-cookie"))
            .filter_map(|(_, value)| value.split(';').next())
            .collect();
        (!pairs.is_empty()).then(|| pairs.join("; "))
    }
}

/// Sends one HTTP/1.1 request on a fresh connection and reads the reply.
fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> Result<Response, String> {
    let mut stream = connect(address)?;
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\
         Origin: http://{address}\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        head.push_str("Content-Type: application/json\r\n");
    }
    for (name, value) in headers {
        let _ = write!(head, "{name}: {value}\r\n");
    }
    head.push_str("\r\n");
    stream
        .write_all(head.as_bytes())
        .and_then(|()| stream.write_all(body.as_bytes()))
        .map_err(|e| format!("{method} {path}: {e}"))?;
    let mut raw = Vec::new();
    let _ = stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("{method} {path}: {e}"))?;
    parse_response(&raw).ok_or_else(|| format!("{method} {path}: unreadable response"))
}

fn connect(address: SocketAddr) -> Result<TcpStream, String> {
    let stream =
        TcpStream::connect_timeout(&address, TIMEOUT).map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|e| format!("socket: {e}"))?;
    Ok(stream)
}

/// Parses a whole HTTP/1.1 response, decoding a chunked body.
fn parse_response(raw: &[u8]) -> Option<Response> {
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(raw.get(..split)?).ok()?;
    let rest = raw.get(split + 4..)?;
    let mut lines = head.split("\r\n");
    let status = crate::budgets::status_code(lines.next()?)?;
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect();
    let chunked = headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("transfer-encoding") && value.eq_ignore_ascii_case("chunked")
    });
    let body = if chunked {
        dechunk(rest)?
    } else {
        rest.to_vec()
    };
    Some(Response {
        status,
        headers,
        body,
    })
}

fn dechunk(mut rest: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let end = rest.windows(2).position(|w| w == b"\r\n")?;
        let size_text = std::str::from_utf8(rest.get(..end)?).ok()?;
        let size = usize::from_str_radix(size_text.split(';').next()?.trim(), 16).ok()?;
        rest = rest.get(end + 2..)?;
        if size == 0 {
            return Some(body);
        }
        body.extend_from_slice(rest.get(..size)?);
        rest = rest.get(size + 2..)?;
    }
}

/// Opens a collaboration WebSocket, sends Yjs sync step 1 with an empty
/// state vector and waits for the server's first message.
fn open_room(address: SocketAddr, path: &str, auth: &str) -> Result<TcpStream, String> {
    let stream = connect(address)?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    write!(
        writer,
        "GET {path} HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\n\
         Connection: Upgrade\r\nSec-WebSocket-Version: 13\r\n\
         Sec-WebSocket-Key: dmVsbHVtLWJ1ZGdldHMtMQ==\r\nAuthorization: {auth}\r\n\r\n"
    )
    .map_err(|e| format!("{path}: {e}"))?;
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    let _ = reader
        .read_line(&mut status_line)
        .map_err(|e| format!("{path}: {e}"))?;
    if crate::budgets::status_code(&status_line) != Some(101) {
        return Err(format!(
            "{path}: no WebSocket upgrade: {}",
            status_line.trim()
        ));
    }
    loop {
        let mut line = String::new();
        let _ = reader
            .read_line(&mut line)
            .map_err(|e| format!("{path}: {e}"))?;
        if line == "\r\n" || line.is_empty() {
            break;
        }
    }
    // Sync step 1 (message 0, sync 0) with an empty state vector.
    writer
        .write_all(&client_frame(&[0, 0, 1, 0]))
        .map_err(|e| format!("{path}: {e}"))?;
    let mut first = [0_u8; 2];
    reader
        .read_exact(&mut first)
        .map_err(|e| format!("{path}: no reply to sync step 1: {e}"))?;
    Ok(reader.into_inner())
}

/// A masked binary WebSocket frame, as a client must send it. Payloads
/// here are short, so the 7-bit length form is enough.
fn client_frame(payload: &[u8]) -> Vec<u8> {
    const MASK: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
    let length = u8::try_from(payload.len()).unwrap_or(125).min(125);
    let mut frame = vec![0x82, 0x80 | length, MASK[0], MASK[1], MASK[2], MASK[3]];
    frame.extend(
        payload
            .iter()
            .take(usize::from(length))
            .zip(MASK.iter().cycle())
            .map(|(byte, mask)| byte ^ mask),
    );
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_chunked_bodies() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nSet-Cookie: a=1; HttpOnly\r\n\r\n{}";
        let response = parse_response(plain).unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{}");
        assert_eq!(response.cookie().as_deref(), Some("a=1"));

        let chunked =
            b"HTTP/1.1 201 Created\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        let response = parse_response(chunked).unwrap();
        assert_eq!(response.status, 201);
        assert_eq!(response.body, br#"{"a":1}"#);
        assert_eq!(response.cookie(), None);
        assert!(parse_response(b"garbage").is_none());
    }

    #[test]
    fn client_frames_are_masked() {
        let frame = client_frame(&[0, 0, 1, 0]);
        assert_eq!(frame.len(), 10);
        assert_eq!(frame.first(), Some(&0x82));
        assert_eq!(frame.get(1), Some(&0x84));
        let unmasked: Vec<u8> = frame
            .get(6..)
            .unwrap()
            .iter()
            .zip(frame.get(2..6).unwrap().iter().cycle())
            .map(|(b, m)| b ^ m)
            .collect();
        assert_eq!(unmasked, [0, 0, 1, 0]);
    }

    #[test]
    fn unexpected_status_is_an_error() {
        let response = Response {
            status: 403,
            headers: Vec::new(),
            body: b"nope".to_vec(),
        };
        let error = response.expect_json(200, "search").unwrap_err();
        assert!(error.contains("search: HTTP 403 nope"), "{error}");
    }
}
