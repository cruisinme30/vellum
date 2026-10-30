// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Outgoing email: the SMTP transport, the people directory and the
//! one-click unsubscribe route (ADR 0041, D262).
//!
//! | Variable | Meaning |
//! |---|---|
//! | `VELLUM_SMTP_URL` | `smtps://user:pass@smtp.example.com` (TLS), `smtp://user:pass@host:587?tls=required` (STARTTLS), or `file:///path` to write `.eml` files instead (for trying it out) |
//! | `VELLUM_MAIL_FROM` | the sender, such as `Vellum <notifications@northstar.dev>`; defaults to `notifications@` the public host |
//!
//! Without `VELLUM_SMTP_URL` no email is sent: notifications, My Work
//! reminders and digests included, stay in the inbox and on push. The SMTP password is part of the URL, so treat the
//! variable as a secret.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use lettre::address::Envelope;
use lettre::{Address, AsyncSmtpTransport, AsyncTransport as _, Tokio1Executor};
use serde::Deserialize;
use url::Url;
use vellum_identity::Identity;
use vellum_kernel::{Error, ErrorKind, UserId};
use vellum_notify::email::{MailTransport, Mailbox};
use vellum_notify::mail::{self, Contact, Directory, MailConfig};
use vellum_notify::service::NotifyStore;

/// The SMTP server URL (a secret: it carries the password).
pub const SMTP_URL_VAR: &str = "VELLUM_SMTP_URL";
/// The sender address.
pub const MAIL_FROM_VAR: &str = "VELLUM_MAIL_FROM";

/// Where email goes.
#[derive(Clone)]
pub enum Outbound {
    /// An SMTP server.
    Smtp(AsyncSmtpTransport<Tokio1Executor>),
    /// `.eml` files in a folder.
    Files(PathBuf),
}

impl std::fmt::Debug for Outbound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // Never print the URL: it holds the password.
            Self::Smtp(_) => f.write_str("Outbound::Smtp"),
            Self::Files(dir) => f.debug_tuple("Outbound::Files").field(dir).finish(),
        }
    }
}

fn from_address(message: &[u8]) -> Option<Address> {
    // The envelope sender is the From address the message was built with.
    let head = std::str::from_utf8(message.get(..message.len().min(4096))?).ok()?;
    let line = head.lines().find(|l| l.starts_with("From: "))?;
    let start = line.rfind('<')?;
    let end = line.rfind('>')?;
    line.get(start + 1..end)?.parse().ok()
}

