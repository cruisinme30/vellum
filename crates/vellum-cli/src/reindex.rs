// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum reindex`: rebuild the index from the workspace folder.

use std::io::Write as _;
use std::process::ExitCode;

use vellum_db::{Index, IndexConfig, IndexKey, PlainFiles, ShardedIndex, ShardsConfig};
use vellum_kernel::config::Config;

/// Environment variable holding the 64-hex-character index key, until the
/// keyring supplies it.
pub const KEY_VAR: &str = "VELLUM_INDEX_KEY";

/// Arguments of `vellum reindex`.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Open the SQLite index without encryption (plaintext mode only).
    #[arg(long)]
    plaintext: bool,
}

/// Runs the command and reports the outcome on standard output.
pub fn run(args: &Args) -> ExitCode {
    match execute(args) {
        Ok(lines) => {
            let mut out = std::io::stdout().lock();
            for line in lines {
                let _ = writeln!(out, "{line}");
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(%error, "reindex failed");
            let _ = writeln!(std::io::stderr(), "vellum reindex: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(args: &Args) -> Result<Vec<String>, String> {
    let config = Config::load(None).map_err(|e| e.to_string())?;
    let shards =
        ShardsConfig::from_env(|key| std::env::var(key).ok()).map_err(|e| e.to_string())?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let report = if let Some(shards) = shards {
        // Sharded deployment: each space goes to the shard the map names.
        runtime.block_on(async {
            let index = ShardedIndex::open(&shards).await?;
            let report = index.reindex(&config.data_dir, PlainFiles).await;
            index.close().await;
            report
        })
    } else {
        let index_config = if let Some(url) = &config.database_url {
            IndexConfig::postgres(url.clone())
        } else if args.plaintext {
            IndexConfig::sqlite(config.data_dir.join(".vellum/index.sqlite"), None)
        } else {
            // The server's key: the variable, or the one kept beside the data.
            let key = crate::workspace::sqlite_key(&config, |k| std::env::var(k).ok())
                .map_err(|e| e.to_string())?;
            IndexConfig::sqlite(
                config.data_dir.join(".vellum/index.sqlite"),
                key.map(IndexKey::new),
            )
        };
        runtime.block_on(async {
            let index = Index::open(&index_config).await?;
            let report = index.reindex(&config.data_dir, PlainFiles).await;
            index.close().await;
            report
        })
    }
    .map_err(|e| e.to_string())?;
    let mut lines = vec![format!(
        "Indexed {} spaces, {} folders and {} documents; skipped {} paths.",
        report.spaces,
        report.folders,
        report.documents,
        report.problems.len()
    )];
    lines.extend(report.problems.iter().map(|p| format!("  skipped {p}")));
    Ok(lines)
}

/// The raw bytes of a key written as 64 hex characters.
pub fn key_bytes(hex: Option<&str>) -> Result<[u8; 32], String> {
    let hex = hex.ok_or_else(|| format!("set {KEY_VAR} to the index key, or pass --plaintext"))?;
    parse_hex_key(KEY_VAR, hex)
}

/// Parses the 32-byte key in variable `var`, written as 64 hex characters.
pub fn parse_hex_key(var: &str, hex: &str) -> Result<[u8; 32], String> {
    let bad = || format!("{var} must be 64 hexadecimal characters");
    if hex.len() != 64 {
        return Err(bad());
    }
    let mut key = [0_u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        let text = std::str::from_utf8(pair).map_err(|_| bad())?;
        *byte = u8::from_str_radix(text, 16).map_err(|_| bad())?;
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::key_bytes;

    #[test]
    fn key_must_be_64_hex_characters() {
        assert!(key_bytes(Some(&"ab".repeat(32))).is_ok());
        assert!(key_bytes(Some("abc")).is_err());
        assert!(key_bytes(Some(&"zz".repeat(32))).is_err());
        assert!(key_bytes(None).unwrap_err().contains("--plaintext"));
    }
}
