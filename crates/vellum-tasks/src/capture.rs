// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Capture from e-mail and Slack into the Inbox (R-MW-8).
//!
//! **E-mail.** Each user has a secret address `tasks+<token>@<domain>`;
//! the token is a kernel [`ShareToken`] stored as its hash, like the ICS
//! feed's. A forwarded message becomes an Inbox task: the subject (minus
//! `Re:`/`Fwd:`) is read by quick-add, so "Pay invoice friday p1" works
//! from a phone, and the plain-text body becomes the task's notes.
//!
//! **Slack.** A "Save as task" message shortcut posts the message to
//! Vellum. The request is accepted only with a valid `v0` signature made
//! with the workspace's signing secret, at most five minutes old. The
//! task links back to the message.
//!
//! Captured text is untrusted: it is escaped so it stays plain text, and
//! nothing in it can set fields other than through quick-add.

use std::fmt::Write as _;

use hmac::{Hmac, KeyInit as _, Mac as _};
use jiff::Timestamp;
use jiff::civil::DateTime;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use subtle::ConstantTimeEq as _;
use vellum_kernel::{BlockId, ShareToken};

use crate::checklist;
use crate::model::TaskFields;
use crate::quickadd;
use crate::telemetry;

/// The local-part prefix of capture addresses.
pub const ADDRESS_PREFIX: &str = "tasks+";

/// The longest notes kept from a captured body.
pub const MAX_NOTES_CHARS: usize = 4_000;

/// A task ready to be added to the Inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    /// The title.
    pub title: String,
    /// Fields from quick-add, with `src` set.
    pub fields: TaskFields,
    /// Notes, written as indented lines under the item.
    pub notes: Option<String>,
    /// People named with `+person`.
    pub assignees: Vec<String>,
}

/// Why a capture was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    /// The address is not a capture address with a well-formed token.
    #[error("not a capture address")]
    Address,
    /// The message could not be parsed.
    #[error("cannot read the message")]
    Message,
    /// The Slack signature is missing, wrong or too old.
    #[error("invalid Slack signature")]
    Signature,
    /// The Slack payload is not a message shortcut.
    #[error("unsupported Slack payload: {0}")]
    Payload(String),
}

/// The secret address for a token.
#[must_use]
pub fn address(token: &ShareToken, domain: &str) -> String {
    format!("{ADDRESS_PREFIX}{}@{domain}", token.expose())
}

/// Reads the token from a recipient address such as
/// `"Tasks" <tasks+abc…@northstar.dev>`.
///
/// # Errors
///
/// Returns [`CaptureError::Address`] for any other address.
pub fn token_from_address(recipient: &str) -> Result<ShareToken, CaptureError> {
    let address = recipient
        .rsplit_once('<')
        .map_or(recipient, |(_, a)| a.trim_end_matches('>'))
        .trim();
    let local = address.split_once('@').map_or(address, |(l, _)| l);
    let token = local
        .get(..ADDRESS_PREFIX.len())
        .filter(|p| p.eq_ignore_ascii_case(ADDRESS_PREFIX))
        .and_then(|_| local.get(ADDRESS_PREFIX.len()..))
        .ok_or(CaptureError::Address)?;
    token.parse().map_err(|_| CaptureError::Address)
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '<' | '{' | '^' | '[' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn short_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().take(8).fold(
        String::new(),
        |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        },
    )
}

fn strip_reply_prefixes(subject: &str) -> &str {
    let mut subject = subject.trim();
    loop {
        let lower = subject.to_ascii_lowercase();
        let Some(prefix) = ["re:", "fw:", "fwd:", "aw:", "wg:"]
            .iter()
            .find(|p| lower.starts_with(*p))
        else {
            return subject;
        };
        subject = subject.get(prefix.len()..).unwrap_or_default().trim_start();
    }
}

fn clip(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut out: String = text.chars().take(MAX_NOTES_CHARS).collect();
    if text.chars().count() > MAX_NOTES_CHARS {
        out.push('…');
    }
    Some(out)
}

