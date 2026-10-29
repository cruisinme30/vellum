// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Issue descriptions built from text that came from outside.
//!
//! Mail, Slack messages, Sentry events and form answers are written by
//! people we do not trust. Pasting them into Markdown would let a sender
//! add links that hide their target, images that track whoever opens the
//! issue, mentions that notify people, or containers and data fences that
//! change how the page renders. [`Body`] instead builds the document tree
//! from plain text nodes and lets `vellum_format::serialize` escape them,
//! so the issue shows exactly the characters that were sent. Only parts
//! Vellum adds itself, such as a facts block or a "View in Slack" link,
//! become markup.

use vellum_format::{Attributes, Block, Container, Document, Inline, List, ListItem};

/// A description under construction.
#[derive(Debug, Clone, Default)]
pub struct Body {
    blocks: Vec<Block>,
}

impl Body {
    /// An empty description.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds untrusted text: blank lines split paragraphs and other line
    /// breaks are kept.
    pub fn text(&mut self, text: &str) -> &mut Self {
        self.blocks.extend(paragraphs(text));
        self
    }

    /// Adds untrusted text as a block quote.
    pub fn quote(&mut self, text: &str) -> &mut Self {
        let blocks = paragraphs(text);
        if !blocks.is_empty() {
            self.blocks.push(Block::BlockQuote { blocks, id: None });
        }
        self
    }

    /// Adds a facts block. Each value is untrusted; a value marked as code
    /// is shown in a code span.
    pub fn facts(&mut self, facts: &[Fact]) -> &mut Self {
        let items: Vec<ListItem> = facts
            .iter()
            .filter(|f| !f.value.trim().is_empty())
            .map(|f| {
                let label = one_line(&f.label);
                let value = one_line(&f.value);
                // One text node for label and value: escaping works per
                // node, so two adjacent nodes could join into `@name`.
                let content = if f.code {
                    vec![
                        text_node(&format!("{label}: ")),
                        Inline::Code { code: value },
                    ]
                } else {
                    vec![text_node(&format!("{label}: {value}"))]
                };
                ListItem {
                    task: None,
                    attrs: Attributes::default(),
                    blocks: vec![Block::Paragraph { content, id: None }],
                }
            })
            .collect();
        if !items.is_empty() {
            self.blocks.push(Block::Container(Container {
                name: "facts".to_owned(),
                arg: String::new(),
                attrs: Attributes::default(),
                id: None,
                sections: vec![vec![Block::List(List {
                    start: None,
                    tight: true,
                    items,
                })]],
            }));
        }
        self
    }

    /// Adds a link Vellum made itself, such as a provider permalink. A
    /// URL that is not `https` or `http` is shown as text instead.
    pub fn link(&mut self, label: &str, url: &str) -> &mut Self {
        let web = url::Url::parse(url)
            .is_ok_and(|u| matches!(u.scheme(), "https" | "http") && u.host().is_some());
        let content = if web {
            vec![Inline::Link {
                url: url.to_owned(),
                title: String::new(),
                content: vec![text_node(label)],
            }]
        } else {
            vec![text_node(&format!("{label}: {url}"))]
        };
        self.blocks.push(Block::Paragraph { content, id: None });
        self
    }

    /// The description as canonical Vellum Markdown.
    #[must_use]
    pub fn finish(&self) -> String {
        let document = Document {
            front_matter: None,
            blocks: self.blocks.clone(),
        };
        vellum_format::serialize(&document).trim_end().to_owned()
    }
}

/// One line of a facts block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    /// The label, such as `Level`.
    pub label: String,
    /// The value, untrusted.
    pub value: String,
    /// Shown in a code span.
    pub code: bool,
}

impl Fact {
    /// A plain fact.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            code: false,
        }
    }

    /// A fact shown as code, such as a function name.
    #[must_use]
    pub fn code(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            code: true,
            ..Self::new(label, value)
        }
    }
}

/// Word joiner: invisible, and stops `@name` being read as a mention.
const JOINER: char = '\u{2060}';

/// A text node. The serializer escapes `@name` as `\@name`, but the issue
/// core's mention scanner (`vellum_work::mention::mentions`) reads raw
/// Markdown and still notifies on it, so a word joiner after `@` keeps an
/// outsider's text from notifying people.
fn text_node(text: &str) -> Inline {
    let mut out = String::with_capacity(text.len());
    let mut prev: Option<char> = None;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        // The parser's mention boundary. An extra joiner is harmless; a
        // missing one lets the text mention someone.
        let word_start = !prev
            .is_some_and(|p| p.is_alphanumeric() || matches!(p, '_' | '-' | '.' | '/' | '@' | ':'));
        if c == '@' && word_start && chars.peek().is_some_and(char::is_ascii_alphabetic) {
            out.push(JOINER);
        }
        prev = Some(c);
    }
    Inline::Text { text: out }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Paragraphs of text nodes joined by hard breaks. Control characters
