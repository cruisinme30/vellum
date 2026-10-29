// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! CalDAV for `VTODO` (RFC 4791, RFC 6578; D20, R-MW-6): Apple Reminders,
//! Thunderbird and DAVx5 see each task list as a calendar collection.
//!
//! This module is the protocol, not the transport: it reads request paths,
//! headers and XML bodies ([`parse_path`], [`parse_propfind`],
//! [`parse_report`]) and writes the multistatus
//! XML responses. The service fetches the data in between and applies
//! `PUT`/`DELETE` as checklist edits, so the Markdown block stays the
//! source of truth.
//!
//! ```text
//! /dav/principals/<user>/                    principal
//! /dav/calendars/<user>/                     calendar home
//! /dav/calendars/<user>/<list>/              one task list (a calendar)
//! /dav/calendars/<user>/<list>/<uid>.ics     one task (a VTODO)
//! ```
//!
//! Sync tokens are the list's version. A client presenting the current
//! token gets an empty change set; any other token is refused with
//! `DAV:valid-sync-token`, and the client falls back to a full listing,
//! which RFC 6578 allows. This keeps the index free of tombstones (D83).

use std::fmt::Write as _;

use quick_xml::XmlVersion;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;
use sha2::{Digest as _, Sha256};

use crate::telemetry;

/// The `DAV:` namespace.
pub const NS_DAV: &str = "DAV:";
/// The CalDAV namespace.
pub const NS_CALDAV: &str = "urn:ietf:params:xml:ns:caldav";
/// The Calendar Server namespace (`getctag`).
pub const NS_CS: &str = "http://calendarserver.org/ns/";
/// The Apple iCal namespace (`calendar-color`).
pub const NS_APPLE: &str = "http://apple.com/ns/ical/";

/// The URL prefix CalDAV is mounted at.
pub const PREFIX: &str = "/dav";

/// A resource addressed by a CalDAV path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    /// `/dav/` itself (service discovery).
    Root,
    /// A user's principal.
    Principal {
        /// The user's handle.
        user: String,
    },
    /// A user's calendar home.
    Home {
        /// The user's handle.
        user: String,
    },
    /// A task list.
    List {
        /// The user's handle.
        user: String,
        /// The list ID.
        list: String,
    },
    /// One task.
    Object {
        /// The user's handle.
        user: String,
        /// The list ID.
        list: String,
        /// The task UID (the resource name without `.ics`).
        uid: String,
    },
}

/// Why a request was refused, with its HTTP status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DavError {
    /// 400: the body could not be read.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// 403 with `DAV:valid-sync-token`: the client must resync.
    #[error("the sync token is no longer valid")]
    InvalidSyncToken,
    /// 404.
    #[error("not found")]
    NotFound,
    /// 405.
    #[error("method not allowed on this resource")]
    MethodNotAllowed,
    /// 412: `If-Match` or `If-None-Match` failed.
    #[error("precondition failed")]
    PreconditionFailed,
    /// 415 with `CALDAV:supported-calendar-data`: not a `VTODO` we accept.
    #[error("unsupported calendar data: {0}")]
    UnsupportedData(String),
}

impl DavError {
    /// The HTTP status code.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::InvalidSyncToken => 403,
            Self::NotFound => 404,
            Self::MethodNotAllowed => 405,
            Self::PreconditionFailed => 412,
            Self::UnsupportedData(_) => 415,
        }
    }

    /// The `DAV:error` body for errors that carry a precondition.
    #[must_use]
    pub fn body(&self) -> Option<String> {
        let condition = match self {
            Self::InvalidSyncToken => "<D:valid-sync-token/>",
            Self::UnsupportedData(_) => "<C:supported-calendar-data/>",
            _ => return None,
        };
        Some(format!(
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
             <D:error xmlns:D=\"DAV:\" xmlns:C=\"{NS_CALDAV}\">{condition}</D:error>\n"
        ))
    }
}

