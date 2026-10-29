// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Admin settings: typed, validated, versioned and audited.
//!
//! Settings changed in the admin console are stored as one JSON document
//! with a version number. A save names the version it was based on, so two
//! admins editing at once cannot silently overwrite each other; every
//! change is audited with the list of fields that changed.

use std::collections::BTreeMap;
use std::path::PathBuf;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use vellum_kernel::{Error, ErrorKind, Result};

/// Opt-in anonymous usage reporting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct TelemetrySettings {
    /// Whether reports are sent. Off until an admin turns it on.
    pub enabled: bool,
    /// Where reports go (HTTPS).
    pub endpoint: Option<String>,
    /// A random id for this installation, unrelated to anything else.
    pub install_id: String,
    /// Hours between reports.
    pub interval_hours: u32,
}

impl Default for TelemetrySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: None,
            install_id: new_install_id(),
            interval_hours: 168,
        }
    }
}

/// A new random installation id: 32 hex digits.
#[must_use]
pub fn new_install_id() -> String {
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        // Without a random source the id only needs to be unlikely to clash.
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        bytes = nanos.to_le_bytes();
    }
    hex::encode(bytes)
}

/// The built-in observability tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct ObservabilitySettings {
    /// Traces kept for Admin → Requests.
    pub trace_buffer_capacity: usize,
    /// Memory budget for those traces, in `MiB`.
    pub trace_buffer_mib: usize,
    /// Requests slower than this are highlighted, in milliseconds.
    pub slow_request_ms: u64,
    /// The p95 latency budget of every route (R-OB-4).
    pub budgets: vellum_observe::Budgets,
}

impl Default for ObservabilitySettings {
    fn default() -> Self {
        Self {
            trace_buffer_capacity: vellum_observe::store::DEFAULT_CAPACITY,
            trace_buffer_mib: vellum_observe::store::DEFAULT_BYTE_BUDGET / (1024 * 1024),
            slow_request_ms: 1000,
            budgets: vellum_observe::Budgets::default(),
        }
    }
}

/// Scheduled encrypted backups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct BackupSettings {
    /// Whether scheduled backups run.
    pub enabled: bool,
    /// Hours between backups.
    pub interval_hours: u32,
    /// Backups kept; older ones are deleted.
    pub keep: u32,
    /// Directory backups are written to; relative paths are under the
    /// data directory.
    pub destination: String,
    /// Days between automatic restore drills (brief §11: monthly).
    pub drill_interval_days: u32,
}

impl Default for BackupSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_hours: 24,
            keep: 14,
            destination: "backups".to_owned(),
            drill_interval_days: 30,
        }
    }
}

/// Admin-session security.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct SecuritySettings {
    /// Admin actions need a sign-in this recent, in minutes.
    pub fresh_auth_minutes: u32,
    /// Session lifetime in hours.
    pub session_hours: u32,
}

impl Default for SecuritySettings {
    fn default() -> Self {
        Self {
            fresh_auth_minutes: 5,
            session_hours: 336,
        }
    }
}

/// The organisation's look (R-TH-1, R-TH-4; ADR 0007 D245).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AppearanceSettings {
    /// The seed colour every built-in theme is derived from, `#rrggbb`.
    /// People can still pick their own accent in the app.
    pub seed: String,
    /// The theme exports (PDF, HTML, Word) use: a light theme, since
    /// paper is white.
    pub export_theme: String,
    /// The theme of published sites; `auto` follows the reader's system
    /// light or dark setting.
    pub site_theme: String,
}

/// The theme ids exports may use.
pub const EXPORT_THEMES: [&str; 3] = ["light", "high-contrast", "sepia"];
/// The theme ids published sites may use.
pub const SITE_THEMES: [&str; 6] = ["auto", "light", "dark", "high-contrast", "sepia", "dim"];

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            seed: "#4f5b92".to_owned(),
            export_theme: "light".to_owned(),
            site_theme: "auto".to_owned(),
        }
    }
}

