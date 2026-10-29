// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Envelope encryption at rest for Vellum (brief §11).
//!
//! Every stored object is sealed with its own random 256-bit data-encryption
//! key (DEK) using XChaCha20-Poly1305. The DEK is wrapped by a key-encryption
//! key (KEK). This module holds the key types and the AEAD primitive.

pub mod aead;
mod error;
mod key;

pub use aead::{NONCE_LEN, TAG_LEN};
pub use error::CryptoError;
pub use key::{KEY_LEN, KekId, MAX_KEK_ID_LEN, SecretKey};