fn decode_segment(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = segment.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Percent-encodes a path segment.
#[must_use]
pub fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for b in segment.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'@') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Reads a request path into the resource it names.
#[must_use]
pub fn parse_path(path: &str) -> Option<Resource> {
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let rest = path.strip_prefix(PREFIX)?;
    let segments: Vec<String> = rest
        .split('/')
        .filter(|s| !s.is_empty())
        .map(decode_segment)
        .collect::<Option<_>>()?;
    let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
    let owned = |s: &&str| (*s).to_owned();
    Some(match parts.as_slice() {
        [] => Resource::Root,
        ["principals", user] => Resource::Principal { user: owned(user) },
        ["calendars", user] => Resource::Home { user: owned(user) },
        ["calendars", user, list] => Resource::List {
            user: owned(user),
            list: owned(list),
        },
        ["calendars", user, list, object] => Resource::Object {
            user: owned(user),
            list: owned(list),
            uid: object.strip_suffix(".ics")?.to_owned(),
        },
        _ => return None,
    })
}

/// The path of a resource, always ending in `/` for collections.
#[must_use]
pub fn href(resource: &Resource) -> String {
    match resource {
        Resource::Root => format!("{PREFIX}/"),
        Resource::Principal { user } => format!("{PREFIX}/principals/{}/", encode_segment(user)),
        Resource::Home { user } => format!("{PREFIX}/calendars/{}/", encode_segment(user)),
        Resource::List { user, list } => format!(
            "{PREFIX}/calendars/{}/{}/",
            encode_segment(user),
            encode_segment(list)
        ),
        Resource::Object { user, list, uid } => format!(
            "{PREFIX}/calendars/{}/{}/{}.ics",
            encode_segment(user),
            encode_segment(list),
            encode_segment(uid)
        ),
    }
}

/// The `Depth` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// The resource only.
    Zero,
    /// The resource and its members.
    One,
}

impl Depth {
    /// Reads the header; a missing or `infinity` depth is treated as 1,
    /// the deepest this tree goes below a collection.
    #[must_use]
    pub fn from_header(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("0") => Self::Zero,
            _ => Self::One,
        }
    }
}

/// A property name with its namespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PropName {
    /// The namespace URI.
    pub ns: String,
    /// The local name.
    pub name: String,
}

impl PropName {
    fn new(ns: &str, name: &str) -> Self {
        Self {
            ns: ns.to_owned(),
            name: name.to_owned(),
        }
    }
}

/// What a `PROPFIND` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropRequest {
    /// `allprop`, or an empty body.
    All,
    /// `propname`.
    Names,
    /// These properties.
    Props(Vec<PropName>),
}

/// A `REPORT` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    /// `calendar-query`; `open_only` when the filter excludes completed
    /// tasks.
    Query {
        /// Properties to return.
        props: PropRequest,
        /// Whether only open tasks match.
        open_only: bool,
    },
    /// `calendar-multiget`.
    Multiget {
        /// Properties to return.
        props: PropRequest,
        /// The requested object paths.
        hrefs: Vec<String>,
    },
    /// `sync-collection`.
    Sync {
        /// Properties to return.
        props: PropRequest,
        /// The client's token; empty for an initial sync.
        token: Option<String>,
    },
}

#[derive(Debug, Default)]
struct XmlScan {
    root: Option<PropName>,
    props: Vec<PropName>,
    allprop: bool,
    propname: bool,
    hrefs: Vec<String>,
    sync_token: Option<String>,
    open_only: bool,
}