impl AppearanceSettings {
    fn problems(&self, problems: &mut Vec<String>) {
        let seed = self.seed.as_bytes();
        if seed.len() != 7
            || seed.first() != Some(&b'#')
            || !seed.iter().skip(1).all(u8::is_ascii_hexdigit)
        {
            problems.push("appearance.seed must be a #rrggbb colour".to_owned());
        }
        if !EXPORT_THEMES.contains(&self.export_theme.as_str()) {
            problems.push(format!(
                "appearance.export_theme must be one of {}",
                EXPORT_THEMES.join(", ")
            ));
        }
        if !SITE_THEMES.contains(&self.site_theme.as_str()) {
            problems.push(format!(
                "appearance.site_theme must be one of {}",
                SITE_THEMES.join(", ")
            ));
        }
    }
}

/// Every admin setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct AdminSettings {
    /// The organisation's display name.
    pub organisation: String,
    /// Anonymous usage reporting.
    pub telemetry: TelemetrySettings,
    /// Built-in observability.
    pub observability: ObservabilitySettings,
    /// Backups.
    pub backups: BackupSettings,
    /// Admin-session security.
    pub security: SecuritySettings,
    /// The organisation's seed colour and export and site themes.
    pub appearance: AppearanceSettings,
}

impl Default for AdminSettings {
    fn default() -> Self {
        Self {
            organisation: "Vellum".to_owned(),
            telemetry: TelemetrySettings::default(),
            observability: ObservabilitySettings::default(),
            backups: BackupSettings::default(),
            security: SecuritySettings::default(),
            appearance: AppearanceSettings::default(),
        }
    }
}

fn check<T: PartialOrd + std::fmt::Display + Copy>(
    problems: &mut Vec<String>,
    field: &str,
    value: T,
    min: T,
    max: T,
) {
    if value < min || value > max {
        problems.push(format!("{field} must be between {min} and {max}"));
    }
}

impl AdminSettings {
    /// Checks every field.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error listing every problem.
    pub fn validate(&self) -> Result<()> {
        let mut problems = Vec::new();
        let name = self.organisation.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            problems.push("organisation must be 1 to 100 printable characters".to_owned());
        }
        let t = &self.telemetry;
        if t.install_id.len() != 32 || !t.install_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            problems.push("telemetry.install_id must be 32 hex digits".to_owned());
        }
        match (&t.endpoint, t.enabled) {
            (Some(url), _) if !url.starts_with("https://") || url.len() > 2048 => {
                problems.push("telemetry.endpoint must be an https:// URL".to_owned());
            }
            (None, true) => {
                problems.push("telemetry.endpoint is required to enable telemetry".to_owned());
            }
            _ => {}
        }
        check(
            &mut problems,
            "telemetry.interval_hours",
            t.interval_hours,
            24,
            720,
        );
        let o = &self.observability;
        check(
            &mut problems,
            "observability.trace_buffer_capacity",
            o.trace_buffer_capacity,
            100,
            100_000,
        );
        check(
            &mut problems,
            "observability.trace_buffer_mib",
            o.trace_buffer_mib,
            1,
            1024,
        );
        check(
            &mut problems,
            "observability.slow_request_ms",
            o.slow_request_ms,
            10,
            60_000,
        );
        problems.extend(
            o.budgets
                .problems()
                .into_iter()
                .map(|p| format!("observability.budgets: {p}")),
        );
        let b = &self.backups;
        check(
            &mut problems,
            "backups.interval_hours",
            b.interval_hours,
            1,
            168,
        );
        check(&mut problems, "backups.keep", b.keep, 1, 365);
        check(
            &mut problems,
            "backups.drill_interval_days",
            b.drill_interval_days,
            1,
            31,
        );
        if b.destination.trim().is_empty() || b.destination.split(['/', '\\']).any(|p| p == "..") {
            problems.push("backups.destination must be a path without '..'".to_owned());
        }
        let s = &self.security;
        check(
            &mut problems,
            "security.fresh_auth_minutes",
            s.fresh_auth_minutes,
            1,
            60,
        );
        check(
            &mut problems,
            "security.session_hours",
            s.session_hours,
            1,
            720,
        );
        self.appearance.problems(&mut problems);
        if problems.is_empty() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::InvalidInput, problems.join("; ")))
        }
    }
}

/// One changed field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FieldChange {
    /// Dotted path, such as `backups.keep`.
    pub path: String,
    /// The old value, as JSON.
    pub from: serde_json::Value,
    /// The new value, as JSON.
    pub to: serde_json::Value,
}

