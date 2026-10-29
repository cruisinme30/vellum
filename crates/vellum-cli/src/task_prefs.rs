// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Each person's My Work settings and dismissed mentions (#472), kept in
//! `<data>/.vellum/tasks/people.json` (or in memory when there is no
//! data folder).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use vellum_kernel::{CommentId, Error, ErrorKind, UserId};
use vellum_tasks::settings::{MyWorkSettingsInput, Preferences};

/// The most dismissed mentions remembered per person; the oldest go
/// first, and by then their threads have usually been answered.
pub const MAX_DISMISSED: usize = 500;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Person {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    settings: Option<MyWorkSettingsInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    dismissed: Vec<CommentId>,
}

fn unavailable(what: &str, e: impl std::fmt::Display) -> Error {
    Error::new(ErrorKind::Unavailable, format!("{what}: {e}"))
}

/// The people file.
#[derive(Debug)]
pub struct PeoplePrefs {
    path: Option<PathBuf>,
    state: Mutex<BTreeMap<UserId, Person>>,
}

impl PeoplePrefs {
    /// Opens `<dir>/people.json`.
    ///
    /// # Errors
    ///
    /// The file exists but cannot be read or is not valid JSON.
    pub fn open(dir: Option<&Path>) -> Result<Self, Error> {
        let path = dir.map(|d| d.join("people.json"));
        let state = match &path {
            Some(p) if p.exists() => {
                let bytes =
                    std::fs::read(p).map_err(|e| unavailable("cannot read people.json", e))?;
                serde_json::from_slice(&bytes)
                    .map_err(|e| unavailable("people.json is not valid JSON", e))?
            }
            _ => BTreeMap::new(),
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, BTreeMap<UserId, Person>>, Error> {
        self.state
            .lock()
            .map_err(|_| Error::new(ErrorKind::Internal, "people lock poisoned"))
    }

    /// A person's settings; defaults when they never saved any, or when
    /// what was saved no longer parses (a zone dropped from the tz
    /// database, say).
    pub fn preferences(&self, user: UserId) -> Preferences {
        let saved = self
            .lock()
            .ok()
            .and_then(|state| state.get(&user).and_then(|p| p.settings.clone()));
        saved
            .and_then(|input| match Preferences::parse(&input) {
                Ok(prefs) => Some(prefs),
                Err(error) => {
                    tracing::warn!(%error, %user, "saved My Work settings ignored");
                    None
                }
            })
            .unwrap_or_default()
    }

    /// The mentions a person dismissed.
    pub fn dismissed(&self, user: UserId) -> Vec<CommentId> {
        self.lock()
            .ok()
            .and_then(|state| state.get(&user).map(|p| p.dismissed.clone()))
            .unwrap_or_default()
    }

    /// Saves a person's settings.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn set_preferences(&self, user: UserId, prefs: &Preferences) -> Result<(), Error> {
        self.change(|state| {
            state.entry(user).or_default().settings = Some(prefs.to_input());
        })
    }

    /// Remembers a dismissed mention.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn dismiss(&self, user: UserId, comment: CommentId) -> Result<(), Error> {
        self.change(|state| {
            let person = state.entry(user).or_default();
            if !person.dismissed.contains(&comment) {
                person.dismissed.push(comment);
            }
            let over = person.dismissed.len().saturating_sub(MAX_DISMISSED);
            let _ = person.dismissed.drain(..over);
        })
    }

    fn change(&self, edit: impl FnOnce(&mut BTreeMap<UserId, Person>)) -> Result<(), Error> {
        let bytes = {
            let mut state = self.lock()?;
            edit(&mut state);
            serde_json::to_vec_pretty(&*state)
                .map_err(|e| Error::new(ErrorKind::Internal, e.to_string()))?
        };
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| unavailable("cannot save people.json", e))?;
        }
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, bytes).map_err(|e| unavailable("cannot save people.json", e))?;
        std::fs::rename(&temp, path).map_err(|e| unavailable("cannot save people.json", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_and_dismissals_persist() {
        let dir = tempfile::tempdir().unwrap();
        let ana = UserId::generate();
        let prefs = Preferences::parse(&MyWorkSettingsInput {
            time_zone: Some("Asia/Kolkata".into()),
            day_starts_at: "10:00".into(),
            digest_at: None,
        })
        .unwrap();
        let comment = CommentId::generate();
        {
            let people = PeoplePrefs::open(Some(dir.path())).unwrap();
            assert_eq!(people.preferences(ana), Preferences::default());
            people.set_preferences(ana, &prefs).unwrap();
            people.dismiss(ana, comment).unwrap();
            people.dismiss(ana, comment).unwrap();
        }
        let people = PeoplePrefs::open(Some(dir.path())).unwrap();
        assert_eq!(people.preferences(ana), prefs);
        assert_eq!(people.dismissed(ana), [comment]);
        assert!(people.dismissed(UserId::generate()).is_empty());
    }

    #[test]
    fn dismissals_are_capped_oldest_first() {
        let people = PeoplePrefs::open(None).unwrap();
        let ana = UserId::generate();
        let first = CommentId::generate();
        people.dismiss(ana, first).unwrap();
        for _ in 0..MAX_DISMISSED {
            people.dismiss(ana, CommentId::generate()).unwrap();
        }
        let kept = people.dismissed(ana);
        assert_eq!(kept.len(), MAX_DISMISSED);
        assert!(!kept.contains(&first));
    }
}