fn scan_xml(body: &str) -> Result<XmlScan, DavError> {
    let mut reader = NsReader::from_str(body);
    let mut scan = XmlScan::default();
    let mut path: Vec<PropName> = Vec::new();
    let mut text = String::new();
    let mut prop_filter: Option<String> = None;
    let bad = |e: &dyn std::fmt::Display| DavError::BadRequest(e.to_string());
    loop {
        let (ns, event) = reader.read_resolved_event().map_err(|e| bad(&e))?;
        let ns = match ns {
            ResolveResult::Bound(ns) => AsRef::<str>::as_ref(&ns).to_owned(),
            _ => String::new(),
        };
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let local = AsRef::<str>::as_ref(&e.local_name()).to_owned();
                let name = PropName::new(&ns, &local);
                if scan.root.is_none() {
                    scan.root = Some(name.clone());
                }
                let parent = path.last().map(|p| (p.ns.as_str(), p.name.as_str()));
                match (ns.as_str(), local.as_str(), parent) {
                    (_, _, Some((NS_DAV, "prop"))) => scan.props.push(name.clone()),
                    (NS_DAV, "allprop", _) => scan.allprop = true,
                    (NS_DAV, "propname", _) => scan.propname = true,
                    (NS_CALDAV, "prop-filter", _) => {
                        prop_filter = e
                            .try_get_attribute("name")
                            .ok()
                            .flatten()
                            .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                            .map(|v| v.to_ascii_uppercase());
                    }
                    (NS_CALDAV, "is-not-defined", _) => {
                        if prop_filter.as_deref() == Some("COMPLETED") {
                            scan.open_only = true;
                        }
                    }
                    (NS_CALDAV, "text-match", _) => {
                        let negated = e
                            .try_get_attribute("negate-condition")
                            .ok()
                            .flatten()
                            .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                            .is_some_and(|v| v == "yes");
                        if negated && prop_filter.as_deref() == Some("STATUS") {
                            scan.open_only = true;
                        }
                    }
                    _ => {}
                }
                if matches!(event, Event::Start(_)) {
                    path.push(name);
                    text.clear();
                }
            }
            Event::Text(e) => text.push_str(&e.xml10_content()),
            Event::CData(e) => text.push_str(&e.xml10_content()),
            Event::GeneralRef(r) => {
                let name: &str = r.as_ref();
                text.push_str(match name {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                });
                if let Ok(Some(c)) = r.resolve_char_ref() {
                    text.push(c);
                }
            }
            Event::End(_) => {
                if let Some(closed) = path.pop() {
                    let value = text.trim().to_owned();
                    match (closed.ns.as_str(), closed.name.as_str()) {
                        (NS_DAV, "href") => scan.hrefs.push(value),
                        (NS_DAV, "sync-token") if !value.is_empty() => {
                            scan.sync_token = Some(value);
                        }
                        (NS_CALDAV, "prop-filter") => prop_filter = None,
                        _ => {}
                    }
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(scan)
}

fn props_of(scan: &XmlScan) -> PropRequest {
    if scan.propname {
        PropRequest::Names
    } else if scan.allprop || scan.props.is_empty() {
        PropRequest::All
    } else {
        PropRequest::Props(scan.props.clone())
    }
}

/// Reads a `PROPFIND` body; an empty body means `allprop`.
///
/// # Errors
///
/// Returns [`DavError::BadRequest`] for malformed XML.
pub fn parse_propfind(body: &str) -> Result<PropRequest, DavError> {
    if body.trim().is_empty() {
        return Ok(PropRequest::All);
    }
    Ok(props_of(&scan_xml(body)?))
}

/// Reads a `REPORT` body.
///
/// # Errors
///
/// Returns [`DavError::BadRequest`] for malformed XML or an unsupported
/// report.
pub fn parse_report(body: &str) -> Result<Report, DavError> {
    let scan = scan_xml(body)?;
    let props = props_of(&scan);
    let root = scan
        .root
        .clone()
        .ok_or_else(|| DavError::BadRequest("empty report".into()))?;
    match (root.ns.as_str(), root.name.as_str()) {
        (NS_CALDAV, "calendar-query") => Ok(Report::Query {
            props,
            open_only: scan.open_only,
        }),
        (NS_CALDAV, "calendar-multiget") => Ok(Report::Multiget {
            props,
            hrefs: scan.hrefs,
        }),
        (NS_DAV, "sync-collection") => Ok(Report::Sync {
            props,
            token: scan.sync_token,
        }),
        (ns, name) => Err(DavError::BadRequest(format!(
            "unsupported report {{{ns}}}{name}"
        ))),
    }
}

/// Checks `If-Match` and `If-None-Match` against the current ETag
/// (`None` when the resource does not exist).
///
/// # Errors
///
/// Returns [`DavError::PreconditionFailed`] when a header does not hold.
pub fn check_preconditions(
    current: Option<&str>,
    if_match: Option<&str>,
    if_none_match: Option<&str>,
) -> Result<(), DavError> {
    let matches = |header: &str| {
        header.trim() == "*"
            || current.is_some_and(|etag| header.split(',').any(|h| h.trim() == etag))
    };
    if let Some(header) = if_match
        && (current.is_none() || !matches(header))
    {
        return Err(DavError::PreconditionFailed);
    }
    if let Some(header) = if_none_match
        && current.is_some()
        && matches(header)
    {
        return Err(DavError::PreconditionFailed);
    }
    Ok(())
}

/// A quoted strong ETag for a calendar object's text.
#[must_use]
pub fn etag(ical: &str) -> String {
    let digest = Sha256::digest(ical.as_bytes());
    let mut out = String::from("\"");
    for byte in digest.iter().take(16) {
        let _ = write!(out, "{byte:02x}");
    }
    out.push('"');
    out
}

/// A task list as a calendar collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListInfo {
    /// The list ID, used in the path.
    pub id: String,
    /// The display name.
    pub title: String,
    /// The list's version; changes on every edit.
    pub version: u64,
    /// A colour as `#RRGGBB`, if set.
    pub color: Option<String>,
}

/// One task as a calendar object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The UID.
    pub uid: String,
    /// The full `VCALENDAR` text.
    pub ical: String,
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// The sync token for a list version.
#[must_use]
pub fn sync_token(list: &ListInfo) -> String {
    format!(
        "https://vellum.invalid/sync/{}/{}",
        encode_segment(&list.id),
        list.version
    )
}

/// Validates a client's sync token against the list.
///
/// # Errors
///
/// Returns [`DavError::InvalidSyncToken`] for any token but the current.
pub fn check_sync_token(list: &ListInfo, token: Option<&str>) -> Result<bool, DavError> {
    match token {
        None => Ok(false),
        Some(token) if token == sync_token(list) => Ok(true),
        Some(_) => Err(DavError::InvalidSyncToken),
    }
}

/// Writes multistatus responses.
#[derive(Debug)]
pub struct Multistatus {
    body: String,
}

/// A property value to write.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    /// Literal XML (already escaped).
    Xml(String),
    /// Text, escaped on write.
    Text(String),
}