impl MailTransport for Outbound {
    async fn send(&self, to: &str, message: &[u8]) -> Result<(), String> {
        match self {
            Self::Smtp(smtp) => {
                let to: Address = to.parse().map_err(|e| format!("bad recipient: {e}"))?;
                let envelope = Envelope::new(from_address(message), vec![to])
                    .map_err(|e| format!("bad envelope: {e}"))?;
                smtp.send_raw(&envelope, message)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
            Self::Files(dir) => {
                let mut id = [0_u8; 8];
                let _ = getrandom::fill(&mut id);
                let id = u64::from_le_bytes(id);
                let name = format!("{}-{id:016x}.eml", jiff::Timestamp::now().as_second());
                tokio::fs::create_dir_all(dir)
                    .await
                    .map_err(|e| e.to_string())?;
                tokio::fs::write(dir.join(name), message)
                    .await
                    .map_err(|e| e.to_string())
            }
        }
    }
}

/// The transport and addressing from the environment, or `None` when no
/// mail server is configured.
///
/// # Errors
///
/// A malformed URL or sender.
pub fn setup(
    var: impl Fn(&str) -> Option<String>,
    public_url: &str,
) -> Result<Option<(Outbound, MailConfig)>, Error> {
    let Some(raw) = var(SMTP_URL_VAR).filter(|v| !v.trim().is_empty()) else {
        tracing::info!("no {SMTP_URL_VAR}; notification email is off");
        return Ok(None);
    };
    let bad = |what: &str| Error::new(ErrorKind::InvalidInput, format!("{SMTP_URL_VAR}: {what}"));
    let base_url = Url::parse(public_url)
        .map_err(|e| Error::new(ErrorKind::InvalidInput, format!("public URL: {e}")))?;
    let outbound = if let Some(path) = raw.strip_prefix("file://") {
        Outbound::Files(PathBuf::from(path))
    } else {
        let parsed = Url::parse(raw.trim()).map_err(|_| bad("not a URL"))?;
        if !matches!(parsed.scheme(), "smtp" | "smtps") {
            return Err(bad("use smtp://, smtps:// or file://"));
        }
        let smtp = AsyncSmtpTransport::<Tokio1Executor>::from_url(raw.trim())
            .map_err(|e| bad(&format!("{e}")))?
            .timeout(Some(std::time::Duration::from_secs(30)))
            .build();
        Outbound::Smtp(smtp)
    };
    let host = base_url.host_str().unwrap_or("localhost").to_owned();
    let from = match var(MAIL_FROM_VAR).filter(|v| !v.trim().is_empty()) {
        Some(text) => parse_mailbox(&text).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("{MAIL_FROM_VAR}: use `Name <address>` or `address`"),
            )
        })?,
        None => Mailbox {
            name: "Vellum".to_owned(),
            address: format!("notifications@{host}"),
        },
    };
    tracing::info!(
        sender = %from.address,
        transport = ?outbound,
        "notification email is on"
    );
    Ok(Some((
        outbound,
        MailConfig {
            from,
            base_url,
            reply_to: None,
        },
    )))
}

/// `Name <address>` or a bare address.
fn parse_mailbox(text: &str) -> Option<Mailbox> {
    let text = text.trim();
    let (name, address) = match (text.rfind('<'), text.strip_suffix('>')) {
        (Some(start), Some(_)) => (
            text.get(..start)?.trim().trim_matches('"').to_owned(),
            text.get(start + 1..text.len() - 1)?.trim().to_owned(),
        ),
        _ => (String::new(), text.to_owned()),
    };
    let _: Address = address.parse().ok()?;
    Some(Mailbox { name, address })
}

/// People, from sign-in.
pub struct People {
    identity: Arc<Identity>,
}

impl People {
    /// The directory over `identity`.
    pub const fn new(identity: Arc<Identity>) -> Self {
        Self { identity }
    }

    /// Everyone who can be emailed.
    pub fn everyone(&self) -> Vec<UserId> {
        self.identity
            .users()
            .into_iter()
            .filter(|u| !u.disabled)
            .map(|u| u.id)
            .collect()
    }
}

impl Directory for People {
    fn contact(&self, user: UserId) -> Option<Contact> {
        let view = self.identity.user(user).ok().filter(|u| !u.disabled)?;
        Some(Contact {
            user,
            mailbox: Mailbox {
                name: view.name.replace(['\r', '\n'], " "),
                address: view.email,
            },
            handle: view.handle,
        })
    }
}

#[derive(Debug, Deserialize)]
struct Link {
    u: String,
    s: String,
    t: String,
}

/// `/unsubscribe`, mounted cross-site: mail apps post to it with no
/// session (RFC 8058). `GET` shows a page with a button, so a link
/// scanner that follows the URL changes nothing; `POST` applies it.
pub fn router(store: Arc<NotifyStore>) -> Router {
    Router::new()
        .route("/", get(confirm).post(apply))
        .with_state(store)
}

