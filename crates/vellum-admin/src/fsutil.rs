// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Small file helpers shared by the adapters.

use std::path::Path;

use tokio::io::AsyncWriteExt as _;
use vellum_kernel::{Error, ErrorKind, Result};

fn io(what: &'static str) -> impl FnOnce(std::io::Error) -> Error {
    move |e| Error::new(ErrorKind::Unavailable, what).with_source(e)
}

/// Writes `bytes` to `path` atomically: a synced temporary file in the
/// same directory is renamed over the target, so readers see the old or
/// the new content, never a mix.
///
/// # Errors
///
/// Returns an unavailable error when the file cannot be written.
pub async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| Error::new(ErrorKind::Internal, "invalid file name"))?;
    let tmp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .map_err(io("cannot create a temporary file"))?;
    file.write_all(bytes)
        .await
        .map_err(io("cannot write a temporary file"))?;
    file.sync_all()
        .await
        .map_err(io("cannot sync a temporary file"))?;
    drop(file);
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(io("cannot replace the file"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn replaces_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        write_atomic(&path, b"one").await.unwrap();
        write_atomic(&path, b"two").await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
        let err = write_atomic(Path::new("/"), b"x").await.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Internal);
        let missing = dir.path().join("no/such/dir/x");
        assert_eq!(
            write_atomic(&missing, b"x").await.unwrap_err().kind(),
            ErrorKind::Unavailable
        );
    }
}
