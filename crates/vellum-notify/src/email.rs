// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Notification email: building RFC 5322 messages and the mail port.
//!
//! Messages are `multipart/alternative` (plain text and HTML), base64
//! encoded, with RFC 2047 encoded subjects. Emails about one subject thread
//! together through `In-Reply-To` and `References` pointing at a stable
//! per-subject message ID, and every message carries one-click unsubscribe
//! headers (RFC 8058). Replies go to the intake address, so answering an
//! email adds a comment (see the integrations crate).

use std::fmt::Write as _;
use std::future::Future;

use jiff::Timestamp;

use crate::encoding::b64;

/// A name and address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mailbox {
    /// Display name; may be empty.
    pub name: String,
    /// The address.
    pub address: String,
}

/// A notification email before encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmailMessage {
    /// The sender, normally `Vellum <notifications@…>`.
    pub from: Mailbox,
    /// The recipient.
    pub to: Mailbox,
    /// Where replies go: the comment intake address for the subject.
    pub reply_to: Option<String>,
    /// Subject line; any language.
    pub subject: String,
    /// Plain-text body.
    pub text: String,
    /// Sanitised HTML body from the renderer.
    pub html: String,
    /// The thing it is about (`ENG-142`), for threading.
    pub thread: String,
    /// Unique ID of this message, without angle brackets.
    pub message_id: String,
    /// One-click unsubscribe URL for this subject and person.
    pub unsubscribe_url: String,
    /// The Date header.
    pub date: Timestamp,
}

/// Why a message could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EmailError {
    /// A header value contained a line break or an address is malformed.
    #[error("invalid header value in `{0}`")]
    InvalidHeader(&'static str),
}

/// Sends encoded messages (SMTP or a provider API, in the edge layer).
pub trait MailTransport: Send + Sync {
    /// Sends one message to `to`.
    fn send(
        &self,
        to: &str,
        message: &[u8],
    ) -> impl Future<Output = Result<(), String>> + Send;
}

fn clean<'a>(
    value: &'a str,
    field: &'static str,
) -> Result<&'a str, EmailError> {
    if value.contains(['\r', '\n']) {
        Err(EmailError::InvalidHeader(field))
    } else {
        Ok(value)
    }
}

fn address<'a>(
    value: &'a str,
    field: &'static str,
) -> Result<&'a str, EmailError> {
    let value = clean(value, field)?;
    let valid = value.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && domain.contains('.')
            && !value.contains([' ', '<', '>', ',', '"'])
    });
    if valid {
        Ok(value)
    } else {
        Err(EmailError::InvalidHeader(field))
    }
}

/// RFC 2047 `B` encoding when the text is not plain printable ASCII.
fn encode_word(text: &str) -> String {
    if text.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        text.to_owned()
    } else {
        format!("=?UTF-8?B?{}?=", b64(text.as_bytes()))
    }
}

fn mailbox(
    mailbox: &Mailbox,
    field: &'static str,
) -> Result<String, EmailError> {
    let addr = address(&mailbox.address, field)?;
    let name = clean(&mailbox.name, field)?;
    if name.is_empty() {
        return Ok(format!("<{addr}>"));
    }
    let display = if name.is_ascii() {
        format!("\"{}\"", name.replace(['"', '\\'], ""))
    } else {
        encode_word(name)
    };
    Ok(format!("{display} <{addr}>"))
}

fn wrapped_base64(bytes: &[u8]) -> String {
    let encoded = b64(bytes);
    let mut out =
        String::with_capacity(encoded.len() + encoded.len() / 76 * 2 + 2);
    let mut rest = encoded.as_str();
    while !rest.is_empty() {
        let (line, tail) = rest.split_at(rest.len().min(76));
        out.push_str(line);
        out.push_str("\r\n");
        rest = tail;
    }
    out
}