fn user_of(link: &Link) -> Option<UserId> {
    link.u.parse::<uuid::Uuid>().ok().map(UserId::from_uuid)
}

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title}</title><style>body{{font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;line-height:1.5;color:#1d2330}}\
         button{{font:inherit;padding:.5rem 1rem;border-radius:6px;border:0;background:#2f5bd3;color:#fff;cursor:pointer}}</style></head>\
         <body><h1>{title}</h1>{body}</body></html>"
    );
    (
        status,
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Html(html),
    )
        .into_response()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn what(subject: &str) -> String {
    format!("emails about {}", escape(subject))
}

async fn confirm(State(store): State<Arc<NotifyStore>>, Query(link): Query<Link>) -> Response {
    let valid = user_of(&link).is_some_and(|u| store.check_unsubscribe(u, &link.s, &link.t));
    if !valid {
        return page(
            StatusCode::BAD_REQUEST,
            "Link not valid",
            "<p>This unsubscribe link is not valid. Change your email settings in Vellum instead.</p>",
        );
    }
    let action = format!(
        "?u={}&amp;s={}&amp;t={}",
        escape(&link.u),
        escape(&url::form_urlencoded::byte_serialize(link.s.as_bytes()).collect::<String>()),
        escape(&link.t)
    );
    page(
        StatusCode::OK,
        "Unsubscribe",
        &format!(
            "<p>Stop {}?</p><form method=\"post\" action=\"{action}\"><button type=\"submit\">Unsubscribe</button></form>",
            what(&link.s)
        ),
    )
}

async fn apply(State(store): State<Arc<NotifyStore>>, Query(link): Query<Link>) -> Response {
    let Some(user) = user_of(&link) else {
        return page(
            StatusCode::BAD_REQUEST,
            "Link not valid",
            "<p>This unsubscribe link is not valid.</p>",
        );
    };
    match mail::unsubscribe(&store, user, &link.s, &link.t) {
        Ok(()) => page(
            StatusCode::OK,
            "Unsubscribed",
            &format!(
                "<p>You will no longer get {}. You can turn it back on in your notification settings.</p>",
                what(&link.s)
            ),
        ),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => page(
            StatusCode::BAD_REQUEST,
            "Link not valid",
            "<p>This unsubscribe link is not valid.</p>",
        ),
        Err(error) => {
            tracing::warn!(%error, "unsubscribe failed");
            page(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Something went wrong",
                "<p>Try again, or change your email settings in Vellum.</p>",
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test fixtures fail loudly")]

    use super::*;

    #[test]
    fn senders_parse_with_or_without_a_name() {
        let m = parse_mailbox("Vellum <notifications@northstar.dev>").unwrap();
        assert_eq!(
            (m.name.as_str(), m.address.as_str()),
            ("Vellum", "notifications@northstar.dev")
        );
        let m = parse_mailbox("ops@northstar.dev").unwrap();
        assert_eq!(
            (m.name.as_str(), m.address.as_str()),
            ("", "ops@northstar.dev")
        );
        assert!(parse_mailbox("Vellum <not an address>").is_none());
    }

    #[test]
    fn setup_reads_the_environment() {
        let none = setup(|_| None, "https://vellum.northstar.dev").unwrap();
        assert!(none.is_none());
        let env = |k: &str| match k {
            SMTP_URL_VAR => Some("file:///tmp/vellum-mail".to_owned()),
            _ => None,
        };
        let (out, config) = setup(env, "https://vellum.northstar.dev").unwrap().unwrap();
        assert!(matches!(out, Outbound::Files(_)));
        assert_eq!(config.from.address, "notifications@vellum.northstar.dev");
        let bad = |k: &str| (k == SMTP_URL_VAR).then(|| "ftp://x".to_owned());
        assert!(setup(bad, "https://vellum.northstar.dev").is_err());
        let smtp =
            |k: &str| (k == SMTP_URL_VAR).then(|| "smtps://u:p@smtp.northstar.dev".to_owned());
        let (out, _) = setup(smtp, "https://vellum.northstar.dev")
            .unwrap()
            .unwrap();
        assert!(
            !format!("{out:?}").contains("u:p"),
            "the password is never printed"
        );
    }

    #[test]
    fn the_envelope_sender_comes_from_the_message() {
        let raw = b"From: \"Vellum\" <notifications@northstar.dev>\r\nTo: <a@b.dev>\r\n\r\nhi";
        assert_eq!(
            from_address(raw).unwrap().to_string(),
            "notifications@northstar.dev"
        );
    }
}
