// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Device keys this device has agreed to seal pages to (D210).
//!
//! The server hands out each private space's key list, and a device seals
//! pages to every key on it. A server that is not fully trusted could slip
//! its own key in. So each device remembers the fingerprints it has sealed
//! to before, per server and space, and refuses to seal to a new one until
//! the person has compared it with its owner and trusts it explicitly.
//! The first list a device sees is trusted as is (trust on first use).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use vellum_kernel::SpaceId;

/// Variable naming the folder that holds the pins.
pub const DIR_VAR: &str = "VELLUM_PRIVATE_DIR";

/// Pinned fingerprints, one small JSON file per server and space.
#[derive(Debug, Clone)]
pub struct Pins {
    dir: PathBuf,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PinFile {
    server: String,
    space: String,
    trusted: BTreeSet<String>,
}

impl Pins {
    /// Pins in `dir`.
    #[must_use]
    pub const fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Pins in `$VELLUM_PRIVATE_DIR`, or `~/.vellum/private`.
    ///
    /// # Errors
    ///
    /// Neither the variable nor a home folder is set.
    pub fn from_env() -> Result<Self, String> {
        if let Some(dir) = std::env::var_os(DIR_VAR).filter(|d| !d.is_empty()) {
            return Ok(Self::new(PathBuf::from(dir)));
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .filter(|h| !h.is_empty())
            .ok_or_else(|| format!("set {DIR_VAR} to a folder for trusted device keys"))?;
        Ok(Self::new(Path::new(&home).join(".vellum").join("private")))
    }

    fn file(&self, server: &str, space: SpaceId) -> PathBuf {
        let digest = Sha256::digest(format!("{}\n{space}", server.trim_end_matches('/')));
        let name = hex::encode(digest.get(..16).unwrap_or_default());
        self.dir.join(format!("{name}.json"))
    }

    /// The trusted fingerprints, or `None` before the first use.
    ///
    /// # Errors
    ///
    /// The file exists but cannot be read or parsed.
    pub fn load(&self, server: &str, space: SpaceId) -> Result<Option<BTreeSet<String>>, String> {
        let path = self.file(server, space);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice::<PinFile>(&bytes)
                .map(|f| Some(f.trusted))
                .map_err(|e| format!("{} is damaged: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Replaces the trusted fingerprints.
    ///
    /// # Errors
    ///
    /// The folder or file cannot be written.
    pub fn save(
        &self,
        server: &str,
        space: SpaceId,
        trusted: &BTreeSet<String>,
    ) -> Result<(), String> {
        let path = self.file(server, space);
        let io = |e: std::io::Error| format!("cannot write {}: {e}", path.display());
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let body = serde_json::to_vec_pretty(&PinFile {
            server: server.to_owned(),
            space: space.to_string(),
            trusted: trusted.clone(),
        })
        .map_err(|e| e.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        let _ = options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            let _ = options.mode(0o600);
        }
        let mut file = options.open(&path).map_err(io)?;
        std::io::Write::write_all(&mut file, &body).map_err(io)
    }
}
