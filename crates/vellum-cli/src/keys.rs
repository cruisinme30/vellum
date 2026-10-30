// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Install keys kept in `<data_dir>/.vellum/keys`, created on first start.

use std::path::Path;

use vellum_kernel::{Error, ErrorKind};

/// Reads the key `name`, creating it (32 random bytes, owner-only) when it
/// is missing or too short.
///
/// # Errors
///
/// The folder or file cannot be created or read, or the random source
/// fails.
pub fn key_file(data_dir: &Path, name: &str) -> Result<Vec<u8>, Error> {
    let dir = data_dir.join(".vellum").join("keys");
    let path = dir.join(name);
    let io = |e: std::io::Error| {
        Error::new(
            ErrorKind::Unavailable,
            format!("cannot use {}", path.display()),
        )
        .with_source(e)
    };
    if let Some(existing) = std::fs::read(&path).ok().filter(|k| k.len() >= 32)
    {
        return Ok(existing);
    }
    std::fs::create_dir_all(&dir).map_err(io)?;
    let mut key = vec![0u8; 32];
    getrandom::fill(&mut key).map_err(|e| {
        Error::new(ErrorKind::Internal, "the random source failed")
            .with_source(e)
    })?;
    let mut options = std::fs::OpenOptions::new();
    let _ = options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        let _ = options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(io)?;
    std::io::Write::write_all(&mut file, &key).map_err(io)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_created_once() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let first = key_file(dir.path(), "test.key")?;
        assert_eq!(first.len(), 32);
        assert_eq!(key_file(dir.path(), "test.key")?, first);
        Ok(())
    }
}
