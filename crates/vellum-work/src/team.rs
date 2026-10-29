// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Teams: key prefixes, workflows and team settings (`teams/KEY/team.yaml`).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::cycle::CycleSettings;
use crate::error::WorkError;
use crate::id::{TeamId, UserRef};
use crate::label::Label;
use crate::scale::EstimateScale;
use crate::sla::SlaPolicy;
use crate::template::Template;
use crate::triage::TriageRule;
use crate::workflow::Workflow;

/// A team's key prefix, such as `ENG`.
///
/// Two to seven characters: an uppercase letter, then uppercase letters or
/// digits.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct TeamKey(String);

impl TeamKey {
    /// Parses a key, accepting any case and normalising to uppercase.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::Invalid`] if the key is malformed.
    pub fn parse(text: &str) -> Result<Self, WorkError> {
        let upper = text.to_ascii_uppercase();
        let invalid = |reason| WorkError::Invalid {
            what: "team key",
            value: text.to_owned(),
            reason,
        };
        if !(2..=7).contains(&upper.len()) {
            return Err(invalid("must be 2 to 7 characters"));
        }
        if !upper.bytes().next().is_some_and(|b| b.is_ascii_uppercase()) {
            return Err(invalid("must start with a letter"));
        }
        if !upper
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err(invalid("must be letters and digits"));
        }
        Ok(Self(upper))
    }

    /// The key, uppercase.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TeamKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for TeamKey {
    type Err = WorkError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl<'de> Deserialize<'de> for TeamKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

/// How a team estimates (`estimates:` in `team.yaml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimateSettings {
    /// The scale issue estimates must be on.
    #[serde(default)]
    pub scale: EstimateScale,
    /// Whether `0` is an allowed estimate on numeric scales.
    #[serde(default)]
    pub allow_zero: bool,
    /// The points an unestimated issue counts for in burn-up, velocity and
    /// progress.
    #[serde(default = "one")]
    pub unestimated_points: u32,
}

const fn one() -> u32 {
    1
}

impl Default for EstimateSettings {
    fn default() -> Self {
        Self {
            scale: EstimateScale::default(),
            allow_zero: false,
            unestimated_points: 1,
        }
    }
}

/// A team and everything it owns, stored as `teams/KEY/team.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Team {
    /// The team's immutable ID.
    pub id: TeamId,
    /// The key prefix of the team's issue identifiers.
    pub key: TeamKey,
    /// The display name.
    pub name: String,
    /// The IANA time zone cycles and recurring issues follow, such as
    /// `Europe/London`.
    #[serde(default = "utc")]
    pub timezone: String,
    /// The team lead, who also receives SLA breach notifications.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead: Option<UserRef>,
    /// The team's members. Issues created by non-members land in triage.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<UserRef>,
    /// The highest issue number ever handed out, including numbers now
    /// held only as aliases. Numbers are never reused.
    #[serde(default)]
    pub last_number: u64,
    /// The team's statuses.
    pub workflow: Workflow,
    /// The estimate scale and its options.
    #[serde(default)]
    pub estimates: EstimateSettings,
    /// Cycle settings; `None` turns cycles off for the team.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cycles: Option<CycleSettings>,
    /// Whether the team uses a triage inbox.
    #[serde(default = "yes")]
    pub triage: bool,
    /// Triage rules, applied top to bottom; the first match wins.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triage_rules: Vec<TriageRule>,
    /// The team's SLA rules.
    #[serde(default)]
    pub sla: SlaPolicy,
    /// Labels that belong to this team.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
    /// Issue templates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<Template>,
}

fn utc() -> String {
    "UTC".to_owned()
}

const fn yes() -> bool {
    true
}

impl Team {
    /// A new team with the standard workflow, Fibonacci estimates, triage
    /// on, the default SLA rules and cycles off.
    ///
    /// # Errors
    ///
    /// Never in practice; the standard workflow is valid.
    pub fn new(id: TeamId, key: TeamKey, name: &str) -> Result<Self, WorkError> {
        Ok(Self {
            id,
            key,
            name: name.to_owned(),
            timezone: utc(),
            lead: None,
            members: Vec::new(),
            last_number: 0,
            workflow: Workflow::standard()?,
            estimates: EstimateSettings::default(),
            cycles: None,
            triage: true,
            triage_rules: Vec::new(),
            sla: SlaPolicy::standard(),
            labels: Vec::new(),
            templates: Vec::new(),
        })
    }