fn prefix_of(ns: &str) -> Option<&'static str> {
    match ns {
        NS_DAV => Some("D"),
        NS_CALDAV => Some("C"),
        NS_CS => Some("CS"),
        NS_APPLE => Some("A"),
        _ => None,
    }
}

impl Default for Multistatus {
    fn default() -> Self {
        Self::new()
    }
}

impl Multistatus {
    /// Starts a response.
    #[must_use]
    pub fn new() -> Self {
        Self {
            body: format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<D:multistatus xmlns:D=\"DAV:\" \
                 xmlns:C=\"{NS_CALDAV}\" xmlns:CS=\"{NS_CS}\" xmlns:A=\"{NS_APPLE}\">\n"
            ),
        }
    }

    fn response(&mut self, href: &str, request: &PropRequest, known: &[(PropName, Value)]) {
        let _ = write!(
            self.body,
            "<D:response><D:href>{}</D:href>",
            xml_escape(href)
        );
        let mut found = String::new();
        let mut missing = String::new();
        let write_name = |out: &mut String, name: &PropName| match prefix_of(&name.ns) {
            Some(prefix) => {
                let _ = write!(out, "<{prefix}:{}/>", name.name);
            }
            None => {
                let _ = write!(
                    out,
                    "<X:{} xmlns:X=\"{}\"/>",
                    name.name,
                    xml_escape(&name.ns)
                );
            }
        };
        let write_value = |out: &mut String, name: &PropName, value: &Value| {
            let prefix = prefix_of(&name.ns).unwrap_or("D");
            let inner = match value {
                Value::Xml(xml) => xml.clone(),
                Value::Text(text) => xml_escape(text),
            };
            let _ = write!(out, "<{prefix}:{0}>{inner}</{prefix}:{0}>", name.name);
        };
        match request {
            PropRequest::All => {
                for (name, value) in known.iter().filter(|(n, _)| n.name != "calendar-data") {
                    write_value(&mut found, name, value);
                }
            }
            PropRequest::Names => {
                for (name, _) in known {
                    write_name(&mut found, name);
                }
            }
            PropRequest::Props(wanted) => {
                for name in wanted {
                    match known.iter().find(|(n, _)| n == name) {
                        Some((name, value)) => write_value(&mut found, name, value),
                        None => write_name(&mut missing, name),
                    }
                }
            }
        }
        if !found.is_empty() {
            let _ = write!(
                self.body,
                "<D:propstat><D:prop>{found}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat>"
            );
        }
        if !missing.is_empty() {
            let _ = write!(
                self.body,
                "<D:propstat><D:prop>{missing}</D:prop><D:status>HTTP/1.1 404 Not Found</D:status></D:propstat>"
            );
        }
        self.body.push_str("</D:response>\n");
    }

    /// Adds a `404` response for a member that does not exist (multiget)
    /// or was removed (sync).
    pub fn not_found(&mut self, href: &str) {
        let _ = writeln!(
            self.body,
            "<D:response><D:href>{}</D:href><D:status>HTTP/1.1 404 Not Found</D:status></D:response>",
            xml_escape(href)
        );
    }

    /// Adds the root or principal resource.
    pub fn principal(&mut self, path: &str, user: &str, display_name: &str, request: &PropRequest) {
        let principal = href(&Resource::Principal {
            user: user.to_owned(),
        });
        let home = href(&Resource::Home {
            user: user.to_owned(),
        });
        let link = |h: &str| Value::Xml(format!("<D:href>{}</D:href>", xml_escape(h)));
        let resourcetype = if path == href(&Resource::Root) {
            "<D:collection/>"
        } else {
            "<D:principal/>"
        };
        let known = vec![
            (
                PropName::new(NS_DAV, "resourcetype"),
                Value::Xml(resourcetype.into()),
            ),
            (
                PropName::new(NS_DAV, "displayname"),
                Value::Text(display_name.into()),
            ),
            (
                PropName::new(NS_DAV, "current-user-principal"),
                link(&principal),
            ),
            (PropName::new(NS_DAV, "principal-URL"), link(&principal)),
            (PropName::new(NS_CALDAV, "calendar-home-set"), link(&home)),
        ];
        self.response(path, request, &known);
    }

    /// Adds the calendar home.
    pub fn home(&mut self, user: &str, request: &PropRequest) {
        let path = href(&Resource::Home {
            user: user.to_owned(),
        });
        let principal = href(&Resource::Principal {
            user: user.to_owned(),
        });
        let known = vec![
            (
                PropName::new(NS_DAV, "resourcetype"),
                Value::Xml("<D:collection/>".into()),
            ),
            (
                PropName::new(NS_DAV, "current-user-principal"),
                Value::Xml(format!("<D:href>{}</D:href>", xml_escape(&principal))),
            ),
        ];
        self.response(&path, request, &known);
    }

    /// Adds a task list collection.
    pub fn list(&mut self, user: &str, list: &ListInfo, request: &PropRequest) {
        let path = href(&Resource::List {
            user: user.to_owned(),
            list: list.id.clone(),
        });
        let principal = href(&Resource::Principal {
            user: user.to_owned(),
        });
        let mut known = vec![
            (
                PropName::new(NS_DAV, "resourcetype"),
                Value::Xml("<D:collection/><C:calendar/>".into()),
            ),
            (PropName::new(NS_DAV, "displayname"), Value::Text(list.title.clone())),
            (
                PropName::new(NS_CALDAV, "supported-calendar-component-set"),
                Value::Xml("<C:comp name=\"VTODO\"/>".into()),
            ),
            (
                PropName::new(NS_DAV, "supported-report-set"),
                Value::Xml(
                    "<D:supported-report><D:report><C:calendar-query/></D:report></D:supported-report>\
                     <D:supported-report><D:report><C:calendar-multiget/></D:report></D:supported-report>\
                     <D:supported-report><D:report><D:sync-collection/></D:report></D:supported-report>"
                        .into(),
                ),
            ),
            (
                PropName::new(NS_DAV, "current-user-privilege-set"),
                Value::Xml(
                    "<D:privilege><D:read/></D:privilege><D:privilege><D:write/></D:privilege>"
                        .into(),
                ),
            ),
            (
                PropName::new(NS_DAV, "owner"),
                Value::Xml(format!("<D:href>{}</D:href>", xml_escape(&principal))),
            ),
            (PropName::new(NS_CS, "getctag"), Value::Text(list.version.to_string())),
            (PropName::new(NS_DAV, "sync-token"), Value::Text(sync_token(list))),
        ];
        if let Some(color) = &list.color {
            known.push((
                PropName::new(NS_APPLE, "calendar-color"),
                Value::Text(color.clone()),
            ));
        }
        self.response(&path, request, &known);
    }

    /// Adds a task object.
    pub fn object(&mut self, user: &str, list: &str, object: &Object, request: &PropRequest) {
        let path = href(&Resource::Object {
            user: user.to_owned(),
            list: list.to_owned(),
            uid: object.uid.clone(),
        });
        let known = vec![
            (
                PropName::new(NS_DAV, "resourcetype"),
                Value::Xml(String::new()),
            ),
            (
                PropName::new(NS_DAV, "getetag"),
                Value::Text(etag(&object.ical)),
            ),
            (
                PropName::new(NS_DAV, "getcontenttype"),
                Value::Text("text/calendar; charset=utf-8; component=VTODO".into()),
            ),
            (
                PropName::new(NS_CALDAV, "calendar-data"),
                Value::Text(object.ical.clone()),
            ),
        ];
        self.response(&path, request, &known);
    }

    /// Adds the `sync-token` element of a `sync-collection` response.
    pub fn sync_token(&mut self, list: &ListInfo) {
        let _ = writeln!(
            self.body,
            "<D:sync-token>{}</D:sync-token>",
            xml_escape(&sync_token(list))
        );
    }

    /// Finishes the response body (status 207).
    #[must_use]
    pub fn finish(mut self) -> String {
        self.body.push_str("</D:multistatus>\n");
        telemetry::record_calendar("caldav", self.body.len());
        self.body
    }
}