/// Turns a raw RFC 5322 message into a capture. `now` is the owner's
/// local time, for quick-add.
///
/// # Errors
///
/// Returns [`CaptureError::Message`] when the message cannot be parsed.
pub fn from_email(raw: &[u8], now: DateTime) -> Result<Capture, CaptureError> {
    let _span =
        tracing::info_span!("tasks.capture.email", bytes = raw.len()).entered();
    let Some(message) = mail_parser::MessageParser::default().parse(raw) else {
        telemetry::record_capture("email", "unreadable");
        return Err(CaptureError::Message);
    };
    let subject = strip_reply_prefixes(message.subject().unwrap_or_default());
    let parsed = quickadd::parse(subject, now);
    let from = message
        .from()
        .and_then(|f| f.first())
        .and_then(|a| a.address())
        .map(str::to_owned);
    let mut notes = message.body_text(0).and_then(|b| clip(&b));
    if let Some(from) = &from {
        let line = format!("From: {from}");
        notes = Some(
            notes.map_or_else(|| line.clone(), |n| format!("{line}\n\n{n}")),
        );
    }
    let id = message
        .message_id()
        .map_or_else(|| short_hash(subject), short_hash);
    let mut fields = parsed.fields;
    fields.src = Some(format!("email:{id}"));
    let title = if parsed.title.is_empty() {
        "Untitled e-mail".to_owned()
    } else {
        parsed.title
    };
    telemetry::record_capture("email", "captured");
    Ok(Capture {
        title,
        fields,
        notes,
        assignees: parsed.assignees,
    })
}

/// Verifies a Slack request signature (`X-Slack-Signature`) over the raw
/// body, made `timestamp` seconds after the epoch; requests older than
/// five minutes are refused to stop replays.
///
/// # Errors
///
/// Returns [`CaptureError::Signature`] when the signature does not hold.
pub fn verify_slack(
    signing_secret: &[u8],
    timestamp: &str,
    signature: &str,
    body: &[u8],
    now: Timestamp,
) -> Result<(), CaptureError> {
    let sent: i64 = timestamp
        .trim()
        .parse()
        .map_err(|_| CaptureError::Signature)?;
    if (now.as_second() - sent).abs() > 300 {
        return Err(CaptureError::Signature);
    }
    let expected = signature
        .strip_prefix("v0=")
        .and_then(|hex| {
            (0..hex.len())
                .step_by(2)
                .map(|i| {
                    hex.get(i..i + 2)
                        .and_then(|b| u8::from_str_radix(b, 16).ok())
                })
                .collect::<Option<Vec<u8>>>()
        })
        .ok_or(CaptureError::Signature)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(signing_secret)
        .map_err(|_| CaptureError::Signature)?;
    mac.update(b"v0:");
    mac.update(timestamp.trim().as_bytes());
    mac.update(b":");
    mac.update(body);
    let actual = mac.finalize().into_bytes();
    if actual.as_slice().ct_eq(&expected).into() {
        Ok(())
    } else {
        Err(CaptureError::Signature)
    }
}

fn form_value(body: &str, key: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| percent_decode(v))?
    })
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                out.push(u8::from_str_radix(text.get(i + 1..i + 3)?, 16).ok()?);
                i += 2;
            }
            other => out.push(other),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

#[derive(Debug, Deserialize)]
struct SlackPayload {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    team: SlackRef,
    #[serde(default)]
    channel: SlackRef,
    #[serde(default)]
    user: SlackRef,
    #[serde(default)]
    message: SlackMessage,
    #[serde(default)]
    response_url: String,
}

#[derive(Debug, Deserialize, Default)]
struct SlackRef {
    #[serde(default)]
    id: String,
    #[serde(default)]
    domain: String,
}

#[derive(Debug, Deserialize, Default)]
struct SlackMessage {
    #[serde(default)]
    text: String,
    #[serde(default)]
    ts: String,
}

/// A Slack message shortcut, verified and read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackCapture {
    /// The Slack user who saved it; the identity service maps it to a
    /// Vellum user.
    pub slack_user: String,
    /// The Slack workspace (team) ID.
    pub team: String,
    /// The task.
    pub capture: Capture,
    /// Where to post a reply only the user sees; kept only when it is a
    /// Slack URL.
    pub response_url: Option<String>,
}

