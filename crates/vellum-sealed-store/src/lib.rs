// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Encryption at rest for any [`vellum_store::BlobStore`] (brief §11).
//!
//! [`Sealed`] wraps a store and seals every object written to an encrypted
//! space with `vellum-crypto` envelopes. Encryption is on by default; a space
//! is stored in plaintext only when its [`ModePolicy`] says so (for example a
//! Git-visible space). Reads open envelopes wherever they are found, so a
//! space can switch mode and be migrated with [`Sealed::reseal`] while it
//! stays readable.

mod policy;
mod sealed;

pub use policy::{Mode, ModePolicy, PrefixPolicy};
pub use sealed::{PLAINTEXT_READS, Sealed};
