// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Which objects are stored encrypted and which in plaintext.

use std::collections::BTreeSet;
use std::fmt;

use vellum_store::BlobPath;

/// How an object is stored at rest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Sealed in an envelope. The default everywhere.
    #[default]
    Encrypted,
    /// Stored as is, for spaces whose files must stay readable on disk.
    Plaintext,
}

/// Decides the at-rest [`Mode`] for a path. The pages service implements it
/// from each space's settings; [`PrefixPolicy`] is the simple form.
pub trait ModePolicy: Send + Sync + fmt::Debug {
    /// The mode new writes to `path` use.
    fn mode_for(&self, path: &BlobPath) -> Mode;
}

/// Everything is encrypted except objects at or beneath the listed
/// plaintext directories.
#[derive(Clone, Debug, Default)]
pub struct PrefixPolicy {
    plaintext: BTreeSet<BlobPath>,
}

impl PrefixPolicy {
    /// A policy that encrypts everything.
    #[must_use]
    pub fn encrypt_all() -> Self {
        Self::default()
    }

    /// Marks `dir` (and everything beneath it) as plaintext, or back to
    /// encrypted when `plaintext` is false.
    pub fn set(&mut self, dir: BlobPath, plaintext: bool) {
        if plaintext {
            let _ = self.plaintext.insert(dir);
        } else {
            let _ = self.plaintext.remove(&dir);
        }
    }
}

impl ModePolicy for PrefixPolicy {
    fn mode_for(&self, path: &BlobPath) -> Mode {
        if self.plaintext.iter().any(|dir| path.starts_with(dir)) {
            Mode::Plaintext
        } else {
            Mode::Encrypted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_unless_under_a_plaintext_dir() -> Result<(), vellum_store::StoreError> {
        let mut policy = PrefixPolicy::encrypt_all();
        let docs = BlobPath::new("docs")?;
        policy.set(docs.clone(), true);
        assert_eq!(
            policy.mode_for(&BlobPath::new("docs/a.md")?),
            Mode::Plaintext
        );
        assert_eq!(
            policy.mode_for(&BlobPath::new("docsx/a.md")?),
            Mode::Encrypted
        );
        assert_eq!(
            policy.mode_for(&BlobPath::new("eng/a.md")?),
            Mode::Encrypted
        );
        policy.set(docs, false);
        assert_eq!(
            policy.mode_for(&BlobPath::new("docs/a.md")?),
            Mode::Encrypted
        );
        Ok(())
    }
}
