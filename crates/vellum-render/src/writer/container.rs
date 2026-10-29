// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `:::` containers.

use std::fmt::Write as _;

use vellum_format::blocks::{Data, Trend};
use vellum_format::{Block, Container, ContainerKind, Inline};

use super::Writer;
use super::inline::safe_url;
use crate::embed;
use crate::html::{self, push_attr};

const CALLOUT_ICONS: [(&str, &str, &str); 6] = [
    ("info", "ℹ", "Info"),
    ("warn", "⚠", "Warning"),
    ("good", "✓", "Good"),
    ("crit", "⛔", "Critical"),
    ("tip", "💡", "Tip"),
    ("note", "📝", "Note"),
];

impl Writer<'_> {
    pub(super) fn container(&mut self, c: &Container) {
        match c.kind() {
            ContainerKind::Callout => self.callout(c),
            ContainerKind::Details => {
                self.out.push_str("<details");
                self.attrs("vl-details", &c.attrs);
                if c.attrs.classes.iter().any(|k| k == "open") {
                    self.out.push_str(" open");
                }
                self.out.push_str("><summary>");
                html::text(
                    &mut self.out,
                    if c.arg.is_empty() { "Details" } else { &c.arg },
                );
                self.out.push_str("</summary>");
                self.sections(c);
                self.out.push_str("</details>");
            }
            ContainerKind::Columns => self.columns(c),
            ContainerKind::Tabs => self.tabs(c),
            ContainerKind::Toc => self.toc_placeholder(&c.attrs),
            ContainerKind::Stats => self.stats(c),
            ContainerKind::Facts => self.facts(c),
            ContainerKind::Progress => self.progress(c),
            ContainerKind::Steps => self.steps(c),
            ContainerKind::Cards => self.cards(c),
            ContainerKind::Link => self.link_card(c),
            ContainerKind::Embed => self.embed(c),
            ContainerKind::Pages => self.pages(c),
            ContainerKind::Unknown if c.name == "issues" => self.issues(c),
            ContainerKind::Unknown => {
                self.out.push_str("<div");
                self.attrs("vl-container", &c.attrs);
                push_attr(&mut self.out, "data-name", &c.name);
                self.out.push('>');
                self.sections(c);
                self.out.push_str("</div>");
            }
        }
    }

    fn sections(&mut self, c: &Container) {
        for section in &c.sections {
            self.blocks(section);
        }
    }

    fn callout(&mut self, c: &Container) {
        let (name, icon, label) = CALLOUT_ICONS
            .iter()
            .find(|(n, _, _)| *n == c.name)
            .copied()
            .unwrap_or(("note", "📝", "Note"));
        self.out.push_str("<aside role=\"note\"");
        push_attr(&mut self.out, "aria-label", label);
        self.attrs(&format!("vl-callout vl-callout-{name}"), &c.attrs);
        self.out
            .push_str("><span class=\"vl-callout-icon\" aria-hidden=\"true\">");
        self.out.push_str(icon);
        self.out.push_str("</span><div class=\"vl-callout-body\">");
        if !c.arg.is_empty() {
            self.out.push_str("<p class=\"vl-callout-title\"><strong>");
            html::text(&mut self.out, &c.arg);
            self.out.push_str("</strong></p>");
        }
        self.sections(c);
        self.out.push_str("</div></aside>");
    }

    fn columns(&mut self, c: &Container) {
        let n = c.sections.len();
        if !(2..=4).contains(&n) {
            self.warn(format!(
                "columns has {n} column(s); use 2 to 4, separated by `---`"
            ));
        }
        self.out.push_str("<div");
        self.attrs(&format!("vl-columns vl-cols-{}", n.clamp(1, 4)), &c.attrs);
        self.out.push('>');
        for section in &c.sections {
            self.out.push_str("<div class=\"vl-column\">");
            self.blocks(section);
            self.out.push_str("</div>");
        }
        self.out.push_str("</div>");
    }

    fn tabs(&mut self, c: &Container) {
        let Some(Data::Tabs(tabs)) = self.data(Data::from_container(c)) else {
            return self.sections(c);
        };
        if tabs.tabs.is_empty() {
            self.warn("tabs has no `## Tab` headings");
        }
        self.out.push_str("<div");
        self.attrs("vl-tabs", &c.attrs);
        self.out.push('>');
        for (i, tab) in tabs.tabs.iter().enumerate() {
            let text = vellum_format::plain_text(&tab.title);
            let id = self.unique_id(&format!("tab-{}", html::slug(&text)));
            self.out.push_str("<section class=\"vl-tab\"");
            push_attr(&mut self.out, "aria-labelledby", &id);
            self.out.push_str("><h3 class=\"vl-tab-title\"");
            push_attr(&mut self.out, "id", &id);
            self.out.push('>');
            if tab.title.is_empty() {
                let _ = write!(self.out, "Tab {}", i + 1);
            } else {
                self.inlines(&tab.title);
            }
            self.out.push_str("</h3>");
            self.blocks(&tab.blocks);
            self.out.push_str("</section>");
        }
        self.out.push_str("</div>");
    }

    fn stats(&mut self, c: &Container) {
        let Some(Data::Stats(stats)) = self.data(Data::from_container(c)) else {
            return self.sections(c);
        };
        self.out.push_str("<div");
        self.attrs("vl-stats", &c.attrs);
        self.out.push('>');
        for stat in &stats.items {
            self.out
                .push_str("<div class=\"vl-stat\"><div class=\"vl-stat-value\">");
            self.inlines(&stat.value);
            self.out.push_str("</div><div class=\"vl-stat-label\">");
            self.inlines(&stat.label);
            self.out.push_str("</div>");
            if let Some(delta) = &stat.delta {
                let (class, mark, said) = match stat.trend {
                    Some(Trend::Up) => ("vl-up", "▲ ", "up "),
                    Some(Trend::Down) => ("vl-down", "▼ ", "down "),
                    Some(Trend::Flat) => ("vl-flat", "▬ ", "flat "),
                    None => ("vl-flat", "", ""),
                };
                self.out.push_str("<div class=\"vl-stat-delta ");
                self.out.push_str(class);
                self.out.push_str("\"><span aria-hidden=\"true\">");
                self.out.push_str(mark);
                self.out.push_str("</span><span class=\"vl-sr-only\">");
                self.out.push_str(said);
                self.out.push_str("</span>");
                self.inlines(delta);
                self.out.push_str("</div>");
            }
            self.out.push_str("</div>");
        }
        self.out.push_str("</div>");
    }

    fn facts(&mut self, c: &Container) {
        let Some(Data::Facts(facts)) = self.data(Data::from_container(c)) else {
            return self.sections(c);
        };
        self.out.push_str("<dl");
        self.attrs("vl-facts", &c.attrs);
        self.out.push('>');
        for fact in &facts.items {
            self.out.push_str("<div><dt>");
            self.inlines(&fact.key);
            self.out.push_str("</dt><dd>");
            self.inlines(&fact.value);
            self.out.push_str("</dd></div>");
        }
        self.out.push_str("</dl>");
    }

    fn progress(&mut self, c: &Container) {
        let Some(Data::Progress(progress)) = self.data(Data::from_container(c)) else {
            return self.sections(c);
        };
        self.out.push_str("<div");
        self.attrs("vl-progress", &c.attrs);
        self.out.push('>');
        for item in &progress.items {
            let pct = html::num(item.percent.clamp(0.0, 100.0).round());
            let text = vellum_format::plain_text(&item.label);
            self.out.push_str("<span class=\"vl-progress-label\">");
            self.inlines(&item.label);
            self.out.push_str("</span><progress max=\"100\"");
            push_attr(&mut self.out, "value", &pct);
            push_attr(&mut self.out, "aria-label", &text);
            self.out.push('>');
            self.out.push_str(&pct);
            self.out
                .push_str("%</progress><span class=\"vl-progress-value\">");
            self.out.push_str(&pct);
            self.out.push_str("%</span>");
        }
        self.out.push_str("</div>");
    }

    fn steps(&mut self, c: &Container) {
        let Some(Data::Steps(steps)) = self.data(Data::from_container(c)) else {
            return self.sections(c);
        };
        self.out.push_str("<ol");
        self.attrs("vl-steps", &c.attrs);
        self.out.push('>');
        for step in &steps.steps {
            self.out.push_str("<li>");
            if step.body.is_empty() {
                self.inlines(&step.title);
            } else {
                self.out.push_str("<strong class=\"vl-step-title\">");
                self.inlines(&step.title);
                self.out.push_str("</strong> ");
                self.inlines(&step.body);
            }
            self.out.push_str("</li>");
        }
        self.out.push_str("</ol>");
    }

    fn cards(&mut self, c: &Container) {
        let Some(Data::Cards(cards)) = self.data(Data::from_container(c)) else {
            return self.sections(c);
        };
        self.out.push_str("<ul");
        self.attrs("vl-cards", &c.attrs);
        self.out.push('>');
        for card in &cards.cards {
            self.out.push_str("<li class=\"vl-card\">");
            if let Some(icon) = &card.icon {
                self.out
                    .push_str("<span class=\"vl-card-icon\" aria-hidden=\"true\">");
                html::text(&mut self.out, icon);
                self.out.push_str("</span>");
            }
            self.out.push_str("<strong class=\"vl-card-title\">");
            self.inlines(&card.title);
            self.out.push_str("</strong>");
            if !card.body.is_empty() {
                self.out.push_str("<p>");
                self.inlines(&card.body);
                self.out.push_str("</p>");
            }
            self.out.push_str("</li>");
        }
        self.out.push_str("</ul>");
    }

    fn link_card(&mut self, c: &Container) {
        let Some(href) = safe_url(self, &c.arg) else {
            self.warn(format!("link card URL {:?} is not allowed", c.arg));
            return self.sections(c);
        };
        let title: String = c
            .sections
            .iter()
            .flatten()
            .find_map(|b| match b {
                Block::Paragraph { content, .. } => Some(vellum_format::plain_text(content)),
                _ => None,
            })
            .unwrap_or_else(|| c.arg.clone());
        let host = href
            .split("://")
            .nth(1)
            .and_then(|r| r.split('/').next())
            .unwrap_or(&href)
            .to_owned();
        self.out.push_str("<a");
        push_attr(&mut self.out, "href", &href);
        self.attrs("vl-link-card", &c.attrs);
        self.out.push_str("><span class=\"vl-link-title\">");
        html::text(&mut self.out, &title);
        self.out.push_str("</span><span class=\"vl-link-host\">");
        html::text(&mut self.out, &host);
        self.out.push_str("</span></a>");
    }

    fn embed(&mut self, c: &Container) {
        match embed::frame_src(&c.arg) {
            Some((provider, src)) => {
                self.out.push_str("<figure");
                self.attrs("vl-figure", &c.attrs);
                self.out.push_str("><div class=\"vl-embed\"");
                push_attr(&mut self.out, "data-provider", provider.name());
                self.out.push_str("><iframe");
                push_attr(&mut self.out, "src", &src);
                push_attr(
                    &mut self.out,
                    "title",
                    &format!("{} embed", provider.name()),
                );
                self.out.push_str(" allow=\"fullscreen; picture-in-picture\" allowfullscreen referrerpolicy=\"no-referrer\" loading=\"lazy\"></iframe></div>");
                if c.sections.iter().any(|s| !s.is_empty()) {
                    self.out.push_str("<figcaption>");
                    self.sections(c);
                    self.out.push_str("</figcaption>");
                }
                self.out.push_str("</figure>");
            }
            None => {
                self.warn(format!(
                    "{:?} cannot be embedded (allowed: YouTube, Vimeo, Loom, Figma, Google Maps, CodePen); showing a link",
                    c.arg
                ));
                self.link_card(c);
            }
        }
    }

    fn pages(&mut self, c: &Container) {
        let mut links = Vec::new();
        for b in c.sections.iter().flatten() {
            let Block::List(list) = b else { continue };
            for item in &list.items {
                let Some(Block::Paragraph { content, .. }) = item.blocks.first() else {
                    continue;
                };
                for inline in content {
                    match inline {
                        Inline::WikiLink { target, label } => {
                            let r = self.resolver().wikilink(target);
                            let title = label
                                .clone()
                                .or_else(|| r.as_ref().map(|r| r.title.clone()))
                                .unwrap_or_else(|| target.clone());
                            links.push((r.map(|r| r.href), title));
                        }
                        Inline::Link { url, content, .. } => {
                            links.push((safe_url(self, url), vellum_format::plain_text(content)));
                        }
                        _ => {}
                    }
                }
            }
        }
        if links.is_empty() {
            links = self
                .resolver()
                .subpages()
                .into_iter()
                .map(|r| (Some(r.href), r.title))
                .collect();
        }
        if links.is_empty() {
            self.out.push_str("<div class=\"vl-live-placeholder\" data-kind=\"pages\">Subpages appear here.</div>");
            return;
        }
        self.out.push_str("<ul");
        self.attrs("vl-pages", &c.attrs);
        self.out.push('>');
        for (href, title) in links {
            self.out.push_str("<li>");
            match href.filter(|h| !h.is_empty()) {
                Some(h) => {
                    self.out.push_str("<a class=\"vl-page-card\"");
                    push_attr(&mut self.out, "href", &h);
                    self.out.push('>');
                    html::text(&mut self.out, &title);
                    self.out.push_str("</a>");
                }
                None => {
                    self.out
                        .push_str("<span class=\"vl-page-card vl-unresolved\">");
                    html::text(&mut self.out, &title);
                    self.out.push_str("</span>");
                }
            }
            self.out.push_str("</li>");
        }
        self.out.push_str("</ul>");
    }
}
