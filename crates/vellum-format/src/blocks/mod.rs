// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Typed data for data fences (`chart`, `timeline`, …) and data containers
//! (`stats`, `facts`, …), read from the AST.
//!
//! Render, export and the editor all read these types, so a block means the
//! same thing everywhere. Reading is lenient: a line that cannot be read is
//! skipped with a [`Diagnostic`] (its `line` is the 1-based line inside the
//! block, or 0 for the whole block), and everything else is kept. When nothing
//! can be read, callers show the block's source as the readable fallback.

mod containers;
mod fences;

use serde::Serialize;

use crate::ast::{Block, Container, ContainerKind, Fence, FenceKind};
use crate::diagnostic::{Diagnostic, DiagnosticCode};

pub use containers::{
    Card, Cards, Fact, Facts, Progress, ProgressItem, Stat, Stats, Step, Steps,
    Tab, Tabs, Trend,
};
pub use fences::{
    Architecture, Arrow, Board, BoardCard, BoardColumn, Calculator,
    CalculatorInput, CalculatorOutput, Chart, ChartKind, CollectionView,
    DataTable, Image, Lane, Risk, RiskMatrix, Roadmap, RoadmapRow, Series,
    Timeline, TimelineItem,
};

/// Typed data read from a block, with the problems found reading it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Read<T> {
    /// The data that could be read.
    pub data: T,
    /// Lines that could not be read. `line` is 1-based within the block's
    /// body, or 0 for a problem with the block as a whole.
    pub diagnostics: Vec<Diagnostic>,
}

/// The typed data of any data block.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Data {
    /// A `chart` fence.
    Chart(Chart),
    /// A `timeline` fence.
    Timeline(Timeline),
    /// A `roadmap` fence.
    Roadmap(Roadmap),
    /// A `risk-matrix` fence.
    RiskMatrix(RiskMatrix),
    /// A `board` fence.
    Board(Board),
    /// A `table` fence.
    Table(DataTable),
    /// An `architecture` fence.
    Architecture(Architecture),
    /// A `calculator` fence.
    Calculator(Calculator),
    /// An `image` fence.
    Image(Image),
    /// A `collection` fence.
    Collection(CollectionView),
    /// A `stats` container.
    Stats(Stats),
    /// A `facts` container.
    Facts(Facts),
    /// A `progress` container.
    Progress(Progress),
    /// A `cards` container.
    Cards(Cards),
    /// A `steps` container.
    Steps(Steps),
    /// A `tabs` container.
    Tabs(Tabs),
}

impl Data {
    /// Reads the typed data of `block`, or `None` when it is not a data
    /// block (code, diagrams, callouts and other containers).
    #[must_use]
    pub fn from_block(block: &Block) -> Option<Read<Self>> {
        match block {
            Block::Fence(fence) => Self::from_fence(fence),
            Block::Container(container) => Self::from_container(container),
            _ => None,
        }
    }

    /// Reads a data fence. Diagram and code fences return `None`.
    #[must_use]
    pub fn from_fence(fence: &Fence) -> Option<Read<Self>> {
        Some(match fence.kind() {
            FenceKind::Chart => Chart::parse(fence).map(Self::Chart),
            FenceKind::Timeline => Timeline::parse(fence).map(Self::Timeline),
            FenceKind::Roadmap => Roadmap::parse(fence).map(Self::Roadmap),
            FenceKind::RiskMatrix => {
                RiskMatrix::parse(fence).map(Self::RiskMatrix)
            }
            FenceKind::Board => Board::parse(fence).map(Self::Board),
            FenceKind::Table => DataTable::parse(fence).map(Self::Table),
            FenceKind::Architecture => {
                Architecture::parse(fence).map(Self::Architecture)
            }
            FenceKind::Calculator => {
                Calculator::parse(fence).map(Self::Calculator)
            }
            FenceKind::Image => Image::parse(fence).map(Self::Image),
            FenceKind::Collection => {
                CollectionView::parse(fence).map(Self::Collection)
            }
            FenceKind::Code | FenceKind::Diagram => return None,
        })
    }

    /// Reads a data container. Other containers return `None`.
    #[must_use]
    pub fn from_container(container: &Container) -> Option<Read<Self>> {
        Some(match container.kind() {
            ContainerKind::Stats => Stats::parse(container).map(Self::Stats),
            ContainerKind::Facts => Facts::parse(container).map(Self::Facts),
            ContainerKind::Progress => {
                Progress::parse(container).map(Self::Progress)
            }
            ContainerKind::Cards => Cards::parse(container).map(Self::Cards),
            ContainerKind::Steps => Steps::parse(container).map(Self::Steps),
            ContainerKind::Tabs => Tabs::parse(container).map(Self::Tabs),
            _ => return None,
        })
    }
}

impl<T> Read<T> {
    /// Maps the data, keeping the diagnostics.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Read<U> {
        Read {
            data: f(self.data),
            diagnostics: self.diagnostics,
        }
    }
}

/// Collects diagnostics while reading one block.
#[derive(Default)]
struct Problems(Vec<Diagnostic>);

impl Problems {
    fn add(&mut self, line: usize, message: impl Into<String>) {
        self.0
            .push(Diagnostic::new(line, DiagnosticCode::BlockData, message));
    }

    fn done<T>(self, data: T) -> Read<T> {
        Read {
            data,
            diagnostics: self.0,
        }
    }
}

/// The non-blank lines of a fence body with their 1-based line numbers.
fn lines(fence: &Fence) -> impl Iterator<Item = (usize, &str)> {
    fence
        .literal
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line.trim()))
        .filter(|(_, line)| !line.is_empty() && !line.starts_with("//"))
}

/// Reads a number, allowing `_` and `,` as digit group separators.
fn number(s: &str) -> Option<f64> {
    let cleaned: String = s
        .trim()
        .chars()
        .filter(|c| !matches!(c, '_' | ','))
        .collect();
    cleaned.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Reads `[a, b, c]` (brackets optional) as trimmed, non-empty items.
fn list(s: &str) -> Vec<String> {
    let s = s.trim();
    let inner = s
        .strip_prefix('[')
        .and_then(|r| r.strip_suffix(']'))
        .unwrap_or(s);
    inner
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests;
