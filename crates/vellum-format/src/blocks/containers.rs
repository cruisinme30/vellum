// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Data containers: lists inside `:::` whose items hold Markdown, so their
//! fields stay inline content (links and mentions keep working).

use serde::Serialize;

use super::{Problems, Read};
use crate::ast::{Block, Container, Inline};

/// Direction of a stat's change, from a leading ▲, ▼ or ▬.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trend {
    /// ▲
    Up,
    /// ▼
    Down,
    /// ▬
    Flat,
}

/// One stat tile: `- value · label · delta`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Stat {
    /// The figure (`94%`).
    pub value: Vec<Inline>,
    /// What it measures.
    pub label: Vec<Inline>,
    /// Change, without its trend arrow (`6 pts`).
    pub delta: Option<Vec<Inline>>,
    /// Trend arrow of the delta.
    pub trend: Option<Trend>,
}

/// A `stats` container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Stats {
    /// Tiles in order.
    pub items: Vec<Stat>,
}

impl Stats {
    /// Reads a `stats` container.
    #[must_use]
    pub fn parse(container: &Container) -> Read<Self> {
        let mut problems = Problems::default();
        let mut stats = Self::default();
        for (n, content) in items(container, &mut problems) {
            let mut parts = split(content, " · ").into_iter();
            let (Some(value), Some(label)) = (parts.next(), parts.next()) else {
                problems.add(n, "expected `- value · label · delta`");
                continue;
            };
            let (delta, trend) = parts.next().map_or((None, None), |mut delta| {
                let trend = take_trend(&mut delta);
                (Some(delta), trend)
            });
            stats.items.push(Stat {
                value,
                label,
                delta,
                trend,
            });
        }
        problems.done(stats)
    }
}

fn take_trend(delta: &mut Vec<Inline>) -> Option<Trend> {
    let Some(Inline::Text { text }) = delta.first_mut() else {
        return None;
    };
    let trend = match text.chars().next()? {
        '▲' => Trend::Up,
        '▼' => Trend::Down,
        '▬' => Trend::Flat,
        _ => return None,
    };
    let rest: String = text.chars().skip(1).collect();
    rest.trim_start().clone_into(text);
    *delta = trim(std::mem::take(delta));
    Some(trend)
}

/// One `- key: value` fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fact {
    /// The key.
    pub key: Vec<Inline>,
    /// The value.
    pub value: Vec<Inline>,
}

/// A `facts` container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Facts {
    /// Facts in order.
    pub items: Vec<Fact>,
}

impl Facts {
    /// Reads a `facts` container.
    #[must_use]
    pub fn parse(container: &Container) -> Read<Self> {
        let mut problems = Problems::default();
        let mut facts = Self::default();
        for (n, content) in items(container, &mut problems) {
            match split_once(content, ": ") {
                Some((key, value)) => facts.items.push(Fact { key, value }),
                None => problems.add(n, "expected `- key: value`"),
            }
        }
        problems.done(facts)
    }
}

/// One `- label · 70%` bar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProgressItem {
    /// What is progressing.
    pub label: Vec<Inline>,
    /// 0 to 100.
    pub percent: f64,
}

/// A `progress` container.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Progress {
    /// Bars in order.
    pub items: Vec<ProgressItem>,
}

impl Progress {
    /// Reads a `progress` container.
    #[must_use]
    pub fn parse(container: &Container) -> Read<Self> {
        let mut problems = Problems::default();
        let mut progress = Self::default();
        for (n, content) in items(container, &mut problems) {
            let mut parts = split(content, " · ");
            let percent = match parts.pop().as_deref() {
                Some([Inline::Text { text }]) => super::number(text.trim_end_matches('%')),
                _ => None,
            };
            match (percent, parts.is_empty()) {
                (Some(percent), false) => progress.items.push(ProgressItem {
                    label: join(parts, " · "),
                    percent: percent.clamp(0.0, 100.0),
                }),
                _ => problems.add(n, "expected `- label · 70%`"),
            }
        }
        problems.done(progress)
    }
}

/// One card: `- icon **title** description`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Card {
    /// Leading emoji or symbol, if any.
    pub icon: Option<String>,
    /// The bold title.
    pub title: Vec<Inline>,
    /// The rest of the item.
    pub body: Vec<Inline>,
}

/// A `cards` container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Cards {
    /// Cards in order.
    pub cards: Vec<Card>,
}

impl Cards {
    /// Reads a `cards` container.
    #[must_use]
    pub fn parse(container: &Container) -> Read<Self> {
        let mut problems = Problems::default();
        let mut cards = Self::default();
        for (_, mut content) in items(container, &mut problems) {
            let icon = match content.first_mut() {
                Some(Inline::Text { text }) => {
                    let icon: String = text.chars().take_while(|c| !c.is_whitespace()).collect();
                    if icon.chars().any(char::is_alphanumeric) || icon.is_empty() {
                        None
                    } else {
                        *text = text
                            .get(icon.len()..)
                            .unwrap_or_default()
                            .trim_start()
                            .to_owned();
                        Some(icon)
                    }
                }
                _ => None,
            };
            let (title, body) = titled(trim(content));
            cards.cards.push(Card { icon, title, body });
        }
        problems.done(cards)
    }
}

