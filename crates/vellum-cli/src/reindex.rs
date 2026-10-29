// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum reindex`: rebuild the index from the workspace files: the data
//! folder, or the bucket `VELLUM_BLOB_URL` names (ADR 0044, D187).
//!
//! A bucket is listed once; the pages and manifests the index reads are
//! fetched a few at a time, then scanned exactly like a folder.

use std::io::Write as _;
use std::process::ExitCode;

use futures::{StreamExt as _, TryStreamExt as _};
use vellum_db::{Index, IndexConfig, IndexKey, Listing, PlainFiles, ShardedIndex, ShardsConfig};
use vellum_kernel::config::Config;
use vellum_store::{BlobStore, StoreError, remote::s3::S3Store};

/// Files fetched from a bucket at once.
const FETCH_CONCURRENCY: usize = 16;

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
    let bucket = crate::workspace::bucket(|k| std::env::var(k).ok())
        .map_err(|e| e.to_string())?
        .map(S3Store::new)
        .transpose()
        .map_err(|e| format!("cannot use the blob store: {e}"))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let listing = match &bucket {
        Some(store) => {
            let listing = runtime
                .block_on(fetch_listing(store))
                .map_err(|e| format!("cannot read the bucket: {e}"))?;
            tracing::info!(files = listing.len(), "listed the bucket");
            Some(listing)
        }
        None => None,
    };
    let report = if let Some(shards) = shards {
        // Sharded deployment: each space goes to the shard the map names.
        runtime.block_on(async {
            let index = ShardedIndex::open(&shards).await?;
            let report = match listing {
                Some(listing) => index.reindex_listing(listing).await,
                None => index.reindex(&config.data_dir, PlainFiles).await,
            };
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
            let report = match listing {
                Some(listing) => index.reindex_listing(listing).await,
                None => index.reindex(&config.data_dir, PlainFiles).await,
            };
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

/// Lists `store` and fetches the files the index reads, a few at a time.
/// A file deleted between the listing and its fetch is left out.
///
/// # Errors
///
/// The listing or a fetch fails (after the store's own retries).
pub async fn fetch_listing(store: &dyn BlobStore) -> Result<Listing, StoreError> {
    let entries: Vec<_> = store.list(None).try_collect().await?;
    let mut fetched = futures::stream::iter(entries)
        .map(|entry| async move {
            if !Listing::wants_content(entry.path.as_str()) {
                return Ok(Some((entry, None)));
            }
            match store.get(&entry.path).await {
                Ok(blob) => Ok(Some((entry, Some(blob.bytes.to_vec())))),
                Err(StoreError::NotFound(_)) => Ok(None),
                Err(e) => Err(e),
            }
        })
        .buffer_unordered(FETCH_CONCURRENCY);
    let mut listing = Listing::new();
    while let Some(item) = fetched.try_next().await? {
        if let Some((entry, bytes)) = item {
            listing.insert(entry.path.as_str(), entry.modified, bytes);
        }
    }
    Ok(listing)
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
    use bytes::Bytes;
    use vellum_store::{BlobPath, BlobStore as _, LocalStore, Precondition};

    use super::{fetch_listing, key_bytes};

    #[tokio::test]
    async fn a_store_listing_fetches_only_pages_and_manifests() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore::open(dir.path()).await.unwrap();
        for (path, body) in [
            (
                "drives/shared/eng/.vellum-folder.yaml",
                "id: 01J0000000000000000000000A\n",
            ),
            ("drives/shared/eng/roadmap.md", "---\ntitle: Roadmap\n---\n"),
            ("drives/shared/eng/diagram.png", "png"),
        ] {
            let _ = store
                .put(
                    &BlobPath::new(path).unwrap(),
                    Bytes::from(body),
                    Precondition::None,
                )
                .await
                .unwrap();
        }
        let listing = fetch_listing(&store).await.unwrap();
        assert!(listing.len() >= 3);
        let read = |p: &str| vellum_db::FileReader::read(&listing, std::path::Path::new(p));
        assert_eq!(
            read("drives/shared/eng/roadmap.md").unwrap(),
            b"---\ntitle: Roadmap\n---\n"
        );
        assert!(read("drives/shared/eng/diagram.png").is_err());
    }

    #[test]
    fn key_must_be_64_hex_characters() {
        assert!(key_bytes(Some(&"ab".repeat(32))).is_ok());
        assert!(key_bytes(Some("abc")).is_err());
        assert!(key_bytes(Some(&"zz".repeat(32))).is_err());
        assert!(key_bytes(None).unwrap_err().contains("--plaintext"));
    }
}