/// Turns Slack's mark-up into plain text: `<@U1>` to `@U1`, `<url|label>`
/// to `label (url)`, and entities decoded.
fn slack_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open + 1..).unwrap_or_default();
        let Some(close) = after.find('>') else {
            out.push_str(rest.get(open..).unwrap_or_default());
            rest = "";
            break;
        };
        let inner = after.get(..close).unwrap_or_default();
        match inner.split_once('|') {
            Some((target, label)) if !target.starts_with(['@', '#', '!']) => {
                let _ = write!(out, "{label} ({target})");
            }
            Some((_, label)) => out.push_str(label),
            None => out.push_str(inner),
        }
        rest = after.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// Reads a verified Slack "Save as task" message shortcut (the
/// form-encoded `payload=`). Call [`verify_slack`] on the raw body first.
///
/// # Errors
///
/// Returns [`CaptureError::Payload`] for anything but a message shortcut.
pub fn from_slack(body: &str) -> Result<SlackCapture, CaptureError> {
    let _span = tracing::info_span!("tasks.capture.slack").entered();
    let payload = form_value(body, "payload")
        .ok_or_else(|| CaptureError::Payload("no payload".into()))?;
    let payload: SlackPayload = serde_json::from_str(&payload)
        .map_err(|e| CaptureError::Payload(e.to_string()))?;
    if payload.kind != "message_action" {
        telemetry::record_capture("slack", "refused");
        return Err(CaptureError::Payload(payload.kind));
    }
    let text = slack_text(&payload.message.text);
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or_default();
    let mut title: String =
        first.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.chars().count() > 140 {
        title = title.chars().take(139).collect::<String>() + "…";
    }
    if title.is_empty() {
        "Slack message".clone_into(&mut title);
    }
    let link = format!(
        "https://{}.slack.com/archives/{}/p{}",
        payload.team.domain,
        payload.channel.id,
        payload.message.ts.replace('.', "")
    );
    let notes =
        clip(&text).map_or_else(|| link.clone(), |t| format!("{t}\n\n{link}"));
    telemetry::record_capture("slack", "captured");
    let response_url = Some(payload.response_url)
        .filter(|u| u.starts_with(SLACK_RESPONSE_PREFIX) && u.len() <= 512);
    Ok(SlackCapture {
        slack_user: payload.user.id,
        team: payload.team.id.clone(),
        response_url,
        capture: Capture {
            title,
            fields: TaskFields {
                src: Some(format!(
                    "slack:{}:{}:{}",
                    payload.team.id, payload.channel.id, payload.message.ts
                )),
                ..TaskFields::default()
            },
            notes: Some(notes),
            assignees: Vec::new(),
        },
    })
}

/// The only place Slack replies are posted (`response_url`).
pub const SLACK_RESPONSE_PREFIX: &str = "https://hooks.slack.com/";

/// How long a Slack link code is valid.
pub const SLACK_LINK_TTL: jiff::SignedDuration =
    jiff::SignedDuration::from_mins(15);

