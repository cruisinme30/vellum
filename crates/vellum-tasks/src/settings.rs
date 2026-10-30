// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! A person's My Work settings: their time zone, when their day starts
//! and when the morning digest goes out (#472, D178).
//!
//! "Today", relative reminders and the digest are all counted in the
//! person's own zone. Someone who never chose one follows the server's.

use jiff::civil::Time;
use jiff::tz::TimeZone;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_kernel::{Error, ErrorKind};

use crate::remind;

/// The longest time zone name accepted; IANA names are far shorter.
const MAX_ZONE: usize = 64;

/// A person's stored My Work settings, checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preferences {
    /// Their chosen zone and its IANA name; `None` follows the server.
    pub time_zone: Option<(String, TimeZone)>,
    /// The time a relative reminder on a date-only deadline counts from.
    pub day_starts_at: Time,
    /// When the daily digest is sent; `None` turns it off.
    pub digest_at: Option<Time>,
}

impl Default for Preferences {
    fn default() -> Self {
        let defaults = remind::Settings::new(TimeZone::UTC);
        Self {
            time_zone: None,
            day_starts_at: defaults.day_starts_at,
            digest_at: defaults.digest_at,
        }
    }
}

impl Preferences {
    /// The zone to count in: the person's, else `server`.
    #[must_use]
    pub fn zone_or(&self, server: &TimeZone) -> TimeZone {
        self.time_zone
            .as_ref()
            .map_or_else(|| server.clone(), |(_, zone)| zone.clone())
    }

    /// The reminder settings these make in `zone`.
    #[must_use]
    pub const fn reminders(&self, zone: TimeZone) -> remind::Settings {
        remind::Settings {
            time_zone: zone,
            day_starts_at: self.day_starts_at,
            digest_at: self.digest_at,
        }
    }

    /// Checks settings as a client or a settings file wrote them.
    ///
    /// # Errors
    ///
    /// `invalid` for an unknown time zone or a time that is not `HH:MM`.
    pub fn parse(input: &MyWorkSettingsInput) -> Result<Self, Error> {
        let time_zone = match input.time_zone.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(name) => Some((name.to_owned(), zone(name)?)),
        };
        Ok(Self {
            time_zone,
            day_starts_at: clock(&input.day_starts_at, "day_starts_at")?,
            digest_at: input
                .digest_at
                .as_deref()
                .map(|t| clock(t, "digest_at"))
                .transpose()?,
        })
    }

    /// The settings in their stored and client form.
    #[must_use]
    pub fn to_input(&self) -> MyWorkSettingsInput {
        MyWorkSettingsInput {
            time_zone: self.time_zone.as_ref().map(|(name, _)| name.clone()),
            day_starts_at: hh_mm(self.day_starts_at),
            digest_at: self.digest_at.map(hh_mm),
        }
    }
}

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidInput, message.into())
}

/// Looks up an IANA zone name such as `Europe/Berlin` or `UTC`.
fn zone(name: &str) -> Result<TimeZone, Error> {
    let known = name.len() <= MAX_ZONE
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+')
        });
    let found = known.then(|| TimeZone::get(name).ok()).flatten();
    found.ok_or_else(|| invalid(format!("unknown time zone \"{name}\"")))
}

/// Reads `HH:MM`.
fn clock(text: &str, field: &str) -> Result<Time, Error> {
    let bad = || invalid(format!("{field} must be a time such as 09:00"));
    let (h, m) = text.trim().split_once(':').ok_or_else(bad)?;
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return Err(bad());
    }
    let hour: i8 = h.parse().map_err(|_| bad())?;
    let minute: i8 = m.parse().map_err(|_| bad())?;
    Time::new(hour, minute, 0, 0).map_err(|_| bad())
}

fn hh_mm(time: Time) -> String {
    format!("{:02}:{:02}", time.hour(), time.minute())
}

fn default_day_start() -> String {
    hh_mm(Preferences::default().day_starts_at)
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "serde's default must return the field's type"
)]
fn default_digest() -> Option<String> {
    Preferences::default().digest_at.map(hh_mm)
}

/// My Work settings as a client sends them, and as they are stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MyWorkSettingsInput {
    /// An IANA time zone such as `Europe/Berlin`; `null` follows the
    /// server's zone.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// When the day starts, `HH:MM`; reminders on a date-only deadline
    /// count from it.
    #[serde(default = "default_day_start")]
    pub day_starts_at: String,
    /// When the morning digest is sent, `HH:MM`; `null` turns it off.
    #[serde(default = "default_digest")]
    pub digest_at: Option<String>,
}

/// A person's My Work settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MyWorkSettingsView {
    /// The zone they chose; `null` when they follow the server's.
    pub time_zone: Option<String>,
    /// The zone My Work counts in for them now.
    pub effective_time_zone: String,
    /// When their day starts, `HH:MM`.
    pub day_starts_at: String,
    /// When their digest is sent, `HH:MM`; `null` when it is off.
    pub digest_at: Option<String>,
}

impl MyWorkSettingsView {
    /// The view of `prefs` for someone whose zone is now `effective`.
    #[must_use]
    pub fn new(prefs: &Preferences, effective: &TimeZone) -> Self {
        let input = prefs.to_input();
        Self {
            time_zone: input.time_zone,
            effective_time_zone: effective
                .iana_name()
                .unwrap_or("UTC")
                .to_owned(),
            day_starts_at: input.day_starts_at,
            digest_at: input.digest_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(
        zone: Option<&str>,
        day: &str,
        digest: Option<&str>,
    ) -> MyWorkSettingsInput {
        MyWorkSettingsInput {
            time_zone: zone.map(str::to_owned),
            day_starts_at: day.to_owned(),
            digest_at: digest.map(str::to_owned),
        }
    }

    #[test]
    fn settings_round_trip() {
        let given = input(Some("America/New_York"), "08:30", None);
        let prefs = Preferences::parse(&given).unwrap();
        assert_eq!(prefs.day_starts_at, Time::constant(8, 30, 0, 0));
        assert_eq!(prefs.digest_at, None);
        assert_eq!(prefs.to_input(), given);
        let view =
            MyWorkSettingsView::new(&prefs, &prefs.zone_or(&TimeZone::UTC));
        assert_eq!(view.effective_time_zone, "America/New_York");
    }

    #[test]
    fn no_zone_follows_the_server() {
        let prefs =
            Preferences::parse(&input(Some(" "), "09:00", Some("07:30")))
                .unwrap();
        assert_eq!(prefs.time_zone, None);
        let server = TimeZone::get("Asia/Kolkata").unwrap();
        assert_eq!(prefs.zone_or(&server), server);
    }

    #[test]
    fn missing_fields_take_the_defaults() {
        let parsed: MyWorkSettingsInput = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed, input(None, "09:00", Some("07:30")));
    }

    #[test]
    fn bad_zones_and_times_are_refused() {
        for zone in [
            "Mars/Olympus",
            "../../etc/passwd",
            "Europe/Berlin;rm",
            &"A".repeat(80),
        ] {
            let err = Preferences::parse(&input(Some(zone), "09:00", None))
                .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::InvalidInput, "{zone:?}");
        }
        for time in ["9", "24:00", "09:60", "9:5", "-1:00", "09:00:00", "ab:cd"]
        {
            assert!(
                Preferences::parse(&input(None, time, None)).is_err(),
                "{time}"
            );
            assert!(
                Preferences::parse(&input(None, "09:00", Some(time))).is_err(),
                "{time}"
            );
        }
        assert!(
            Preferences::parse(&input(None, "7:05", Some("23:59"))).is_ok()
        );
    }
}
