// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Inline content.

use vellum_format::Inline;

use super::Writer;
use crate::html::{self, push_attr};

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `2026-11-24` → `Nov 24, 2026`.
fn display_date(iso: &str) -> Option<String> {
    let mut parts = iso.splitn(3, '-');
    let y = parts.next()?;
    let m: usize = parts.next()?.parse().ok()?;
    let d: u32 = parts.next()?.parse().ok()?;
    let month = MONTHS.get(m.checked_sub(1)?)?;
    Some(format!("{month} {d}, {y}"))
}

/// A URL that may appear in `href`: http(s), mailto, tel, or relative.
/// `vellum://` URLs are mapped by the resolver.
pub(super) fn safe_url(w: &Writer<'_>, url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.chars().any(|c| c.is_control() || html::is_invisible(c)) {
        return None;
    }
    if url.starts_with("vellum://") {
        return w.resolver().href(url);
    }
    // Same-page links follow the prefixed ids.
    if let Some(fragment) = url.strip_prefix('#') {
        return html::is_user_id(fragment).then(|| format!("#{}{fragment}", super::ID_PREFIX));
    }
    let scheme_end = url.find(':');
    let path_start = url.find(['/', '?', '#']);
    match (scheme_end, path_start) {
        (Some(c), p) if p.is_none_or(|p| c < p) => {
            let scheme = url.get(..c)?.to_ascii_lowercase();
            matches!(scheme.as_str(), "http" | "https" | "mailto" | "tel").then(|| url.to_owned())
        }
        _ => Some(url.to_owned()),
    }
}

/// An image source: https, http, or relative (assets next to the file).
pub(super) fn safe_image(url: &str) -> Option<String> {
    let url = url.trim();
    let lower = url.to_ascii_lowercase();
    let scheme = lower
        .find(':')
        .filter(|c| lower.find(['/', '?', '#']).is_none_or(|p| *c < p));
    match scheme {
        Some(_) => {
            (lower.starts_with("https://") || lower.starts_with("http://")).then(|| url.to_owned())
        }
        None => (!url.is_empty() && !url.chars().any(char::is_control)).then(|| url.to_owned()),
    }
}

impl Writer<'_> {
    pub fn inlines(&mut self, inlines: &[Inline]) {
        for i in inlines {
            self.inline(i);
        }
    }

    fn wrap(&mut self, tag: &str, content: &[Inline]) {
        self.out.push('<');
        self.out.push_str(tag);
        self.out.push('>');
        self.inlines(content);
        self.out.push_str("</");
        self.out.push_str(tag);
        self.out.push('>');
    }

    fn inline(&mut self, inline: &Inline) {
        match inline {
            Inline::Text { text } => html::text(&mut self.out, text),
            Inline::Code { code } => {
                self.out.push_str("<code>");
                html::text(&mut self.out, code);
                self.out.push_str("</code>");
            }
            Inline::SoftBreak => self.out.push('\n'),
            Inline::HardBreak => self.out.push_str("<br>"),
            Inline::Emphasis { content } => self.wrap("em", content),
            Inline::Strong { content } => self.wrap("strong", content),
            Inline::Strikethrough { content } => self.wrap("s", content),
            Inline::Highlight { content } => self.wrap("mark", content),
            Inline::Subscript { content } => self.wrap("sub", content),
            Inline::Superscript { content } => self.wrap("sup", content),
            Inline::Link {
                url,
                title,
                content,
            } => match safe_url(self, url) {
                Some(href) => {
                    self.out.push_str("<a");
                    push_attr(&mut self.out, "href", &href);
                    if !title.is_empty() {
                        push_attr(&mut self.out, "title", title);
                    }
                    self.out.push('>');
                    self.inlines(content);
                    self.out.push_str("</a>");
                }
                None => {
                    self.warn(format!("link to {url:?} was removed: only http, https, mailto, tel and relative links are allowed"));
                    self.wrap("span", content);
                }
            },
            Inline::Image { url, title, alt } => match safe_image(url) {
                Some(src) => {
                    self.out.push_str("<img");
                    push_attr(&mut self.out, "src", &src);
                    push_attr(&mut self.out, "alt", &vellum_format::plain_text(alt));
                    if !title.is_empty() {
                        push_attr(&mut self.out, "title", title);
                    }
                    self.out.push_str(" loading=\"lazy\" decoding=\"async\">");
                }
                None => {
                    self.warn(format!(
                        "image {url:?} was removed: only https and relative images are allowed"
                    ));
                    html::text(&mut self.out, &vellum_format::plain_text(alt));
                }
            },
            Inline::WikiLink { target, label } => self.wikilink(target, label.as_deref()),
            Inline::Mention { handle } => {
                let name = self.resolver().mention(handle);
                self.out.push_str("<span class=\"vl-mention\"");
                push_attr(&mut self.out, "data-handle", handle);
                self.out.push_str(">@");
                html::text(&mut self.out, name.as_deref().unwrap_or(handle));
                self.out.push_str("</span>");
            }
            Inline::IssueKey { key } => match self.resolver().issue(key) {
                Some(issue) => {
                    self.out.push_str("<a class=\"vl-issue\"");
                    push_attr(&mut self.out, "href", &issue.href);
                    push_attr(&mut self.out, "data-issue", key);
                    self.out.push('>');
                    html::text(&mut self.out, key);
                    self.out.push_str(" · ");
                    html::text(&mut self.out, &issue.title);
                    if !issue.state.is_empty() {
                        self.out.push_str(" · ");
                        html::text(&mut self.out, &issue.state);
                    }
                    self.out.push_str("</a>");
                }
                None => html::text(&mut self.out, key),
            },
            Inline::Date { date } => {
                if date == "today" {
                    // Stays relative: the client renders the reader's today.
                    self.out
                        .push_str("<time class=\"vl-date\" data-relative=\"today\">Today</time>");
                } else {
                    self.out.push_str("<time class=\"vl-date\"");
                    push_attr(&mut self.out, "datetime", date);
                    self.out.push('>');
                    html::text(
                        &mut self.out,
                        &display_date(date).unwrap_or_else(|| date.clone()),
                    );
                    self.out.push_str("</time>");
                }
            }
        }
    }

    fn wikilink(&mut self, target: &str, label: Option<&str>) {
        match self.resolver().wikilink(target) {
            Some(r) if !r.href.is_empty() => {
                self.out.push_str("<a class=\"vl-wikilink\"");
                push_attr(&mut self.out, "href", &r.href);
                self.out.push('>');
                html::text(&mut self.out, label.unwrap_or(&r.title));
                self.out.push_str("</a>");
            }
            _ => {
                self.out
                    .push_str("<span class=\"vl-wikilink vl-unresolved\"");
                push_attr(&mut self.out, "data-target", target);
                push_attr(&mut self.out, "title", "No page with this title yet");
                self.out.push('>');
                html::text(&mut self.out, label.unwrap_or(target));
                self.out.push_str("</span>");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_dates() {
        assert_eq!(display_date("2026-11-24").as_deref(), Some("Nov 24, 2026"));
        assert_eq!(display_date("2026-13-01"), None);
    }

    #[test]
    fn image_sources() {
        assert_eq!(
            safe_image("assets/arch.png").as_deref(),
            Some("assets/arch.png")
        );
        assert_eq!(
            safe_image("https://northstar.dev/a.png").as_deref(),
            Some("https://northstar.dev/a.png")
        );
        assert_eq!(safe_image("data:image/svg+xml,<svg onload=x>"), None);
        assert_eq!(safe_image("javascript:alert(1)"), None);
    }
}