/// Whether an object is still open, for `calendar-query` filters.
#[must_use]
pub fn is_open(object: &Object) -> bool {
    !object.ical.contains("\r\nSTATUS:COMPLETED\r\n")
}

/// The `Allow` and `DAV` headers for `OPTIONS`.
pub const ALLOW: &str = "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, REPORT";
/// The `DAV` compliance classes.
pub const DAV_HEADER: &str = "1, 3, calendar-access";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_parse_and_print() {
        for (path, resource) in [
            ("/dav/", Resource::Root),
            (
                "/dav/principals/ann/",
                Resource::Principal { user: "ann".into() },
            ),
            ("/dav/calendars/ann/", Resource::Home { user: "ann".into() }),
            (
                "/dav/calendars/ann/errands/",
                Resource::List {
                    user: "ann".into(),
                    list: "errands".into(),
                },
            ),
            (
                "/dav/calendars/ann/errands/A%20B@vellum.ics",
                Resource::Object {
                    user: "ann".into(),
                    list: "errands".into(),
                    uid: "A B@vellum".into(),
                },
            ),
        ] {
            assert_eq!(parse_path(path), Some(resource.clone()), "{path}");
            assert_eq!(href(&resource), path);
        }
        assert_eq!(parse_path("/dav/calendars/ann/errands/x.txt"), None);
        assert_eq!(parse_path("/other"), None);
        assert_eq!(parse_path("/dav/calendars/ann/l/%zz.ics"), None);
    }

    #[test]
    fn reads_propfind_and_report_bodies() {
        let propfind = r#"<?xml version="1.0"?>
<d:propfind xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop><d:displayname/><cs:getctag/><x:unknown xmlns:x="urn:x"/></d:prop>
</d:propfind>"#;
        let PropRequest::Props(props) = parse_propfind(propfind).unwrap() else {
            panic!("expected props");
        };
        assert_eq!(props.len(), 3);
        assert_eq!(props[1], PropName::new(NS_CS, "getctag"));
        assert_eq!(parse_propfind("").unwrap(), PropRequest::All);

        let query = r#"<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VTODO">
    <c:prop-filter name="COMPLETED"><c:is-not-defined/></c:prop-filter>
  </c:comp-filter></c:comp-filter></c:filter>
</c:calendar-query>"#;
        assert!(matches!(
            parse_report(query).unwrap(),
            Report::Query {
                open_only: true,
                ..
            }
        ));

        let multiget = r#"<C:calendar-multiget xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <D:prop><D:getetag/><C:calendar-data/></D:prop>
  <D:href>/dav/calendars/ann/l/a.ics</D:href><D:href>/dav/calendars/ann/l/b&amp;c.ics</D:href>
</C:calendar-multiget>"#;
        let Report::Multiget { hrefs, .. } = parse_report(multiget).unwrap() else {
            panic!("expected multiget");
        };
        assert_eq!(
            hrefs,
            ["/dav/calendars/ann/l/a.ics", "/dav/calendars/ann/l/b&c.ics"]
        );

        let sync = r#"<d:sync-collection xmlns:d="DAV:"><d:sync-token>tok</d:sync-token>
  <d:sync-level>1</d:sync-level><d:prop><d:getetag/></d:prop></d:sync-collection>"#;
        assert_eq!(
            parse_report(sync).unwrap(),
            Report::Sync {
                props: PropRequest::Props(vec![PropName::new(NS_DAV, "getetag")]),
                token: Some("tok".into())
            }
        );
        assert!(parse_report("<d:other xmlns:d=\"DAV:\"/>").is_err());
        assert!(parse_report("<unclosed").is_err() || parse_report("<unclosed").is_ok());
        assert!(parse_propfind("<a><b></a>").is_err());
    }

    #[test]
    fn preconditions() {
        let tag = "\"abc\"";
        assert!(check_preconditions(None, None, Some("*")).is_ok());
        assert_eq!(
            check_preconditions(Some(tag), None, Some("*")),
            Err(DavError::PreconditionFailed)
        );
        assert!(check_preconditions(Some(tag), Some(tag), None).is_ok());
        assert_eq!(
            check_preconditions(Some(tag), Some("\"old\""), None),
            Err(DavError::PreconditionFailed)
        );
        assert_eq!(
            check_preconditions(None, Some("*"), None),
            Err(DavError::PreconditionFailed)
        );
    }

    #[test]
    fn writes_multistatus() {
        let list = ListInfo {
            id: "errands".into(),
            title: "Errands & co".into(),
            version: 7,
            color: Some("#4F5B92".into()),
        };
        let object = Object {
            uid: "x@vellum".into(),
            ical: "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".into(),
        };
        let mut ms = Multistatus::new();
        ms.list(
            "ann",
            &list,
            &PropRequest::Props(vec![
                PropName::new(NS_DAV, "displayname"),
                PropName::new(NS_CS, "getctag"),
                PropName::new("urn:x", "unknown"),
            ]),
        );
        ms.object("ann", "errands", &object, &PropRequest::All);
        ms.sync_token(&list);
        let body = ms.finish();
        assert!(body.contains("<D:href>/dav/calendars/ann/errands/</D:href>"));
        assert!(
            body.contains(
                "<D:displayname>Errands &amp; co</D:displayname><CS:getctag>7</CS:getctag>"
            )
        );
        assert!(
            body.contains(
                "<X:unknown xmlns:X=\"urn:x\"/></D:prop><D:status>HTTP/1.1 404 Not Found"
            )
        );
        assert!(body.contains(&format!(
            "<D:getetag>{}</D:getetag>",
            xml_escape(&etag(&object.ical))
        )));
        assert!(
            !body.contains("calendar-data"),
            "allprop leaves out calendar-data"
        );
        assert!(
            body.contains("<D:sync-token>https://vellum.invalid/sync/errands/7</D:sync-token>")
        );
        assert_eq!(check_sync_token(&list, Some(&sync_token(&list))), Ok(true));
        assert_eq!(check_sync_token(&list, None), Ok(false));
        assert_eq!(
            check_sync_token(&list, Some("old")),
            Err(DavError::InvalidSyncToken)
        );
        assert_eq!(DavError::InvalidSyncToken.status(), 403);
        assert!(
            DavError::InvalidSyncToken
                .body()
                .unwrap()
                .contains("valid-sync-token")
        );
        assert!(is_open(&object));
    }

    #[test]
    fn principal_discovery() {
        let mut ms = Multistatus::new();
        ms.principal(
            "/dav/",
            "ann",
            "Ann",
            &PropRequest::Props(vec![PropName::new(NS_DAV, "current-user-principal")]),
        );
        ms.principal(
            "/dav/principals/ann/",
            "ann",
            "Ann",
            &PropRequest::Props(vec![PropName::new(NS_CALDAV, "calendar-home-set")]),
        );
        ms.home("ann", &PropRequest::All);
        let body = ms.finish();
        assert!(body.contains("<D:current-user-principal><D:href>/dav/principals/ann/</D:href>"));
        assert!(body.contains("<C:calendar-home-set><D:href>/dav/calendars/ann/</D:href>"));
    }
}
