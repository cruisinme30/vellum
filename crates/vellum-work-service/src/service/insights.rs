// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Insights over the gateway: how long work takes and how much gets done.
//!
//! The engine in `vellum_work::insights` does the counting; this module
//! reads the question from strings, narrows it to a team when asked, and
//! returns plain numbers a chart can draw.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use vellum_contracts::RequestContext;
use vellum_kernel::Error;
use vellum_work::filter::{Condition, Field, Filter, Op};
use vellum_work::insights::{self, Bucket, Query, Stats};

use super::{WorkService, date};
use crate::api::{invalid, parse_name};

/// An insights question.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct InsightsQuery {
    /// Only this team's issues (its key).
    pub team: Option<String>,
    /// A filter as JSON, the same as a view's; empty means all issues.
    pub filter: Option<String>,
    /// `lead_time` (default), `cycle_time`, `triage_time`, `throughput`
    /// or `created`.
    pub measure: Option<String>,
    /// Break down by `none` (default), `team`, `assignee`, `priority`,
    /// `label`, `project`, `cycle` or `estimate`.
    pub pivot: Option<String>,
    /// First day of the range, `YYYY-MM-DD`.
    pub from: String,
    /// Day after the last day of the range, `YYYY-MM-DD`.
    pub to: String,
    /// Split each breakdown into `week`s or `month`s.
    pub interval: Option<String>,
}

/// Summary numbers for a set of issues.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct StatsOut {
    /// How many issues.
    pub count: u32,
    /// Median, in hours; absent for counts.
    pub p50_hours: Option<f64>,
    /// 85th percentile, in hours: most work finishes within this.
    pub p85_hours: Option<f64>,
    /// 95th percentile, in hours.
    pub p95_hours: Option<f64>,
    /// Mean, in hours.
    pub mean_hours: Option<f64>,
}

impl From<Stats> for StatsOut {
    fn from(s: Stats) -> Self {
        Self {
            count: s.count,
            p50_hours: s.p50_hours,
            p85_hours: s.p85_hours,
            p95_hours: s.p95_hours,
            mean_hours: s.mean_hours,
        }
    }
}

/// One week or month of a series.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PointOut {
    /// First day of the week or month, `YYYY-MM-DD`.
    pub start: String,
    /// The numbers for that interval.
    pub stats: StatsOut,
}

/// One breakdown value.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BucketOut {
    /// The value, such as `ENG`, `dev` or `urgent`; empty means "none".
    pub key: Option<String>,
    /// Over the whole range.
    pub overall: StatsOut,
    /// Per interval, when asked for; every interval is present.
    pub series: Vec<PointOut>,
}

impl From<Bucket> for BucketOut {
    fn from(b: Bucket) -> Self {
        Self {
            key: b.key,
            overall: b.overall.into(),
            series: b
                .series
                .into_iter()
                .map(|p| PointOut {
                    start: p.start.to_string(),
                    stats: p.stats.into(),
                })
                .collect(),
        }
    }
}

/// The answer to an insights question.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct InsightsOut {
    /// Everything together.
    pub overall: StatsOut,
    /// Per breakdown value, largest first.
    pub buckets: Vec<BucketOut>,
}

impl WorkService {
    pub(crate) async fn insights(
        &self,
        rc: RequestContext,
        i: InsightsQuery,
    ) -> Result<InsightsOut, Error> {
        let ctx = self.ctx(&rc).await?;
        let st = self.state.lock().await;
        let mut filter = match &i.filter {
            Some(text) => {
                let f: Filter =
                    serde_json::from_str(text).map_err(|e| invalid(&format!("filter: {e}")))?;
                f.check().map_err(|e| invalid(&format!("filter: {e}")))?;
                f
            }
            None => Filter::default(),
        };
        if let Some(team) = &i.team {
            let key = Self::team_key(&st.graph, team)?.to_string();
            let only = Condition::new(Field::Team, Op::Is, &[key.as_str()]).into();
            filter = Filter::And {
                and: vec![only, filter],
            };
        }
        let query = Query {
            filter,
            measure: i
                .measure
                .as_deref()
                .map(|t| parse_name("measure", t))
                .transpose()?
                .unwrap_or_default(),
            pivot: i
                .pivot
                .as_deref()
                .map(|t| parse_name("pivot", t))
                .transpose()?
                .unwrap_or_default(),
            from: date(&i.from)?,
            to: date(&i.to)?,
            interval: i
                .interval
                .as_deref()
                .map(|t| parse_name("interval", t))
                .transpose()?,
        };
        let answer = insights::run(&st.graph, &query, &ctx, ctx.actor.as_ref())?;
        Ok(InsightsOut {
            overall: answer.overall.into(),
            buckets: answer.buckets.into_iter().map(Into::into).collect(),
        })
    }
}
