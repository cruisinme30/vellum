// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Repository automation, run as `cargo xtask <command>`.
//!
//! `cargo xtask boundaries` enforces the crate layering: every workspace
//! crate declares a layer in `[package.metadata.vellum]`, and may depend
//! only on Vellum crates in strictly lower layers. Crates in the same layer
//! (for example two services) can never depend on each other, so a service
//! cannot reach into another service's storage.

use std::process::ExitCode;

mod boundaries;

fn main() -> ExitCode {
    let command = std::env::args().nth(1);
    let result = match command.as_deref() {
        Some("boundaries") => boundaries::run(),
        _ => Err("usage: cargo xtask boundaries".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            #[expect(clippy::print_stderr, reason = "xtask reports to the terminal")]
            {
                eprintln!("{message}");
            }
            ExitCode::FAILURE
        }
    }
}
