// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The notifications and webhooks service.
//!
//! Work, docs and My Work publish changes; this service turns them into
//! deliveries on four channels:
//!
//! | Channel | Module | Transport port |
//! |---|---|---|
//! | Signed webhooks | [`webhook`], [`signature`], [`delivery`] | [`delivery::HttpTransport`] |
//! | In-app inbox | [`notification`] | stored by the caller |
//! | Email | [`email`], [`mail`] (queue, unsubscribe) | [`email::MailTransport`] |
//! | Web push | [`webpush`] | [`delivery::HttpTransport`] |
//!
//! [`service`] serves each person's push subscriptions and preferences
//! through the gateway, and [`hooks`] the admin's webhook subscriptions. The rest of the crate is pure domain logic plus
//! ports: it opens no sockets and reads no tables. Adapters in the edge layer supply the transports, and
//! every delivery runs inside a `tracing` span so it appears in the trace
//! of the write that caused it (ADR 0017).

pub mod delivery;
pub mod dispatch;
pub mod email;
pub mod endpoint;
pub mod event;
pub mod hooks;
pub mod inbox;
pub mod mail;
pub mod notification;
pub mod secret;
pub mod service;
pub mod signature;
pub mod webhook;
pub mod webpush;
pub mod work;

mod encoding;

pub use event::{EventAction, EventType, WebhookEvent};
pub use notification::{Channel, Notification, NotificationKind, Preferences};
pub use secret::WebhookSecret;
pub use signature::{SIGNATURE_HEADER, SignatureError};
pub use webhook::{Subscription, WebhookPayload};