impl EmailMessage {
    /// Encodes the message as RFC 5322 bytes with CRLF line endings.
    ///
    /// # Errors
    ///
    /// [`EmailError::InvalidHeader`] if any header would be malformed or
    /// carry an injected line.
    pub fn encode(&self) -> Result<Vec<u8>, EmailError> {
        let domain = address(&self.from.address, "from")?
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_owned())
            .unwrap_or_default();
        let thread = clean(&self.thread, "thread")?;
        let message_id = clean(&self.message_id, "message_id")?;
        let root = format!("<thread.{thread}@{domain}>");
        let boundary =
            format!("vellum-{message_id}").replace(['@', '<', '>'], "-");
        let mut headers = vec![
            format!("From: {}", mailbox(&self.from, "from")?),
            format!("To: {}", mailbox(&self.to, "to")?),
            format!(
                "Subject: {}",
                encode_word(clean(&self.subject, "subject")?)
            ),
            format!(
                "Date: {}",
                jiff::fmt::rfc2822::to_string(
                    &self.date.to_zoned(jiff::tz::TimeZone::UTC)
                )
                .unwrap_or_default()
            ),
            format!("Message-ID: <{message_id}>"),
            format!("In-Reply-To: {root}"),
            format!("References: {root}"),
            format!(
                "List-Unsubscribe: <{}>",
                clean(&self.unsubscribe_url, "unsubscribe_url")?
            ),
            "List-Unsubscribe-Post: List-Unsubscribe=One-Click".to_owned(),
            "Auto-Submitted: auto-generated".to_owned(),
            "MIME-Version: 1.0".to_owned(),
            format!(
                "Content-Type: multipart/alternative; boundary=\"{boundary}\""
            ),
        ];
        if let Some(reply_to) = &self.reply_to {
            headers.insert(
                2,
                format!("Reply-To: <{}>", address(reply_to, "reply_to")?),
            );
        }
        let mut out = headers.join("\r\n");
        out.push_str("\r\n\r\n");
        for (mime, body) in
            [("text/plain", &self.text), ("text/html", &self.html)]
        {
            let _ = write!(
                out,
                "--{boundary}\r\nContent-Type: {mime}; charset=utf-8\r\n\
                    Content-Transfer-Encoding: base64\r\n\r\n"
            );
            out.push_str(&wrapped_base64(body.as_bytes()));
        }
        let _ = write!(out, "--{boundary}--\r\n");
        Ok(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> EmailMessage {
        EmailMessage {
            from: Mailbox {
                name: "Vellum".to_owned(),
                address: "notifications@northstar.dev".to_owned(),
            },
            to: Mailbox {
                name: "Zoë Park".to_owned(),
                address: "zoe@northstar.dev".to_owned(),
            },
            reply_to: Some("reply+ENG-142.abc@northstar.dev".to_owned()),
            subject: "ENG-142 assigned to you".to_owned(),
            text: "Fix login redirect".repeat(10),
            html: "<p>Fix login redirect</p>".to_owned(),
            thread: "ENG-142".to_owned(),
            message_id: "01JD3Z6Q8W@northstar.dev".to_owned(),
            unsubscribe_url: "https://vellum.northstar.dev/u/abc".to_owned(),
            date: Timestamp::UNIX_EPOCH,
        }
    }

    #[test]
    fn encodes_a_threaded_multipart_message() -> Result<(), EmailError> {
        let bytes = message().encode()?;
        let text = String::from_utf8(bytes).unwrap_or_default();
        assert!(
            text.contains("From: \"Vellum\" <notifications@northstar.dev>\r\n")
        );
        assert!(
            text.contains(
                "To: =?UTF-8?B?Wm/DqyBQYXJr?= <zoe@northstar.dev>\r\n"
            )
        );
        assert!(
            text.contains("Reply-To: <reply+ENG-142.abc@northstar.dev>\r\n")
        );
        assert!(text.contains("Date: Thu, 1 Jan 1970 00:00:00 +0000\r\n"));
        assert!(
            text.contains("In-Reply-To: <thread.ENG-142@northstar.dev>\r\n")
        );
        assert!(
            text.contains(
                "List-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n"
            )
        );
        assert!(text.contains("Content-Type: text/html; charset=utf-8"));
        assert!(text.ends_with("--\r\n"));
        assert!(text.lines().all(|line| line.len() <= 998));
        Ok(())
    }

    #[test]
    fn refuses_header_injection() {
        let mut bad = message();
        bad.subject = "hi\r\nBcc: everyone@northstar.dev".to_owned();
        assert_eq!(bad.encode(), Err(EmailError::InvalidHeader("subject")));
        let mut bad = message();
        bad.to.address = "not an address".to_owned();
        assert_eq!(bad.encode(), Err(EmailError::InvalidHeader("to")));
    }

    #[test]
    fn non_ascii_subjects_are_encoded() {
        assert_eq!(encode_word("plain"), "plain");
        assert_eq!(encode_word("Café"), "=?UTF-8?B?Q2Fmw6k=?=");
    }
}