    /// The path of the team's settings file.
    #[must_use]
    pub fn path(&self) -> String {
        format!("teams/{}/team.yaml", self.key)
    }

    /// Checks every rule a team's settings must follow.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::TeamConfig`] naming the first broken rule.
    pub fn check(&self) -> Result<(), WorkError> {
        let fail = |reason: String| WorkError::TeamConfig {
            team: self.key.clone(),
            reason,
        };
        self.workflow.check().map_err(fail)?;
        let has_triage = self.workflow.first_in(crate::Category::Triage).is_some();
        if self.triage && !has_triage {
            return Err(fail(
                "triage is on but the workflow has no triage status".to_owned(),
            ));
        }
        if let Some(c) = &self.cycles {
            c.check().map_err(|r| fail(r.to_owned()))?;
        }
        for (i, l) in self.labels.iter().enumerate() {
            if self.labels.iter().skip(i + 1).any(|m| m.same_name(l)) {
                return Err(fail(format!("label {} is defined twice", l.name)));
            }
        }
        for (i, t) in self.templates.iter().enumerate() {
            if self.templates.iter().skip(i + 1).any(|u| u.name == t.name) {
                return Err(fail(format!("template {} is defined twice", t.name)));
            }
        }
        Ok(())
    }

    /// Whether `user` is a member or the lead.
    #[must_use]
    pub fn is_member(&self, user: &UserRef) -> bool {
        self.lead.as_ref() == Some(user) || self.members.contains(user)
    }

    /// Parses `team.yaml`.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if the YAML is malformed, and the
    /// [`Team::check`] error if the settings break a rule.
    pub fn from_yaml(path: &str, text: &str) -> Result<Self, WorkError> {
        let team: Self = serde_yaml_ng::from_str(text).map_err(|e| WorkError::File {
            path: path.to_owned(),
            reason: e.to_string(),
        })?;
        team.check()?;
        Ok(team)
    }

    /// Serialises the settings to `team.yaml` text.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::File`] if serialisation fails.
    pub fn to_yaml(&self) -> Result<String, WorkError> {
        serde_yaml_ng::to_string(self).map_err(|e| WorkError::File {
            path: self.path(),
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn team() -> Team {
        Team::new(
            "01JBZ3M8K2Q4R5S6T7V8W9X0T1".parse::<TeamId>().unwrap(),
            TeamKey::parse("eng").unwrap(),
            "Engineering",
        )
        .unwrap()
    }

    #[test]
    fn keys_are_normalised_and_validated() {
        assert_eq!(TeamKey::parse("eng").unwrap().as_str(), "ENG");
        assert!(TeamKey::parse("E").is_err());
        assert!(TeamKey::parse("1ENG").is_err());
        assert!(TeamKey::parse("EN-G").is_err());
        assert!(TeamKey::parse("ABCDEFGH").is_err());
    }

    #[test]
    fn team_yaml_round_trips() {
        let t = team();
        t.check().unwrap();
        let yaml = t.to_yaml().unwrap();
        let back = Team::from_yaml("teams/ENG/team.yaml", &yaml).unwrap();
        assert_eq!(back, t);
        assert_eq!(t.path(), "teams/ENG/team.yaml");
    }

    #[test]
    fn triage_needs_a_triage_status() {
        let mut t = team();
        t.workflow
            .statuses
            .retain(|s| s.category != crate::Category::Triage);
        assert!(t.check().is_err());
        t.triage = false;
        t.check().unwrap();
    }

    #[test]
    fn unknown_keys_in_team_yaml_are_refused() {
        let mut yaml = team().to_yaml().unwrap();
        yaml.push_str("colour: red\n");
        assert!(Team::from_yaml("t", &yaml).is_err());
    }
}
