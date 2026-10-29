// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The private agent: end-to-end encrypted spaces, opened on the member's
//! own device (ADR 0036).
//!
//! In an E2E private space the server only ever holds ciphertext sealed to
//! the members' device keys, so it cannot search the space or hand it to an
//! AI tool. This crate does both on the device instead:
//!
//! - [`DeviceIdentity`]: the device's X25519 key pair, created once and kept
//!   in a [`vellum_crypto::SecretStore`] (the OS keychain in the
//!   desktop app and the CLI);
//! - [`read_document`]: turns an opened page into blocks with stable IDs,
//!   heading paths and plain text, ready to index and to quote;
//! - [`PrivateAgent`]: opens every page of the listed spaces that is sealed
//!   to this device and answers searches from an index that lives in memory
//!   only. Nothing it decrypts is written to disk.
//!
//! `vellum mcp --private` serves the agent to local AI clients, and
//! `vellum private key` prints the public key a space owner enrols.

mod agent;
mod extract;
mod identity;

pub use agent::{AgentError, PrivateAgent, PrivateSpace, Report};
pub use extract::{ExtractError, PrivateBlock, PrivateDoc, read_document};
pub use identity::{DEVICE_KEY_NAME, DeviceIdentity};
