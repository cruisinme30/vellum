// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Email intake: mail to a team's intake address files an issue, and a
//! reply to a notification email adds a comment.
//!
//! | Address | Effect |
//! |---|---|
//! | `ENG@intake.northstar.dev` or `issues+ENG@northstar.dev` | new issue in ENG's triage |
//! | `reply+ENG-142.<user>.<tag>@northstar.dev` | comment on ENG-142 as `<user>` |
//!
//! The reply tag is an HMAC of the issue key, the user ID and the address
//! the notification went to, so a reply address cannot be forged for
//! another issue or person, and a forwarded notification does not let
//! someone else comment as the user. With no reply secret configured,
//! replies are refused. Mail marked
//! `Auto-Submitted` or `Precedence: bulk/list/junk` is dropped, which stops
//! loops with auto-responders. The mail adapter passes only messages its
//! SPF/DKIM/DMARC check accepted.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::Routing;
use super::text::Body;
use crate::inbound::InboundError;
use crate::mac;
use crate::model::{IssueDraft, IssueKey, Source, one_line};

/// A parsed message, reduced to what intake needs.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mail {
    /// The sender's address, lower case.
    pub from: String,
    /// The sender's display name.
    pub from_name: String,
    /// Every recipient address (To, Cc, Delivered-To), lower case.
    pub recipients: Vec<String>,
    /// The decoded subject.
    pub subject: String,
    /// The Message-ID without angle brackets.
    pub message_id: String,
    /// Whether the message is automatic (auto-reply, bulk, list).
    pub automatic: bool,
    /// The plain-text body (HTML converted when there is no text part).
    pub text: String,
}

struct Part<'a> {
    headers: Vec<(String, String)>,
    body: &'a [u8],
}

impl Part<'_> {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn split_part(raw: &[u8]) -> Part<'_> {
    let (head, body) = match (find(raw, b"\r\n\r\n"), find(raw, b"\n\n")) {
        (Some(crlf), Some(lf)) if lf < crlf => (raw.get(..lf), raw.get(lf + 2..)),
        (Some(crlf), _) => (raw.get(..crlf), raw.get(crlf + 4..)),
        (None, Some(lf)) => (raw.get(..lf), raw.get(lf + 2..)),
        (None, None) => (Some(raw), None),
    };
    let head = String::from_utf8_lossy(head.unwrap_or_default());
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = headers.last_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
        } else if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_owned(), value.trim().to_owned()));
        }
    }
    Part {
        headers,
        body: body.unwrap_or_default(),
    }
}

/// The value of a `key=value` parameter in a structured header.
fn param(header: &str, key: &str) -> Option<String> {
    header.split(';').skip(1).find_map(|p| {
        let (k, v) = p.split_once('=')?;
        k.trim()
            .eq_ignore_ascii_case(key)
            .then(|| v.trim().trim_matches('"').to_owned())
    })
}