fn flatten(prefix: &str, value: &serde_json::Value, out: &mut BTreeMap<String, serde_json::Value>) {
    if let serde_json::Value::Object(map) = value {
        for (k, v) in map {
            let path = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{prefix}.{k}")
            };
            flatten(&path, v, out);
        }
    } else {
        let _ = out.insert(prefix.to_owned(), value.clone());
    }
}

/// The fields that differ between two settings documents.
#[must_use]
pub fn diff(old: &AdminSettings, new: &AdminSettings) -> Vec<FieldChange> {
    let mut a = BTreeMap::new();
    let mut b = BTreeMap::new();
    flatten("", &serde_json::to_value(old).unwrap_or_default(), &mut a);
    flatten("", &serde_json::to_value(new).unwrap_or_default(), &mut b);
    b.into_iter()
        .filter_map(|(path, to)| {
            let from = a.get(&path).cloned().unwrap_or_default();
            (from != to).then_some(FieldChange { path, from, to })
        })
        .collect()
}

/// Settings with the version they were stored at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Versioned {
    /// Increases by one on every save; 0 before the first.
    pub version: u64,
    /// The settings.
    pub settings: AdminSettings,
}

/// Where settings are kept.
#[async_trait]
pub trait SettingsStore: Send + Sync {
    /// The current settings (defaults before the first save).
    async fn load(&self) -> Result<Versioned>;
    /// Saves `settings` if the stored version is still `expected_version`;
    /// returns the new version.
    async fn save(&self, expected_version: u64, settings: &AdminSettings) -> Result<u64>;
}

fn conflict(expected: u64, actual: u64) -> Error {
    Error::new(
        ErrorKind::Conflict,
        format!("settings changed since version {expected} (now {actual}); reload and retry"),
    )
}

/// Settings in memory.
#[derive(Debug, Default)]
pub struct MemorySettingsStore {
    current: Mutex<Option<Versioned>>,
}

#[async_trait]
impl SettingsStore for MemorySettingsStore {
    async fn load(&self) -> Result<Versioned> {
        Ok(self
            .current
            .lock()
            .await
            .clone()
            .unwrap_or_else(|| Versioned {
                version: 0,
                settings: AdminSettings::default(),
            }))
    }

    async fn save(&self, expected_version: u64, settings: &AdminSettings) -> Result<u64> {
        settings.validate()?;
        let mut current = self.current.lock().await;
        let actual = current.as_ref().map_or(0, |v| v.version);
        if actual != expected_version {
            return Err(conflict(expected_version, actual));
        }
        *current = Some(Versioned {
            version: actual + 1,
            settings: settings.clone(),
        });
        drop(current);
        Ok(actual + 1)
    }
}

/// Settings in one JSON file, normally `.vellum/admin/settings.json`.
#[derive(Debug)]
pub struct FileSettingsStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileSettingsStore {
    /// A store at `path`; the file is created on first save.
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    async fn read(&self) -> Result<Versioned> {
        match tokio::fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
                Error::new(ErrorKind::Internal, "the settings file is not valid").with_source(e)
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // First start: persist fresh defaults so the install id stays
                // stable across restarts.
                let fresh = Versioned {
                    version: 0,
                    settings: AdminSettings::default(),
                };
                self.write(&fresh).await?;
                Ok(fresh)
            }
            Err(e) => Err(
                Error::new(ErrorKind::Unavailable, "cannot read the settings file").with_source(e),
            ),
        }
    }

    async fn write(&self, v: &Versioned) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                Error::new(
                    ErrorKind::Unavailable,
                    "cannot create the settings directory",
                )
                .with_source(e)
            })?;
        }
        let bytes = serde_json::to_vec_pretty(v).map_err(|e| {
            Error::new(ErrorKind::Internal, "cannot encode settings").with_source(e)
        })?;
        crate::fsutil::write_atomic(&self.path, &bytes).await
    }
}

#[async_trait]
impl SettingsStore for FileSettingsStore {
    async fn load(&self) -> Result<Versioned> {
        let _guard = self.lock.lock().await;
        self.read().await
    }

