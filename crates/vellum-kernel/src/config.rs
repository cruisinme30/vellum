// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Server configuration.
//!
//! Values are layered: built-in defaults, then a TOML file, then
//! environment variables. Telemetry settings stay in [`crate::telemetry`].
//!
//! | File key | Variable | Default |
//! |---|---|---|
//! | — | `VELLUM_CONFIG` | unset (no file) |
//! | `data_dir` | `VELLUM_DATA_DIR` | `vellum-data` |
//! | `bind` | `VELLUM_BIND` | `127.0.0.1:7700` |
//! | `public_url` | `VELLUM_PUBLIC_URL` | unset (derived from `bind`) |
//! | `profile` | `VELLUM_PROFILE` | `single` |
//! | `database_url` | `VELLUM_DATABASE_URL` | unset (SQLite in `data_dir`) |

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The deployment profile, from the brief's deployment ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Profile {
    /// One binary, SQLite and a folder.
    #[default]
    Single,
    /// `docker compose` with Postgres, object storage and Kroki.
    Team,
    /// Helm: replicated services, Postgres replica, shards by space.
    Org,
}

/// Validated server configuration. `Debug` hides the database URL.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Config {
    /// Root folder holding `.vellum/`, drives, teams and tasks.
    pub data_dir: PathBuf,
    /// Address the HTTP server listens on.
    pub bind: SocketAddr,
    /// The external base URL, when behind a proxy or on a public host.
    pub public_url: Option<String>,
    /// The deployment profile.
    pub profile: Profile,
    /// Postgres URL; `None` means the bundled SQLite index.
    pub database_url: Option<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("data_dir", &self.data_dir)
            .field("bind", &self.bind)
            .field("public_url", &self.public_url)
            .field("profile", &self.profile)
            .field(
                "database_url",
                &self.database_url.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("vellum-data"),
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 7700)),
            public_url: None,
            profile: Profile::Single,
            database_url: None,
        }
    }
}

/// Why configuration could not be loaded.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("cannot read config file {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The I/O error.
        source: std::io::Error,
    },
    /// The configuration file is not valid TOML or has unknown keys.
    #[error("invalid config file: {0}")]
    Parse(#[from] toml::de::Error),
    /// A setting has an invalid value.
    #[error("invalid {key}: {reason}")]
    Invalid {
        /// The file key or variable name.
        key: &'static str,
        /// What is wrong.
        reason: String,
    },
}

/// The file layer: every key optional, unknown keys rejected.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileLayer {
    data_dir: Option<PathBuf>,
    bind: Option<SocketAddr>,
    public_url: Option<String>,
    profile: Option<Profile>,
    database_url: Option<String>,
}

impl Config {
    /// Loads from `path` (or `VELLUM_CONFIG`) and the process environment.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the file cannot be read or any value is invalid.
    pub fn load(path: Option<&Path>) -> Result<Self, ConfigError> {
        let from_env = std::env::var_os("VELLUM_CONFIG").map(PathBuf::from);
        let text = match path.map(Path::to_path_buf).or(from_env) {
            Some(path) => {
                let _span = tracing::info_span!("config.read", path = %path.display()).entered();
                Some(
                    std::fs::read_to_string(&path)
                        .map_err(|source| ConfigError::Read { path, source })?,
                )
            }
            None => None,
        };
        Self::from_sources(text.as_deref(), |key| std::env::var(key).ok())
    }

    /// Builds configuration from file text and a variable lookup, without I/O.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if the TOML or any value is invalid.
    pub fn from_sources(
        file: Option<&str>,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let file: FileLayer = file.map(toml::from_str).transpose()?.unwrap_or_default();
        let var = |key: &str| lookup(key).filter(|v| !v.trim().is_empty());
        let defaults = Self::default();
        let config = Self {
            data_dir: var("VELLUM_DATA_DIR")
                .map(PathBuf::from)
                .or(file.data_dir)
                .unwrap_or(defaults.data_dir),
            bind: match var("VELLUM_BIND") {
                Some(v) => v
                    .parse()
                    .map_err(|e: std::net::AddrParseError| invalid("VELLUM_BIND", e))?,
                None => file.bind.unwrap_or(defaults.bind),
            },
            public_url: var("VELLUM_PUBLIC_URL").or(file.public_url),
            profile: match var("VELLUM_PROFILE") {
                Some(v) => parse_profile(&v)?,
                None => file.profile.unwrap_or_default(),
            },
            database_url: var("VELLUM_DATABASE_URL").or(file.database_url),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if let Some(url) = &self.public_url
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(invalid("public_url", "must start with http:// or https://"));
        }
        if let Some(url) = &self.database_url
            && !(url.starts_with("postgres://") || url.starts_with("postgresql://"))
        {
            return Err(invalid("database_url", "must be a postgres:// URL"));
        }
        if self.profile != Profile::Single && self.database_url.is_none() {
            return Err(invalid(
                "database_url",
                "is required for the team and org profiles",
            ));
        }
        Ok(())
    }
}

fn parse_profile(value: &str) -> Result<Profile, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "single" => Ok(Profile::Single),
        "team" => Ok(Profile::Team),
        "org" => Ok(Profile::Org),
        other => Err(invalid(
            "VELLUM_PROFILE",
            format!("unknown profile {other:?}"),
        )),
    }
}

fn invalid(key: &'static str, reason: impl std::fmt::Display) -> ConfigError {
    ConfigError::Invalid {
        key,
        reason: reason.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(file: Option<&str>, vars: &[(&str, &str)]) -> Result<Config, ConfigError> {
        Config::from_sources(file, |key| {
            vars.iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn defaults_need_nothing() {
        assert_eq!(load(None, &[]).unwrap(), Config::default());
    }

    #[test]
    fn env_overrides_file() {
        let file = "data_dir = \"/srv/northstar\"\nbind = \"0.0.0.0:80\"\nprofile = \"team\"\n\
                    database_url = \"postgres://db/vellum\"\n";
        let c = load(
            Some(file),
            &[("VELLUM_BIND", "127.0.0.1:9000"), ("VELLUM_DATA_DIR", " ")],
        )
        .unwrap();
        assert_eq!(c.data_dir, PathBuf::from("/srv/northstar"));
        assert_eq!(c.bind, "127.0.0.1:9000".parse().unwrap());
        assert_eq!(c.profile, Profile::Team);
        assert!(!format!("{c:?}").contains("postgres://"));
    }

    #[test]
    fn rejects_bad_values() {
        assert!(matches!(
            load(Some("bnd = \"x\""), &[]),
            Err(ConfigError::Parse(_))
        ));
        assert!(matches!(
            load(None, &[("VELLUM_BIND", "nope")]),
            Err(ConfigError::Invalid {
                key: "VELLUM_BIND",
                ..
            })
        ));
        assert!(matches!(
            load(None, &[("VELLUM_PROFILE", "huge")]),
            Err(ConfigError::Invalid { .. })
        ));
        assert!(matches!(
            load(None, &[("VELLUM_PROFILE", "org")]),
            Err(ConfigError::Invalid {
                key: "database_url",
                ..
            })
        ));
        assert!(matches!(
            load(None, &[("VELLUM_PUBLIC_URL", "northstar.dev")]),
            Err(ConfigError::Invalid {
                key: "public_url",
                ..
            })
        ));
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let err = Config::load(Some(Path::new("/nonexistent/vellum.toml"))).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
