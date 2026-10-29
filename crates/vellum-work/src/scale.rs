// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Priority (0 to 4) and per-team estimate scales.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::WorkError;

/// Issue priority, written as a number in files.
///
/// `0` none · `1` urgent · `2` high · `3` normal · `4` low. Sorting by
/// [`Priority::rank`] puts urgent first and none last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Priority {
    /// No priority set (`0`).
    #[default]
    None,
    /// Urgent (`1`).
    Urgent,
    /// High (`2`).
    High,
    /// Normal (`3`).
    Normal,
    /// Low (`4`).
    Low,
}

impl Priority {
    /// Every priority, in sort order (urgent first, none last).
    pub const ALL: [Self; 5] = [
        Self::Urgent,
        Self::High,
        Self::Normal,
        Self::Low,
        Self::None,
    ];

    /// The number written in files.
    #[must_use]
    pub const fn number(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Urgent => 1,
            Self::High => 2,
            Self::Normal => 3,
            Self::Low => 4,
        }
    }

    /// The priority for a file number.
    #[must_use]
    pub const fn from_number(n: u8) -> Option<Self> {
        match n {
            0 => Some(Self::None),
            1 => Some(Self::Urgent),
            2 => Some(Self::High),
            3 => Some(Self::Normal),
            4 => Some(Self::Low),
            _ => None,
        }
    }

    /// The lowercase name used in filters and URLs (`urgent`).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Urgent => "urgent",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }

    /// Parses a name (`urgent`) or a number (`1`).
    #[must_use]
    pub fn from_name(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|p| p.name() == text)
            .or_else(|| text.parse().ok().and_then(Self::from_number))
    }

    /// The sort rank: urgent 0, high 1, normal 2, low 3, none 4.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::None => 4,
            p => p.number() - 1,
        }
    }
}

impl fmt::Display for Priority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl Serialize for Priority {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(self.number())
    }
}

impl<'de> Deserialize<'de> for Priority {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let n = u8::deserialize(d)?;
        Self::from_number(n)
            .ok_or_else(|| serde::de::Error::custom(format!("priority {n} is not 0 to 4")))
    }
}

/// T-shirt sizes for the t-shirt scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Size {
    /// Extra small (1 point).
    XS,
    /// Small (2 points).
    S,
    /// Medium (3 points).
    M,
    /// Large (5 points).
    L,
    /// Extra large (8 points).
    XL,
}

impl Size {
    const ALL: [Self; 5] = [Self::XS, Self::S, Self::M, Self::L, Self::XL];

    /// The points a size counts for in burn-up, velocity and progress.
    #[must_use]
    pub const fn points(self) -> u32 {
        match self {
            Self::XS => 1,
            Self::S => 2,
            Self::M => 3,
            Self::L => 5,
            Self::XL => 8,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::XS => "XS",
            Self::S => "S",
            Self::M => "M",
            Self::L => "L",
            Self::XL => "XL",
        }
    }
}

/// An estimate as written in an issue file: points or a t-shirt size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Estimate {
    /// A point value on the linear, Fibonacci or exponential scale.
    Points(u32),
    /// A size on the t-shirt scale.
    Size(Size),
}

impl Estimate {
    /// The points this estimate counts for.
    #[must_use]
    pub const fn points(self) -> u32 {
        match self {
            Self::Points(p) => p,
            Self::Size(s) => s.points(),
        }
    }

    /// Parses `5` or `M`.
    #[must_use]
    pub fn from_text(text: &str) -> Option<Self> {
        text.parse().ok().map(Self::Points).or_else(|| {
            Size::ALL
                .into_iter()
                .find(|s| s.name() == text)
                .map(Self::Size)
        })
    }
}

impl fmt::Display for Estimate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Points(p) => write!(f, "{p}"),
            Self::Size(s) => f.write_str(s.name()),
        }
    }
}

/// A team's estimate scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EstimateScale {
    /// 1, 2, 3, 4, 5.
    Linear,
    /// 1, 2, 3, 5, 8.
    #[default]
    Fibonacci,
    /// 1, 2, 4, 8, 16.
    Exponential,
    /// XS, S, M, L, XL.
    TShirt,
}

impl EstimateScale {
    /// The scale's name, as written in `team.yaml`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Fibonacci => "fibonacci",
            Self::Exponential => "exponential",
            Self::TShirt => "t-shirt",
        }
    }

    /// The values on the scale, smallest first.
    #[must_use]
    pub fn values(self) -> Vec<Estimate> {
        let points = |v: &[u32]| v.iter().copied().map(Estimate::Points).collect();
        match self {
            Self::Linear => points(&[1, 2, 3, 4, 5]),
            Self::Fibonacci => points(&[1, 2, 3, 5, 8]),
            Self::Exponential => points(&[1, 2, 4, 8, 16]),
            Self::TShirt => Size::ALL.into_iter().map(Estimate::Size).collect(),
        }
    }

    /// Checks that an estimate is on this scale; `0` is allowed when
    /// `allow_zero` is set and the scale is numeric.
    ///
    /// # Errors
    ///
    /// Returns [`WorkError::InvalidEstimate`] otherwise.
    pub fn check(self, estimate: Estimate, allow_zero: bool) -> Result<(), WorkError> {
        let zero_ok = allow_zero && estimate == Estimate::Points(0) && self != Self::TShirt;
        if zero_ok || self.values().contains(&estimate) {
            Ok(())
        } else {
            Err(WorkError::InvalidEstimate {
                value: estimate.to_string(),
                scale: self.name(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_round_trips_and_sorts_urgent_first() {
        for n in 0..=4 {
            assert_eq!(Priority::from_number(n).unwrap().number(), n);
        }
        assert!(Priority::from_number(5).is_none());
        let mut all = vec![
            Priority::None,
            Priority::Low,
            Priority::Urgent,
            Priority::High,
        ];
        all.sort_by_key(|p| p.rank());
        assert_eq!(
            all,
            [
                Priority::Urgent,
                Priority::High,
                Priority::Low,
                Priority::None
            ]
        );
        assert_eq!(Priority::from_name("high"), Some(Priority::High));
        assert_eq!(Priority::from_name("1"), Some(Priority::Urgent));
    }

    #[test]
    fn estimates_are_checked_against_the_scale() {
        let fib = EstimateScale::Fibonacci;
        assert!(fib.check(Estimate::Points(5), false).is_ok());
        assert!(fib.check(Estimate::Points(4), false).is_err());
        assert!(fib.check(Estimate::Points(0), false).is_err());
        assert!(fib.check(Estimate::Points(0), true).is_ok());
        let shirt = EstimateScale::TShirt;
        assert!(shirt.check(Estimate::Size(Size::M), true).is_ok());
        assert!(shirt.check(Estimate::Points(0), true).is_err());
        assert_eq!(Estimate::from_text("XL").unwrap().points(), 8);
    }

    #[test]
    fn estimates_serialise_as_numbers_or_sizes() {
        let yaml =
            serde_yaml_ng::to_string(&vec![Estimate::Points(3), Estimate::Size(Size::L)]).unwrap();
        assert_eq!(yaml, "- 3\n- L\n");
        let back: Vec<Estimate> = serde_yaml_ng::from_str(&yaml).unwrap();
        assert_eq!(back, [Estimate::Points(3), Estimate::Size(Size::L)]);
    }
}