/// other than tabs are dropped and lines are trimmed, so indentation can
/// never turn a line into code.
fn paragraphs(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut content: Vec<Inline> = Vec::new();
    let clean: String = text
        .chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect();
    for line in clean.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !content.is_empty() {
                blocks.push(Block::Paragraph {
                    content: std::mem::take(&mut content),
                    id: None,
                });
            }
            continue;
        }
        if !content.is_empty() {
            content.push(Inline::HardBreak);
        }
        content.push(text_node(line));
    }
    if !content.is_empty() {
        blocks.push(Block::Paragraph { content, id: None });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inputs that become markup when pasted into Markdown.
    const HOSTILE: &[&str] = &[
        "[click here](https://evil.example/phish)",
        "![](https://evil.example/pixel.png)",
        "<https://evil.example> and <img src=x onerror=alert(1)>",
        "::: embed https://evil.example\nhi\n:::",
        "```chart\ntype: bar\n```",
        "~~~mermaid\ngraph TD\n~~~",
        "$$\n\\href{https://evil.example}{x}\n$$",
        "| a | b |\n|:-:|:-:|\n| 1 | 2 |",
        "- item\n* item\n1. item\n- [ ] task",
        "# Heading\nSetext\n===\n---",
        "> quoted\n    indented code",
        "@everyone please look, cc @alice",
        "+@priya and a+@priya (@priya) *@priya ~@priya \\@priya @@priya",
        "[[Secret page]] and [ref]\n\n[ref]: https://evil.example",
        "==mark== ^sup^ ~sub~ **bold** _em_ `code` \\ trailing \\",
        "line ^blk-00000000\n{#id .class}",
        "&lt;script&gt; &#x2F; &amp;",
        "---\ntitle: front matter\n---",
    ];

    fn reparsed(markdown: &str) -> Vec<Block> {
        let parsed = vellum_format::parse(markdown);
        assert_eq!(parsed.diagnostics, Vec::new(), "{markdown}");
        parsed.document.blocks
    }

    fn only_text(inlines: &[Inline]) {
        for inline in inlines {
            assert!(
                matches!(
                    inline,
                    Inline::Text { .. }
                        | Inline::HardBreak
                        | Inline::SoftBreak
                        | Inline::IssueKey { .. }
                        | Inline::Date { .. }
                ),
                "markup leaked: {inline:?}"
            );
        }
    }

    #[test]
    fn hostile_text_stays_text() {
        for input in HOSTILE {
            let mut body = Body::new();
            let _ = body.text(input).quote(input);
            let markdown = body.finish();
            // Nobody is notified by what an outsider wrote.
            assert_eq!(
                vellum_work::mention::mentions(&markdown),
                Vec::new(),
                "{markdown}"
            );
            let blocks = reparsed(&markdown);
            for block in &blocks {
                match block {
                    Block::Paragraph { content, .. } => only_text(content),
                    Block::BlockQuote { blocks, .. } => {
                        for inner in blocks {
                            let Block::Paragraph { content, .. } = inner else {
                                panic!("markup leaked in quote: {inner:?}");
                            };
                            only_text(content);
                        }
                    }
                    other => panic!("markup leaked from {input:?}: {other:?}"),
                }
            }
            // Every character survives, so the issue shows what was sent.
            let Some(Block::Paragraph { content, .. }) = blocks.first() else {
                panic!("no paragraph for {input:?}");
            };
            let shown = vellum_format::plain_text(content).replace(JOINER, "");
            let words: Vec<&str> = input.split_whitespace().collect();
            let seen: Vec<&str> = shown.split_whitespace().collect();
            assert_eq!(seen, words.get(..seen.len()).unwrap_or_default());
        }
    }

    #[test]
    fn facts_and_links_are_the_only_markup() {
        let mut body = Body::new();
        let _ = body
            .facts(&[
                Fact::new("Level", "error"),
                Fact::code("Culprit", "app.`render`](x)"),
                Fact::new("Project", "[web](https://evil.example)\n- injected"),
                Fact::new("Empty", " "),
            ])
            .link("Open in Sentry", "https://sentry.io/issues/1/")
            .link("Open", "javascript:alert(1)");
        let markdown = body.finish();
        let blocks = reparsed(&markdown);
        let Some(Block::Container(facts)) = blocks.first() else {
            panic!("no facts block: {markdown}");
        };
        let [section] = facts.sections.as_slice() else {
            panic!("facts have one section");
        };
        let [Block::List(list)] = section.as_slice() else {
            panic!("facts hold one list");
        };
        assert_eq!(list.items.len(), 3);
        for item in &list.items {
            for block in &item.blocks {
                let Block::Paragraph { content, .. } = block else {
                    panic!("markup leaked in facts: {block:?}");
                };
                for inline in content {
                    assert!(
                        matches!(inline, Inline::Text { .. } | Inline::Code { .. }),
                        "markup leaked in facts: {inline:?}"
                    );
                }
            }
        }
        assert!(matches!(
            blocks.get(1),
            Some(Block::Paragraph { content, .. })
                if matches!(content.as_slice(), [Inline::Link { url, .. }] if url == "https://sentry.io/issues/1/")
        ));
        let Some(Block::Paragraph { content, .. }) = blocks.get(2) else {
            panic!("no fallback paragraph");
        };
        only_text(content);
    }

    #[test]
    fn control_characters_and_indentation_are_dropped() {
        let mut body = Body::new();
        let _ = body.text("    four spaces\n\u{1b}[31mred\u{7}\n\n\n\tnext");
        assert_eq!(reparsed(&body.finish()).len(), 2);
        assert!(!body.finish().contains('\u{1b}'));
    }
}
