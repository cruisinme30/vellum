// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Envelope encryption at rest for Vellum (brief §11).
//!
//! Every stored object is sealed with its own random 256-bit data-encryption
//! key (DEK) using XChaCha20-Poly1305. The DEK is wrapped by a key-encryption
//! key (KEK) held by a [`KeyProvider`]: a [`LocalKeyring`] fed from the OS
//! keychain or an Argon2id passphrase ([`PassphraseKdf`]), or a remote KMS or Vault adapter.
//! Rotating the KEK re-wraps DEKs ([`envelope::rewrap`]) without re-encrypting
//! payloads, and dropping one object's DEK shreds that object.

pub mod aead;
pub mod envelope;
mod error;
mod key;
mod keychain;
mod passphrase;
mod provider;

pub use aead::{NONCE_LEN, TAG_LEN};
pub use envelope::{is_envelope, kek_of, open, plaintext_len, rewrap, seal};
pub use error::CryptoError;
pub use key::{KEY_LEN, KekId, MAX_KEK_ID_LEN, SecretKey};
#[cfg(feature = "os-keychain")]
pub use keychain::OsKeychain;
pub use keychain::{SecretStore, keyring_from_store, load_or_create_kek, rotate_in_store};
pub use passphrase::PassphraseKdf;
pub use provider::{KeyProvider, LocalKeyring, WrappedDek};
