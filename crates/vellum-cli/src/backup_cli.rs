// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `vellum admin backup`, `vellum admin backups` and `vellum admin restore`:
//! the admin console's encrypted backups, from the command line (ADR 0021).
//!
//! `backup` runs the same audited backup as the console's "Back up now",
//! into the configured destination, and prunes old backups the same way.
//! `restore` writes a backup into an empty folder and checks every file
//! against its manifest; point a stopped server's data folder at it after.

use std::path::{Path, PathBuf};

use vellum_admin::backup;
use vellum_admin::service::Caller;
use vellum_kernel::{Config, Error, ErrorKind};

use crate::observability::{self, BACKUP_KEY_VAR, Observability};

/// Where a backup goes and which key restores it.
#[derive(Debug, clap::Args)]
pub struct RestoreArgs {
    /// The `.vbk` file to restore.
    pub file: PathBuf,
    /// The empty folder to restore into; it becomes a data folder.
    #[arg(long)]
    pub into: PathBuf,
    /// The configuration file, to find the backup key kept beside the
    /// data (`VELLUM_BACKUP_KEY` wins when set).
    #[arg(long)]
    pub config: Option<PathBuf>,
}

/// Takes a backup now and reports it.
///
/// # Errors
///
/// An unreadable configuration or data folder, a bad key or a failed write.
pub async fn backup_now(config: Option<&Path>) -> Result<Vec<String>, Error> {
    let config = load(config)?;
    let (service, _) =
        observability::open(&config, &Observability::new(), |k| std::env::var(k).ok()).await?;
    let info = service.backup_now(&Caller::system()).await?;
    Ok(vec![format!(
        "Backup {} written: {} files, {} bytes ({} on disk) in {} ms",
        info.name, info.files, info.bytes, info.size, info.duration_ms
    )])
}

/// Lists the backups in the configured destination, oldest first.
///
/// # Errors
///
/// An unreadable configuration, settings or backup folder.
pub async fn list(config: Option<&Path>) -> Result<Vec<String>, Error> {
    let config = load(config)?;
    let (service, _) =
        observability::open(&config, &Observability::new(), |k| std::env::var(k).ok()).await?;
    let files = service.backups().await?;
    if files.is_empty() {
        return Ok(vec!["No backups yet.".to_owned()]);
    }
    Ok(files
        .iter()
        .map(|f| format!("{}  {} bytes", f.name, f.size))
        .collect())
}

/// Restores a backup into an empty folder.
///
/// # Errors
///
/// No backup key, a folder that is not empty, or a damaged backup.
pub async fn restore(args: &RestoreArgs) -> Result<Vec<String>, Error> {
    let key = restore_key(args.config.as_deref(), |k| std::env::var(k).ok())?;
    let keys = observability::keyring("backup", &key)?;
    let report = backup::restore(&keys, &args.file, &args.into).await?;
    Ok(vec![format!(
        "Restored {} files ({} bytes) into {} in {} ms; every file matches the manifest.",
        report.files,
        report.bytes,
        args.into.display(),
        report.duration_ms
    )])
}

fn load(config: Option<&Path>) -> Result<Config, Error> {
    Config::load(config).map_err(|e| {
        Error::new(ErrorKind::InvalidInput, "cannot read the configuration").with_source(e)
    })
}

/// The backup key: `VELLUM_BACKUP_KEY`, else the one kept beside the
/// configured data. Unlike the server, a restore never makes a new key,
/// since a new key cannot open an old backup.
fn restore_key(
    config: Option<&Path>,
    var: impl Fn(&str) -> Option<String>,
) -> Result<Vec<u8>, Error> {
    if let Some(hex) = var(BACKUP_KEY_VAR) {
        return crate::reindex::parse_hex_key(BACKUP_KEY_VAR, &hex)
            .map(|k| k.to_vec())
            .map_err(|why| Error::new(ErrorKind::InvalidInput, why));
    }
    let data_dir = load(config)?.data_dir;
    let path = data_dir.join(".vellum").join("keys").join("backup.key");
    std::fs::read(&path)
        .ok()
        .filter(|k| k.len() >= 32)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidInput,
                format!(
                    "no backup key: set {BACKUP_KEY_VAR}, or keep the key at {}",
                    path.display()
                ),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration file whose data folder is `dir`.
    fn config_in(dir: &Path) -> PathBuf {
        let config = dir.join("vellum.toml");
        std::fs::write(
            &config,
            format!("data_dir = {:?}\n", dir.display().to_string()),
        )
        .unwrap();
        config
    }

    #[test]
    fn a_restore_never_invents_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_in(dir.path());
        let error = restore_key(Some(&config), |_| None).unwrap_err();
        assert!(error.to_string().contains(BACKUP_KEY_VAR), "{error}");
        assert!(!dir.path().join(".vellum/keys/backup.key").exists());

        let hex = "ab".repeat(32);
        let key = restore_key(Some(&config), |k| {
            (k == BACKUP_KEY_VAR).then(|| hex.clone())
        });
        assert_eq!(key.unwrap(), vec![0xab; 32]);
        assert!(restore_key(Some(&config), |_| Some("zz".to_owned())).is_err());
    }

    #[tokio::test]
    async fn a_backup_restores_into_an_empty_folder() {
        let data = tempfile::tempdir().unwrap();
        let config = config_in(data.path());
        std::fs::write(data.path().join("page.md"), "# Northstar launch\n").unwrap();
        let keys_dir = data.path().join(".vellum").join("keys");
        std::fs::create_dir_all(&keys_dir).unwrap();
        std::fs::write(keys_dir.join("backup.key"), [7; 32]).unwrap();
        let keys = observability::keyring("backup", &[7; 32]).unwrap();
        let out = tempfile::tempdir().unwrap();
        let info = backup::create(&keys, data.path(), out.path(), &[])
            .await
            .unwrap();

        let target = tempfile::tempdir().unwrap();
        let restored = target.path().join("restored");
        let lines = restore(&RestoreArgs {
            file: out.path().join(&info.name),
            into: restored.clone(),
            config: Some(config),
        })
        .await
        .unwrap();
        assert!(lines[0].contains("matches the manifest"), "{lines:?}");
        assert_eq!(
            std::fs::read_to_string(restored.join("page.md")).unwrap(),
            "# Northstar launch\n"
        );
        // The backup never carries the key that opens it.
        assert!(!restored.join(".vellum/keys/backup.key").exists());
    }
}
