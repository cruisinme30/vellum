// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! End-to-end encrypted private spaces on the server (ADR 0043).
//!
//! The server never holds a key that opens an E2E space. What it does hold
//! is the list of public keys a space's pages are sealed to, and the
//! ciphertext itself. This crate serves both to members' devices:
//!
//! - **Device enrolment** (D182, D183): each space's key list lives in the
//!   blob store at `.vellum/e2e/<space id>.yaml`, so every replica sees the
//!   same list and writes to it compare-and-swap. A member enrols their own
//!   device; the first key needs a manager, because it turns the space
//!   private. The rules (no weak keys, no duplicates, at most 255, never the
//!   last key) are `vellum_security::e2e::KeyRegistry`'s.
//! - **Ciphertext** (D184): members list a space's objects, download
//!   recipient envelopes, and upload pages their device sealed. An upload
//!   must be sealed to exactly the space's current keys, so nobody enrolled
//!   is left out and no revoked device is kept in. Nothing that is not
//!   ciphertext is served here, apart from the space's own folder manifest.
//!
//! - **Guard** (D208): [`Guard`] wraps the workspace's store and refuses
//!   any readable write into a private space, whichever service makes it.
//!
//! [`E2e`] is the application service over two ports: a [`BlobStore`]
//! and a [`SpaceAccess`] that the composition root builds from the drive's
//! spaces and permissions.
//!
//! [`BlobStore`]: vellum_store::BlobStore

mod api;
mod app;
mod guard;
mod keys;
mod ports;
mod service;

pub use api::{
    DeleteObject, Deleted, DeviceKeyInfo, EnrolDevice, GetObject, ListObjects,
    MAX_ENVELOPE_BYTES, MAX_LABEL_CHARS, ObjectInfo, ObjectList, PutObject,
    RevokeDevice, Revoked, SealedObject, SpaceKeys, SpaceRef, Stored,
};
pub use app::E2e;
pub use guard::{Guard, PrivateRoots, structural};
pub use keys::{KEYS_DIR, keys_path};
pub use ports::{Need, SpaceAccess};
