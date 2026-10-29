// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Rebuilds the crate when a migration file is added or edited, so
//! `sqlx::migrate!` never embeds a stale set in an incremental build.

fn main() {
    println!("cargo:rerun-if-changed=migrations");
}
