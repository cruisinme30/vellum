// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Argon2id passphrase-derived KEKs.
//!
//! The KEK id carries the salt and cost parameters, so the same passphrase
//! re-derives the same KEK from nothing but an envelope's header:
//! `argon2id$m=<KiB>$t=<passes>$p=<lanes>$<32 hex salt>`.

use std::fmt::Write as _;

use argon2::{Algorithm, Argon2, Params, Version};
use zeroize::Zeroizing;

use crate::error::CryptoError;
use crate::key::{KEY_LEN, KekId, SecretKey};

const SALT_LEN: usize = 16;
const PREFIX: &str = "argon2id";
/// Caps that stop a crafted KEK id from requesting absurd memory or time.
const MAX_M_KIB: u32 = 1 << 20;
const MAX_T: u32 = 16;
const MAX_P: u32 = 16;

/// Argon2id parameters plus salt for one passphrase KEK.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassphraseKdf {
    m_kib: u32,
    t: u32,
    p: u32,
    salt: [u8; SALT_LEN],
}

impl PassphraseKdf {
    /// RFC 9106 second recommended option: 64 mebibytes, 3 passes, 4 lanes.
    pub const DEFAULT_M_KIB: u32 = 64 * 1024;
    /// Default number of passes.
    pub const DEFAULT_T: u32 = 3;
    /// Default number of lanes.
    pub const DEFAULT_P: u32 = 4;

    /// Default parameters with a fresh random salt.
    ///
    /// # Errors
    /// [`CryptoError::Rng`] if the system random source fails.
    pub fn generate() -> Result<Self, CryptoError> {
        Self::with_params(Self::DEFAULT_M_KIB, Self::DEFAULT_T, Self::DEFAULT_P)
    }

    /// Custom costs with a fresh random salt (low-memory hosts, tests).
    ///
    /// # Errors
    /// [`CryptoError::Kdf`] if the costs are outside Argon2's or our limits,
    /// [`CryptoError::Rng`] if the system random source fails.
    pub fn with_params(m_kib: u32, t: u32, p: u32) -> Result<Self, CryptoError> {
        let mut salt = [0_u8; SALT_LEN];
        getrandom::fill(&mut salt).map_err(|_| CryptoError::Rng)?;
        let kdf = Self { m_kib, t, p, salt };
        let _ = kdf.params()?;
        Ok(kdf)
    }

    /// Recovers the parameters from a KEK id written by [`Self::kek_id`].
    ///
    /// # Errors
    /// [`CryptoError::Kdf`] if the id is not a valid passphrase KEK id.
    pub fn from_kek_id(id: &KekId) -> Result<Self, CryptoError> {
        let mut parts = id.as_str().split('$');
        if parts.next() != Some(PREFIX) {
            return Err(CryptoError::Kdf);
        }
        let mut cost = |key: &str| -> Result<u32, CryptoError> {
            parts
                .next()
                .and_then(|part| part.strip_prefix(key))
                .and_then(|n| n.parse().ok())
                .ok_or(CryptoError::Kdf)
        };
        let (m_kib, t, p) = (cost("m=")?, cost("t=")?, cost("p=")?);
        let salt = parts.next().and_then(decode_hex).ok_or(CryptoError::Kdf)?;
        if parts.next().is_some() {
            return Err(CryptoError::Kdf);
        }
        let kdf = Self { m_kib, t, p, salt };
        let _ = kdf.params()?;
        Ok(kdf)
    }

    /// The Argon2id memory cost in `KiB`, so callers can refuse KEK ids that
    /// ask for more than they are willing to spend.
    #[must_use]
    pub const fn m_kib(&self) -> u32 {
        self.m_kib
    }

    /// The KEK id that names this salt and these parameters.
    ///
    /// # Errors
    /// Never in practice; the id is always valid printable ASCII.
    pub fn kek_id(&self) -> Result<KekId, CryptoError> {
        let salt = self.salt.iter().fold(String::new(), |mut hex, b| {
            let _ = write!(hex, "{b:02x}");
            hex
        });
        KekId::new(format!(
            "{PREFIX}$m={}$t={}$p={}${salt}",
            self.m_kib, self.t, self.p
        ))
    }

    /// Derives the KEK from `passphrase`. Deliberately slow.
    ///
    /// # Errors
    /// [`CryptoError::Kdf`] if Argon2 rejects the input.
    #[tracing::instrument(name = "crypto.kdf.argon2id", skip_all, fields(m_kib = self.m_kib, t = self.t, p = self.p))]
    pub fn derive(&self, passphrase: &[u8]) -> Result<SecretKey, CryptoError> {
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, self.params()?);
        let mut out = Zeroizing::new([0_u8; KEY_LEN]);
        argon
            .hash_password_into(passphrase, &self.salt, out.as_mut())
            .map_err(|_| CryptoError::Kdf)?;
        Ok(SecretKey::from_bytes(*out))
    }

    fn params(&self) -> Result<Params, CryptoError> {
        if self.m_kib > MAX_M_KIB || self.t > MAX_T || self.p > MAX_P {
            return Err(CryptoError::Kdf);
        }
        Params::new(self.m_kib, self.t, self.p, Some(KEY_LEN)).map_err(|_| CryptoError::Kdf)
    }
}

/// Environment variable holding the KEK passphrase itself.
pub const PASSPHRASE_ENV: &str = "VELLUM_KEK_PASSPHRASE";
/// Environment variable naming a file that holds the KEK passphrase, for
/// container secrets mounted as files.
pub const PASSPHRASE_FILE_ENV: &str = "VELLUM_KEK_PASSPHRASE_FILE";

