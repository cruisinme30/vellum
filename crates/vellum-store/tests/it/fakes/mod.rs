// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! In-process fakes of the cloud storage APIs, one per adapter.

pub mod dropbox;
pub mod gdrive;
#[cfg(feature = "onedrive")]
pub mod onedrive;