    async fn save(&self, expected_version: u64, settings: &AdminSettings) -> Result<u64> {
        settings.validate()?;
        let _guard = self.lock.lock().await;
        let current = self.read().await?;
        if current.version != expected_version {
            return Err(conflict(expected_version, current.version));
        }
        let next = Versioned {
            version: current.version + 1,
            settings: settings.clone(),
        };
        self.write(&next).await?;
        Ok(next.version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_telemetry_is_off() {
        let s = AdminSettings::default();
        s.validate().unwrap();
        assert!(!s.telemetry.enabled);
        assert_ne!(new_install_id(), new_install_id());
    }

    #[test]
    fn validation_lists_every_problem() {
        let mut s = AdminSettings {
            organisation: " ".into(),
            ..AdminSettings::default()
        };
        s.telemetry.enabled = true;
        s.telemetry.install_id = "nope".into();
        s.backups.keep = 0;
        s.backups.destination = "../etc".into();
        s.observability.trace_buffer_capacity = 5;
        let err = s.validate().unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        for needle in [
            "organisation",
            "install_id",
            "endpoint is required",
            "backups.keep",
            "destination",
            "trace_buffer_capacity",
        ] {
            assert!(
                err.message().contains(needle),
                "{needle}: {}",
                err.message()
            );
        }
        let mut http = AdminSettings::default();
        http.telemetry.endpoint = Some("http://insecure".into());
        assert!(http.validate().unwrap_err().message().contains("https://"));
    }

    #[test]
    fn appearance_is_checked() {
        let mut s = AdminSettings::default();
        s.appearance.seed = "#ABCDEF".into();
        s.appearance.export_theme = "sepia".into();
        s.appearance.site_theme = "dim".into();
        s.validate().unwrap();
        for (seed, export, site, needle) in [
            ("red", "light", "auto", "appearance.seed"),
            ("#12345", "light", "auto", "appearance.seed"),
            ("#12345g", "light", "auto", "appearance.seed"),
            ("#123456", "dark", "auto", "export_theme"),
            ("#123456", "light", "neon", "site_theme"),
        ] {
            s.appearance = AppearanceSettings {
                seed: seed.into(),
                export_theme: export.into(),
                site_theme: site.into(),
            };
            let err = s.validate().unwrap_err();
            assert!(
                err.message().contains(needle),
                "{needle}: {}",
                err.message()
            );
        }
        // Settings saved before appearance existed still load.
        let old: AdminSettings = serde_json::from_str(r#"{"organisation":"Northstar"}"#).unwrap();
        assert_eq!(old.appearance, AppearanceSettings::default());
    }

    #[test]
    fn diffs_fields() {
        let a = AdminSettings::default();
        let mut b = a.clone();
        b.backups.keep = 30;
        b.organisation = "Northstar".into();
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 2);
        let keep = changes.iter().find(|c| c.path == "backups.keep").unwrap();
        assert_eq!(keep.from, serde_json::json!(14));
        assert_eq!(keep.to, serde_json::json!(30));
    }

    #[tokio::test]
    async fn memory_store_versions() {
        let store = MemorySettingsStore::default();
        let v0 = store.load().await.unwrap();
        assert_eq!(v0.version, 0);
        assert_eq!(store.save(0, &v0.settings).await.unwrap(), 1);
        assert_eq!(
            store.save(0, &v0.settings).await.unwrap_err().kind(),
            ErrorKind::Conflict
        );
        let mut bad = v0.settings;
        bad.backups.keep = 0;
        assert_eq!(
            store.save(1, &bad).await.unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }

    #[tokio::test]
    async fn file_store_persists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("admin/settings.json");
        let store = FileSettingsStore::new(&path);
        let first = store.load().await.unwrap();
        // The install id is persisted on first load.
        assert_eq!(
            store.load().await.unwrap().settings.telemetry.install_id,
            first.settings.telemetry.install_id
        );
        let mut s = first.settings.clone();
        s.organisation = "Northstar".into();
        assert_eq!(store.save(0, &s).await.unwrap(), 1);
        let reopened = FileSettingsStore::new(&path).load().await.unwrap();
        assert_eq!(reopened.version, 1);
        assert_eq!(reopened.settings.organisation, "Northstar");
        assert_eq!(
            store.save(0, &s).await.unwrap_err().kind(),
            ErrorKind::Conflict
        );
        std::fs::write(&path, "{").unwrap();
        assert_eq!(store.load().await.unwrap_err().kind(), ErrorKind::Internal);
    }
}