fn link_mac(secret: &[u8], team: &str, user: &str, expires: i64) -> Vec<u8> {
    #[expect(clippy::expect_used, reason = "HMAC takes keys of any length")]
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
    mac.update(b"vellum-slack-link\0");
    mac.update(team.as_bytes());
    mac.update(b"\0");
    mac.update(user.as_bytes());
    mac.update(b"\0");
    mac.update(expires.to_string().as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn is_slack_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A code that proves a Slack user asked to link their account (#163).
///
/// Slack shows it to them as a link; opening it while signed in to
/// Vellum links that Slack user to the Vellum account. It is signed with
/// the Slack signing secret and expires after [`SLACK_LINK_TTL`].
#[must_use]
pub fn slack_link_code(
    secret: &[u8],
    team: &str,
    user: &str,
    now: Timestamp,
) -> String {
    let expires = now.checked_add(SLACK_LINK_TTL).unwrap_or(now).as_second();
    let mac = link_mac(secret, team, user, expires);
    let tag: String = mac.get(..16).unwrap_or_default().iter().fold(
        String::new(),
        |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        },
    );
    format!("{team}.{user}.{expires}.{tag}")
}

/// Checks a [`slack_link_code`] and returns the Slack team and user.
///
/// # Errors
///
/// [`CaptureError::Signature`] for a forged, malformed or expired code.
pub fn read_slack_link_code(
    secret: &[u8],
    code: &str,
    now: Timestamp,
) -> Result<(String, String), CaptureError> {
    let bad = || CaptureError::Signature;
    let mut parts = code.trim().split('.');
    let (Some(team), Some(user), Some(expires), Some(tag), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(bad());
    };
    if !is_slack_id(team) || !is_slack_id(user) || tag.len() != 32 {
        return Err(bad());
    }
    let expires: i64 = expires.parse().map_err(|_| bad())?;
    if now.as_second() > expires {
        return Err(bad());
    }
    let expected = slack_link_code(
        secret,
        team,
        user,
        Timestamp::from_second(expires)
            .ok()
            .and_then(|t| t.checked_sub(SLACK_LINK_TTL).ok())
            .ok_or_else(bad)?,
    );
    let expected_tag = expected.rsplit('.').next().unwrap_or_default();
    if bool::from(expected_tag.as_bytes().ct_eq(tag.as_bytes())) {
        Ok((team.to_owned(), user.to_owned()))
    } else {
        Err(bad())
    }
}

/// Adds a capture to the end of an Inbox file, with its notes. A capture
/// whose `src` is already in the file (a retried delivery) is skipped and
/// `None` is returned.
#[must_use]
pub fn append_to_inbox(
    inbox: &str,
    capture: &Capture,
) -> Option<(String, BlockId)> {
    let list = checklist::Checklist::parse(inbox);
    if capture.fields.src.is_some()
        && list
            .items
            .iter()
            .any(|i| i.fields.src == capture.fields.src)
    {
        return None;
    }
    let block = BlockId::generate();
    let mut title = escape(&capture.title);
    for person in &capture.assignees {
        let _ = write!(title, " @{person}");
    }
    let mut out = inbox.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&checklist::render_item(
        0,
        false,
        &title,
        &capture.fields,
        block,
    ));
    out.push('\n');
    if let Some(notes) = &capture.notes {
        for line in notes.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let line = line.trim();
            let lead = if line.starts_with(['-', '*', '+', '#', '>'])
                || line.starts_with(|c: char| c.is_ascii_digit())
            {
                "\\"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "  {lead}{}",
                line.replace('<', "\\<").replace('`', "\\`")
            );
        }
    }
    Some((out, block))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slack_link_codes_are_signed_bound_and_expire() {
        let now = Timestamp::from_second(1_800_000_000)
            .unwrap_or(Timestamp::UNIX_EPOCH);
        let code = slack_link_code(b"secret", "T01", "U01", now);
        assert_eq!(
            read_slack_link_code(b"secret", &code, now),
            Ok(("T01".to_owned(), "U01".to_owned()))
        );
        let forged = code.replacen("U01", "U02", 1);
        assert!(
            read_slack_link_code(b"secret", &forged, now).is_err(),
            "bound to the user"
        );
        assert!(
            read_slack_link_code(b"other", &code, now).is_err(),
            "bound to the secret"
        );
        let later = now
            .checked_add(SLACK_LINK_TTL + jiff::SignedDuration::from_secs(1));
        assert!(
            read_slack_link_code(b"secret", &code, later.unwrap_or(now))
                .is_err(),
            "expires"
        );
        for junk in [
            "",
            "a.b.c",
            "T01.U01.x.y",
            "T 1.U01.1.00000000000000000000000000000000",
        ] {
            assert!(
                read_slack_link_code(b"secret", junk, now).is_err(),
                "{junk}"
            );
        }
    }
    use jiff::civil::date;

    fn now() -> DateTime {
        date(2026, 11, 2).at(9, 0, 0, 0)
    }

    #[test]
    fn addresses() {
        let token = ShareToken::generate().unwrap();
        let addr = address(&token, "northstar.dev");
        assert!(addr.starts_with("tasks+") && addr.ends_with("@northstar.dev"));
        let back =
            token_from_address(&format!("\"My tasks\" <{addr}>")).unwrap();
        assert!(token.hash().verify(&back));
        assert_eq!(
            token_from_address("ann@northstar.dev"),
            Err(CaptureError::Address)
        );
        assert_eq!(
            token_from_address("tasks+short@x"),
            Err(CaptureError::Address)
        );
    }

    #[test]
    fn email_becomes_an_inbox_task() {
        let raw = b"From: Ann <ann@northstar.dev>\r\n\
To: tasks+abc@northstar.dev\r\n\
Subject: Fwd: Re: Pay the <Northstar> invoice friday p1\r\n\
Message-ID: <m1@northstar.dev>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\r\n\
Hi,\r\n- line item {x=1}\r\nThanks\r\n";
        let capture = from_email(raw, now()).unwrap();
        assert_eq!(capture.title, "Pay the <Northstar> invoice");
        assert_eq!(capture.fields.due.unwrap().to_string(), "2026-11-06");
        assert_eq!(
            capture.fields.priority.map(crate::model::Priority::level),
            Some(1)
        );
        assert!(capture.fields.src.as_deref().unwrap().starts_with("email:"));
        let inbox = "---\ntitle: Inbox\n---\n";
        let (out, block) = append_to_inbox(inbox, &capture).unwrap();
        assert!(out.contains(&format!(
            "- [ ] Pay the \\<Northstar> invoice {{due=2026-11-06 p=1 src={}}} \
                ^{block}\n",
            capture.fields.src.as_deref().unwrap()
        )));
        assert!(out.contains(
            "\n  From: ann@northstar.dev\n  Hi,\n  \\- line item \
                {x=1}\n  Thanks\n"
        ));
        let list = checklist::Checklist::parse(&out);
        assert_eq!(list.items.len(), 1);
        assert_eq!(list.items[0].fields.due, capture.fields.due);
        // A retried delivery is not added twice.
        assert_eq!(append_to_inbox(&out, &capture), None);
    }

    #[test]
    fn slack_signature() {
        // Random secrets, so no key is written into the source.
        let token = ShareToken::generate().unwrap();
        let secret = token.expose().as_bytes();
        let other = ShareToken::generate().unwrap();
        let body = b"payload=%7B%7D";
        let now: Timestamp = "2026-11-02T09:00:00Z".parse().unwrap();
        let ts = now.as_second().to_string();
        let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
        mac.update(format!("v0:{ts}:").as_bytes());
        mac.update(body);
        let sig = format!(
            "v0={}",
            mac.finalize().into_bytes().iter().fold(
                String::new(),
                |mut s, b| {
                    let _ = write!(s, "{b:02x}");
                    s
                }
            )
        );
        assert_eq!(verify_slack(secret, &ts, &sig, body, now), Ok(()));
        assert_eq!(
            verify_slack(other.expose().as_bytes(), &ts, &sig, body, now),
            Err(CaptureError::Signature)
        );
        assert_eq!(
            verify_slack(secret, &ts, &sig, b"payload=x", now),
            Err(CaptureError::Signature)
        );
        let later =
            now.checked_add(jiff::SignedDuration::from_mins(6)).unwrap();
        assert_eq!(
            verify_slack(secret, &ts, &sig, body, later),
            Err(CaptureError::Signature)
        );
        assert_eq!(
            verify_slack(secret, &ts, "v1=00", body, now),
            Err(CaptureError::Signature)
        );
    }

    #[test]
    fn slack_shortcut_becomes_a_task() {
        let json = r#"{"type":"message_action","callback_id":"save_as_task","team":{"id":"T1","domain":"northstar"},
"channel":{"id":"C9","name":"launch"},"user":{"id":"U7","username":"ann"},
"message":{"ts":"1730538000.000200","text":"<@U2> can you review <https://northstar.dev/prd|the PRD> &amp; reply?\nthanks"}}"#;
        let encode = |json: &str| {
            format!(
                "payload={}",
                json.bytes().fold(String::new(), |mut s, b| {
                    if b.is_ascii_alphanumeric() {
                        s.push(char::from(b));
                    } else {
                        let _ = write!(s, "%{b:02X}");
                    }
                    s
                })
            )
        };
        let body = encode(json);
        let slack = from_slack(&body).unwrap();
        assert_eq!(slack.slack_user, "U7");
        assert_eq!(
            slack.capture.title,
            "@U2 can you review the PRD (https://northstar.dev/prd) & reply?"
        );
        assert!(slack.capture.notes.as_deref().unwrap().ends_with(
            "https://northstar.slack.com/archives/C9/p1730538000000200"
        ));
        assert_eq!(
            slack.capture.fields.src.as_deref(),
            Some("slack:T1:C9:1730538000.000200")
        );
        let other = encode(&json.replace("message_action", "block_actions"));
        assert!(matches!(from_slack(&other), Err(CaptureError::Payload(_))));
        assert!(from_slack("nothing=1").is_err());
    }
}
