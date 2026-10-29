// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Remote key-encryption-key providers for envelope encryption.
//!
//! Each type here implements [`vellum_crypto::KeyProvider`] by asking a key
//! service to wrap and unwrap data-encryption keys, so the KEK never leaves
//! that service:
//!
//! - [`AwsKms`]: AWS KMS `Encrypt`/`Decrypt`, signed with `SigV4`.
//! - [`GcpKms`]: Google Cloud KMS `encrypt`/`decrypt` with an `OAuth` token.
//! - [`VaultTransit`]: `HashiCorp` Vault's transit secrets engine.
//!
//! The KEK id names the remote key (`aws-kms:<region>:<key>`,
//! `gcp-kms:<key name>`, `vault:<mount>/<key>`), so an envelope written under
//! an old key still opens after the provider moves to a new one, and
//! [`vellum_crypto::rewrap`] rotates envelopes onto the new key.
//!
//! Every call carries `vellum/dek/v1` plus the KEK id as associated data, so
//! a wrapped key only opens under the KEK id its envelope names.

pub mod aws;
pub mod gcp;
mod http;
mod sigv4;
pub mod vault;

pub use aws::{AwsCredentials, AwsKms};
pub use gcp::{GcpKms, MetadataServerToken, StaticToken, TokenSource};
pub use vault::VaultTransit;