/// Reads the KEK passphrase from [`PASSPHRASE_ENV`] or the file named by
/// [`PASSPHRASE_FILE_ENV`]. One trailing newline is dropped from the file.
/// Returns `None` when neither is set.
///
/// # Errors
/// [`CryptoError::Provider`] if both are set, the passphrase is empty, or the
/// file cannot be read. Messages name the variable, never its value.
pub fn passphrase_from_env() -> Result<Option<Zeroizing<Vec<u8>>>, CryptoError> {
    passphrase_from(|name| std::env::var_os(name), |path| std::fs::read(path))
}

/// [`passphrase_from_env`] with the environment and file system passed in.
fn passphrase_from(
    var: impl Fn(&str) -> Option<std::ffi::OsString>,
    read: impl Fn(&std::path::Path) -> std::io::Result<Vec<u8>>,
) -> Result<Option<Zeroizing<Vec<u8>>>, CryptoError> {
    let inline = var(PASSPHRASE_ENV);
    let file = var(PASSPHRASE_FILE_ENV);
    let secret = match (inline, file) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => {
            return Err(CryptoError::Provider(format!(
                "set only one of {PASSPHRASE_ENV} and {PASSPHRASE_FILE_ENV}"
            )));
        }
        (Some(value), None) => Zeroizing::new(value.into_encoded_bytes()),
        (None, Some(path)) => {
            let mut bytes = Zeroizing::new(read(std::path::Path::new(&path)).map_err(|e| {
                CryptoError::Provider(format!("cannot read {PASSPHRASE_FILE_ENV}: {e}"))
            })?);
            if bytes.ends_with(b"\n") {
                let _ = bytes.pop();
                if bytes.ends_with(b"\r") {
                    let _ = bytes.pop();
                }
            }
            bytes
        }
    };
    if secret.is_empty() {
        return Err(CryptoError::Provider("the KEK passphrase is empty".into()));
    }
    Ok(Some(secret))
}

fn decode_hex(text: &str) -> Option<[u8; SALT_LEN]> {
    if text.len() != SALT_LEN * 2 {
        return None;
    }
    let mut out = [0_u8; SALT_LEN];
    for (slot, pair) in out.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let pair = std::str::from_utf8(pair).ok()?;
        *slot = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(
        pairs: &'a [(&'a str, &'a str)],
    ) -> impl Fn(&str) -> Option<std::ffi::OsString> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.into())
        }
    }

    fn file(content: &'static [u8]) -> impl Fn(&std::path::Path) -> std::io::Result<Vec<u8>> {
        move |_| Ok(content.to_vec())
    }

    #[test]
    fn passphrase_comes_from_the_variable_or_the_file() -> Result<(), CryptoError> {
        let none = passphrase_from(env(&[]), file(b""))?;
        assert!(none.is_none());
        let inline = passphrase_from(env(&[(PASSPHRASE_ENV, "hunter2")]), file(b""))?;
        assert_eq!(inline.as_deref().map(Vec::as_slice), Some(&b"hunter2"[..]));
        let from_file = passphrase_from(
            env(&[(PASSPHRASE_FILE_ENV, "/run/secrets/kek")]),
            file(b"correct horse\r\n"),
        )?;
        assert_eq!(
            from_file.as_deref().map(Vec::as_slice),
            Some(&b"correct horse"[..])
        );
        Ok(())
    }

    #[test]
    fn passphrase_rejects_ambiguous_or_empty_input() {
        let both = passphrase_from(
            env(&[(PASSPHRASE_ENV, "a"), (PASSPHRASE_FILE_ENV, "/x")]),
            file(b"b"),
        );
        assert!(matches!(both, Err(CryptoError::Provider(_))));
        let empty = passphrase_from(env(&[(PASSPHRASE_FILE_ENV, "/x")]), file(b"\n"));
        assert!(matches!(empty, Err(CryptoError::Provider(_))));
        let missing = passphrase_from(env(&[(PASSPHRASE_FILE_ENV, "/x")]), |_| {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        });
        assert!(matches!(missing, Err(CryptoError::Provider(m)) if !m.contains("/x")));
    }

    fn cheap() -> Result<PassphraseKdf, CryptoError> {
        PassphraseKdf::with_params(64, 1, 1)
    }

    #[test]
    fn same_passphrase_same_kek_via_id() -> Result<(), CryptoError> {
        let kdf = cheap()?;
        let restored = PassphraseKdf::from_kek_id(&kdf.kek_id()?)?;
        assert_eq!(restored, kdf);
        let a = kdf.derive(b"correct horse")?;
        let b = restored.derive(b"correct horse")?;
        let c = kdf.derive(b"wrong horse")?;
        assert_eq!(a.expose(), b.expose());
        assert_ne!(a.expose(), c.expose());
        Ok(())
    }

    #[test]
    fn rejects_bad_ids_and_costs() -> Result<(), CryptoError> {
        for bad in [
            "kek-1",
            "argon2id$m=64$t=1$p=1",
            "argon2id$m=64$t=1$p=1$zz",
            "argon2id$m=64$t=1$p=1$00112233445566778899aabbccddeeff$x",
            "argon2id$m=99999999$t=1$p=1$00112233445566778899aabbccddeeff",
        ] {
            assert!(
                PassphraseKdf::from_kek_id(&KekId::new(bad)?).is_err(),
                "{bad}"
            );
        }
        assert!(PassphraseKdf::with_params(64, MAX_T + 1, 1).is_err());
        Ok(())
    }
}