fn decode_qp(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while let Some(&b) = input.get(i) {
        if b == b'=' {
            match (input.get(i + 1), input.get(i + 2)) {
                (Some(b'\r'), Some(b'\n')) => i += 3,
                (Some(b'\n'), _) => i += 2,
                (Some(h), Some(l)) => {
                    let hex = [*h, *l];
                    match std::str::from_utf8(&hex)
                        .ok()
                        .and_then(|s| u8::from_str_radix(s, 16).ok())
                    {
                        Some(byte) => out.push(byte),
                        None => out.extend_from_slice(&[b'=', *h, *l]),
                    }
                    i += 3;
                }
                _ => {
                    out.push(b);
                    i += 1;
                }
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    out
}

fn decode_charset(bytes: &[u8], charset: &str) -> String {
    match charset.to_ascii_lowercase().as_str() {
        "iso-8859-1" | "latin1" | "windows-1252" => bytes.iter().map(|b| char::from(*b)).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

fn decode_body(part: &Part<'_>) -> String {
    let encoding = part
        .header("Content-Transfer-Encoding")
        .unwrap_or("7bit")
        .to_ascii_lowercase();
    let bytes = match encoding.as_str() {
        "base64" => {
            let compact: Vec<u8> = part
                .body
                .iter()
                .copied()
                .filter(|b| !b.is_ascii_whitespace())
                .collect();
            STANDARD.decode(compact).unwrap_or_default()
        }
        "quoted-printable" => decode_qp(part.body),
        _ => part.body.to_vec(),
    };
    let charset = part
        .header("Content-Type")
        .and_then(|ct| param(ct, "charset"))
        .unwrap_or_default();
    decode_charset(&bytes, &charset)
}

/// Decodes RFC 2047 encoded words (`=?UTF-8?B?…?=`, `=?…?Q?…?=`).
fn decode_words(value: &str) -> String {
    let mut out = String::new();
    let mut rest = value;
    let mut last_was_word = false;
    while let Some(start) = rest.find("=?") {
        let (before, tail) = rest.split_at(start);
        let decoded = tail.get(2..).and_then(|inner| {
            let mut pieces = inner.splitn(3, '?');
            let charset = pieces.next()?;
            let encoding = pieces.next()?;
            let remainder = pieces.next()?;
            let end = remainder.find("?=")?;
            let text = remainder.get(..end)?;
            let bytes = match encoding.to_ascii_uppercase().as_str() {
                "B" => STANDARD.decode(text).ok()?,
                "Q" => decode_qp(text.replace('_', " ").as_bytes()),
                _ => return None,
            };
            let consumed = 2 + charset.len() + 1 + encoding.len() + 1 + end + 2;
            Some((decode_charset(&bytes, charset), consumed))
        });
        if let Some((word, consumed)) = decoded {
            if !(last_was_word && before.trim().is_empty()) {
                out.push_str(before);
            }
            out.push_str(&word);
            rest = tail.get(consumed..).unwrap_or_default();
            last_was_word = true;
        } else {
            out.push_str(before);
            out.push_str("=?");
            rest = tail.get(2..).unwrap_or_default();
            last_was_word = false;
        }
    }
    out.push_str(rest);
    out
}

/// Splits `Name <addr>, other@x` into `(name, address)` pairs.
fn addresses(value: &str) -> Vec<(String, String)> {
    value
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            let (name, addr) = match (item.rfind('<'), item.rfind('>')) {
                (Some(open), Some(close)) if open < close => (
                    item.get(..open).unwrap_or_default(),
                    item.get(open + 1..close).unwrap_or_default(),
                ),
                _ => ("", item),
            };
            let addr = addr.trim().to_ascii_lowercase();
            addr.contains('@')
                .then(|| (decode_words(name.trim().trim_matches('"')), addr))
        })
        .collect()
}

fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The best text body of a (possibly multipart) part.
fn text_of(part: &Part<'_>, depth: usize) -> Option<(bool, String)> {
    let content_type = part
        .header("Content-Type")
        .unwrap_or("text/plain")
        .to_ascii_lowercase();
    if content_type.starts_with("multipart/") && depth < 5 {
        let boundary = param(part.header("Content-Type").unwrap_or_default(), "boundary")?;
        let delimiter = format!("--{boundary}");
        let body = String::from_utf8_lossy(part.body);
        let mut html = None;
        for chunk in body.split(delimiter.as_str()).skip(1) {
            if chunk.starts_with("--") {
                break;
            }
            let chunk = chunk.trim_start_matches(['\r', '\n']);
            let child = split_part(chunk.as_bytes());
            match text_of(&child, depth + 1) {
                Some((true, text)) => return Some((true, text)),
                Some((false, text)) => html = html.or(Some(text)),
                None => {}
            }
        }
        return html.map(|h| (false, h));
    }
    if content_type.starts_with("text/plain") {
        return Some((true, decode_body(part)));
    }
    if content_type.starts_with("text/html") {
        return Some((false, html_to_text(&decode_body(part))));
    }
    None
}

/// Parses a raw RFC 5322 message.
///
/// # Errors
///
/// [`InboundError::Malformed`] if there is no sender.
pub fn parse(raw: &[u8]) -> Result<Mail, InboundError> {
    let part = split_part(raw);
    let (from_name, from) = part
        .header("From")
        .and_then(|v| addresses(v).into_iter().next())
        .ok_or_else(|| InboundError::malformed("no From address"))?;
    let mut recipients = Vec::new();
    for name in ["To", "Cc", "Delivered-To", "X-Original-To"] {
        for (key, value) in &part.headers {
            if key.eq_ignore_ascii_case(name) {
                recipients.extend(addresses(value).into_iter().map(|(_, a)| a));
            }
        }
    }
    let automatic = part
        .header("Auto-Submitted")
        .is_some_and(|v| !v.trim().eq_ignore_ascii_case("no"))
        || part.header("Precedence").is_some_and(|v| {
            ["bulk", "list", "junk"].contains(&v.trim().to_ascii_lowercase().as_str())
        })
        || part.header("List-Id").is_some();
    Ok(Mail {
        from,
        from_name,
        recipients,
        subject: decode_words(part.header("Subject").unwrap_or_default()),
        message_id: part
            .header("Message-ID")
            .unwrap_or_default()
            .trim_matches(['<', '>', ' '])
            .to_owned(),
        automatic,
        text: text_of(&part, 0)
            .map(|(_, t)| t)
            .unwrap_or_default()
            .replace("\r\n", "\n"),
    })
}

/// Drops quoted history and signatures from a reply.
#[must_use]
pub fn strip_quoted(text: &str) -> String {
    let mut kept = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let starts_history = trimmed == "-- "
            || trimmed == "--"
            || trimmed.starts_with("-----Original Message-----")
            || trimmed.starts_with("________________________________")
            || (trimmed.starts_with("On ") && trimmed.ends_with("wrote:"));
        if starts_history {
            break;
        }
        if !trimmed.starts_with('>') {
            kept.push(line.trim_end());
        }
    }
    kept.join("\n").trim().to_owned()
}

/// The tag binds the issue, the user and the address the notification was
/// sent to, so a reply only counts when it comes back from that address.
/// A forwarded notification or a mailing-list copy carries a valid tag,
/// but whoever replies from another address is not taken for the user.
fn reply_tag(secret: &[u8], key: &IssueKey, user: &str, email: &str) -> String {
    let user = user.to_ascii_lowercase();
    let email = email.trim().to_lowercase();
    let tag = mac::hmac_sha256(
        secret,
        &[
            key.to_string().as_bytes(),
            b":",
            user.as_bytes(),
            b":",
            email.as_bytes(),
        ],
    );
    mac::hex(tag.get(..10).unwrap_or_default())
}

/// The reply address for notification emails about `key` sent to `user`
/// at `email`. Replies are accepted only from `email`.
#[must_use]
pub fn reply_address(
    secret: &[u8],
    key: &IssueKey,
    user: &str,
    email: &str,
    domain: &str,
) -> String {
    format!(
        "reply+{key}.{user}.{}@{domain}",
        reply_tag(secret, key, user, email)
    )
    .to_ascii_lowercase()
}

/// What an incoming email does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmailIntake {
    /// File a new issue.
    NewIssue(Box<IssueDraft>),
    /// Add a comment.
    Comment {
        /// The issue.
        key: IssueKey,
        /// The user the reply address belongs to.
        user: String,
        /// The reply without quoted history.
        body: String,
    },
    /// Drop it, for the reason given.
    Ignored(&'static str),
}

/// Settings for email intake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailSettings {
    /// Team routing.
    pub routing: Routing,
    /// The domain whose local part is a team key (`intake.northstar.dev`).
    pub intake_domain: String,
    /// The `issues+TEAM@` prefix accepted on any domain.
    pub plus_prefix: String,
    /// The domain of reply addresses.
    pub reply_domain: String,
    /// The secret for reply tags.
    pub reply_secret: Vec<u8>,
}

/// Decides what to do with a parsed message.
#[must_use]
pub fn classify(mail: &Mail, settings: &EmailSettings) -> EmailIntake {
    if mail.automatic {
        return EmailIntake::Ignored("automatic message");
    }
    for recipient in &mail.recipients {
        let Some((local, domain)) = recipient.split_once('@') else {
            continue;
        };
        if domain == settings.reply_domain
            && let Some(rest) = local.strip_prefix("reply+")
        {
            let mut parts = rest.splitn(3, '.');
            let (Some(key), Some(user), Some(tag)) = (parts.next(), parts.next(), parts.next())
            else {
                return EmailIntake::Ignored("malformed reply address");
            };
            let Ok(key) = IssueKey::parse(key) else {
                return EmailIntake::Ignored("malformed reply address");
            };
            // With no secret anyone could compute a tag, so replies are
            // refused rather than trusted.
            if settings.reply_secret.is_empty() {
                return EmailIntake::Ignored("reply addresses are not configured");
            }
            // Addresses may be lower-cased in transit, so tags are computed
            // over the lower-case user ID (ULIDs are case-insensitive).
            // The sender is part of the tag, so a reply from anyone but the
            // notified address does not match.
            let expected = reply_tag(&settings.reply_secret, &key, user, &mail.from);
            if !mac::equal(expected.as_bytes(), tag.as_bytes()) {
                return EmailIntake::Ignored("reply address tag does not match");
            }
            let body = strip_quoted(&mail.text);
            if body.is_empty() {
                return EmailIntake::Ignored("empty reply");
            }
            return EmailIntake::Comment {
                key,
                user: user.to_owned(),
                body,
            };
        }
        let team = if domain == settings.intake_domain {
            settings.routing.team(local)
        } else {
            local
                .strip_prefix(&format!("{}+", settings.plus_prefix))
                .and_then(|team| settings.routing.team(team))
        };
        if let Some(team) = team {
            let title = one_line(mail.subject.trim(), 255);
            let title = if title.is_empty() {
                one_line(&mail.text, 120)
            } else {
                title
            };
            if title.is_empty() {
                return EmailIntake::Ignored("empty message");
            }
            let mut draft = IssueDraft::new(
                team,
                title,
                Source::Email {
                    from: mail.from.clone(),
                    message_id: mail.message_id.clone(),
                },
            );
            draft.description = Body::new().text(&strip_quoted(&mail.text)).finish();
            draft.reporter = Some(mail.from.clone());
            return EmailIntake::NewIssue(Box::new(draft));
        }
    }
    EmailIntake::Ignored("no intake recipient")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> EmailSettings {
        EmailSettings {
            routing: Routing {
                default_team: "ENG".to_owned(),
                teams: ["ENG", "OPS"].iter().map(|t| (*t).to_owned()).collect(),
            },
            intake_domain: "intake.northstar.dev".to_owned(),
            plus_prefix: "issues".to_owned(),
            reply_domain: "northstar.dev".to_owned(),
            reply_secret: b"reply-secret".to_vec(),
        }
    }

    const PLAIN: &str = "From: \"Ada Park\" <Ada@Northstar.dev>\r\n\
To: ops@intake.northstar.dev\r\n\
Subject: =?UTF-8?B?RGlzayBmdWxs?= =?UTF-8?Q?_on_db=E2=80=912?=\r\n\
Message-ID: <abc@mail.northstar.dev>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
The disk on db-2 is at 98=25.\r\n\
Please look soon=\r\n\
.\r\n";

    #[test]
    fn plain_message_files_an_issue() -> Result<(), InboundError> {
        let mail = parse(PLAIN.as_bytes())?;
        assert_eq!(mail.from, "ada@northstar.dev");
        assert_eq!(mail.from_name, "Ada Park");
        assert_eq!(mail.subject, "Disk full on db\u{2011}2");
        assert_eq!(mail.message_id, "abc@mail.northstar.dev");
        assert_eq!(
            mail.text,
            "The disk on db-2 is at 98%.\nPlease look soon.\n"
        );
        let EmailIntake::NewIssue(draft) = classify(&mail, &settings()) else {
            return Err(InboundError::malformed("expected an issue"));
        };
        assert_eq!(draft.team, "OPS");
        assert_eq!(draft.title, "Disk full on db\u{2011}2");
        assert_eq!(draft.reporter.as_deref(), Some("ada@northstar.dev"));
        Ok(())
    }

    #[test]
    fn multipart_prefers_text_and_plus_addresses_route() -> Result<(), InboundError> {
        let raw = "From: bo@northstar.dev\nTo: issues+eng@northstar.dev\nSubject: Crash\n\
Content-Type: multipart/alternative; boundary=\"b1\"\n\n\
--b1\nContent-Type: text/html; charset=utf-8\n\n<p>HTML &amp; body</p>\n\
--b1\nContent-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: base64\n\nQ3Jhc2ggb24gc3RhcnQ=\n\
--b1--\n";
        let mail = parse(raw.as_bytes())?;
        assert_eq!(mail.text.trim(), "Crash on start");
        assert!(
            matches!(classify(&mail, &settings()), EmailIntake::NewIssue(d) if d.team == "ENG")
        );

        let html_only = "From: bo@northstar.dev\nTo: issues+eng@northstar.dev\nSubject: X\n\
Content-Type: multipart/alternative; boundary=b2\n\n--b2\nContent-Type: text/html\n\n<p>Only &amp; html</p>\n--b2--\n";
        assert_eq!(parse(html_only.as_bytes())?.text.trim(), "Only & html");
        Ok(())
    }

    #[test]
    fn replies_become_comments() -> Result<(), InboundError> {
        let key = IssueKey::parse("ENG-142").map_err(InboundError::malformed)?;
        let address = reply_address(
            b"reply-secret",
            &key,
            "01JD3Z",
            "bo@northstar.dev",
            "northstar.dev",
        );
        let raw = format!(
            "From: bo@northstar.dev\nTo: {address}\nSubject: Re: ENG-142\n\n\
Looks good, shipping it.\n\nOn Tue, Nov 24, 2026 at 9:00 AM Vellum wrote:\n> old text\n"
        );
        let mail = parse(raw.as_bytes())?;
        assert_eq!(
            classify(&mail, &settings()),
            EmailIntake::Comment {
                key: key.clone(),
                user: "01jd3z".to_owned(),
                body: "Looks good, shipping it.".to_owned()
            }
        );
        let forged = raw.replace(
            &address,
            "reply+eng-143.01jd3z.00000000000000000000@northstar.dev",
        );
        assert_eq!(
            classify(&parse(forged.as_bytes())?, &settings()),
            EmailIntake::Ignored("reply address tag does not match")
        );
        // A forwarded notification: the tag is valid but the sender is not
        // the user it was sent to.
        let forwarded = raw.replace("From: bo@", "From: eve@");
        assert_eq!(
            classify(&parse(forwarded.as_bytes())?, &settings()),
            EmailIntake::Ignored("reply address tag does not match")
        );
        // A tag made with an empty key must not pass when no secret is set.
        let unset = EmailSettings {
            reply_secret: Vec::new(),
            ..settings()
        };
        let keyless = raw.replace(
            &address,
            &reply_address(b"", &key, "01JD3Z", "bo@northstar.dev", "northstar.dev"),
        );
        assert_eq!(
            classify(&parse(keyless.as_bytes())?, &unset),
            EmailIntake::Ignored("reply addresses are not configured")
        );
        Ok(())
    }

    #[test]
    fn automatic_and_unrouted_mail_is_ignored() -> Result<(), InboundError> {
        let auto = "From: a@b.dev\nTo: eng@intake.northstar.dev\nAuto-Submitted: auto-replied\nSubject: Out of office\n\nAway";
        assert_eq!(
            classify(&parse(auto.as_bytes())?, &settings()),
            EmailIntake::Ignored("automatic message")
        );
        let other = "From: a@b.dev\nTo: someone@else.dev\nSubject: Hi\n\nHello";
        assert_eq!(
            classify(&parse(other.as_bytes())?, &settings()),
            EmailIntake::Ignored("no intake recipient")
        );
        assert!(parse(b"Subject: no sender\n\nx").is_err());
        Ok(())
    }

    #[test]
    fn strips_signatures_and_outlook_history() {
        let text = "Thanks!\n\n-- \nBo\n";
        assert_eq!(strip_quoted(text), "Thanks!");
        let outlook = "Done.\n-----Original Message-----\nFrom: x";
        assert_eq!(strip_quoted(outlook), "Done.");
    }
}