/// One step: `1. **title** detail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    /// The bold title, or the whole item when there is none.
    pub title: Vec<Inline>,
    /// The rest of the item.
    pub body: Vec<Inline>,
}

/// A `steps` container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Steps {
    /// Steps in order.
    pub steps: Vec<Step>,
}

impl Steps {
    /// Reads a `steps` container.
    #[must_use]
    pub fn parse(container: &Container) -> Read<Self> {
        let mut problems = Problems::default();
        let mut steps = Self::default();
        for (_, content) in items(container, &mut problems) {
            let (title, body) = titled(content);
            steps.steps.push(Step { title, body });
        }
        problems.done(steps)
    }
}

/// One tab: a `##` heading and the blocks under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Tab {
    /// Tab title.
    pub title: Vec<Inline>,
    /// Tab body.
    pub blocks: Vec<Block>,
}

/// A `tabs` container.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Tabs {
    /// Tabs in order.
    pub tabs: Vec<Tab>,
}

impl Tabs {
    /// Reads a `tabs` container. Blocks before the first `##` heading go in
    /// an untitled first tab.
    #[must_use]
    pub fn parse(container: &Container) -> Read<Self> {
        let mut problems = Problems::default();
        let mut tabs = Self::default();
        for block in container.sections.iter().flatten() {
            match block {
                Block::Heading {
                    level: 2, content, ..
                } => {
                    tabs.tabs.push(Tab {
                        title: content.clone(),
                        blocks: Vec::new(),
                    });
                }
                other => {
                    if let Some(tab) = tabs.tabs.last_mut() {
                        tab.blocks.push(other.clone());
                    } else {
                        problems.add(0, "content before the first `##` tab title");
                        tabs.tabs.push(Tab {
                            title: Vec::new(),
                            blocks: vec![other.clone()],
                        });
                    }
                }
            }
        }
        problems.done(tabs)
    }
}

/// The first paragraph of each list item in the container, numbered from 1.
fn items(container: &Container, problems: &mut Problems) -> Vec<(usize, Vec<Inline>)> {
    let mut out = Vec::new();
    for block in container.sections.iter().flatten() {
        match block {
            Block::List(list) => {
                for item in &list.items {
                    let n = out.len() + 1;
                    match item.blocks.first() {
                        Some(Block::Paragraph { content, .. }) => out.push((n, content.clone())),
                        _ => problems.add(n, "list item has no text"),
                    }
                }
            }
            _ => problems.add(0, "only a list belongs in this container"),
        }
    }
    out
}

/// Splits inline content at each `separator` found in its text runs.
fn split(content: Vec<Inline>, separator: &str) -> Vec<Vec<Inline>> {
    let mut parts = vec![Vec::new()];
    for inline in content {
        match inline {
            Inline::Text { text } => {
                let mut pieces = text.split(separator);
                if let (Some(first), Some(part)) = (pieces.next(), parts.last_mut()) {
                    part.push(Inline::Text {
                        text: first.to_owned(),
                    });
                }
                for piece in pieces {
                    parts.push(vec![Inline::Text {
                        text: piece.to_owned(),
                    }]);
                }
            }
            other => {
                if let Some(part) = parts.last_mut() {
                    part.push(other);
                }
            }
        }
    }
    parts.into_iter().map(trim).collect()
}

fn split_once(content: Vec<Inline>, separator: &str) -> Option<(Vec<Inline>, Vec<Inline>)> {
    let mut parts = split(content, separator).into_iter();
    let first = parts.next()?;
    let rest: Vec<Vec<Inline>> = parts.collect();
    (!rest.is_empty()).then(|| (first, join(rest, separator)))
}

fn join(parts: Vec<Vec<Inline>>, separator: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if i > 0 {
            out.push(Inline::Text {
                text: separator.to_owned(),
            });
        }
        out.extend(part);
    }
    crate::extras::merge_text(out)
}

/// A leading `**title**` and the rest, or the whole content as the title.
fn titled(mut content: Vec<Inline>) -> (Vec<Inline>, Vec<Inline>) {
    if matches!(content.first(), Some(Inline::Strong { .. })) {
        let Inline::Strong { content: title } = content.remove(0) else {
            return (content, Vec::new());
        };
        (title, trim(content))
    } else {
        (content, Vec::new())
    }
}

/// Drops empty text runs and trims whitespace at both ends.
fn trim(content: Vec<Inline>) -> Vec<Inline> {
    let mut content = crate::extras::merge_text(content);
    if let Some(Inline::Text { text }) = content.first_mut() {
        *text = text.trim_start().to_owned();
    }
    if let Some(Inline::Text { text }) = content.last_mut() {
        *text = text.trim_end().to_owned();
    }
    crate::extras::merge_text(content)
}
