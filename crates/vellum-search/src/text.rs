// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Text analysis shared by every backend: tokens, query terms and snippets.
//!
//! Highlights are computed here, from the block text a backend returns, with
//! the same analyzer the tantivy index uses. So every backend highlights the
//! same words the same way, and a backend only has to rank.

use std::collections::HashSet;

use tantivy::tokenizer::{
    AsciiFoldingFilter, Language, LowerCaser, RemoveLongFilter,
    SimpleTokenizer, Stemmer, TextAnalyzer, TokenStream,
};

use crate::model::Fragment;

/// The analyzer name registered in the tantivy index.
pub const ANALYZER: &str = "vellum";

/// The longest token kept; longer ones are usually hashes or base64.
const MAX_TOKEN_LEN: usize = 40;

/// The longest snippet, in characters, before ellipses.
pub const SNIPPET_CHARS: usize = 200;

/// How many characters of context to keep before the first match.
const LEAD_CHARS: usize = 60;

/// The analyzer: words, lower-cased, folded to ASCII, English-stemmed.
#[must_use]
pub fn analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(MAX_TOKEN_LEN))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .filter(Stemmer::new(Language::English))
        .build()
}

/// A token with its byte range in the source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The analysed term.
    pub term: String,
    /// Start byte offset in the source.
    pub from: usize,
    /// End byte offset in the source.
    pub to: usize,
}

/// Analyses `text` into tokens.
#[must_use]
pub fn tokens(text: &str) -> Vec<Token> {
    let mut analyzer = analyzer();
    let mut stream = analyzer.token_stream(text);
    let mut out = Vec::new();
    while stream.advance() {
        let token = stream.token();
        out.push(Token {
            term: token.text.clone(),
            from: token.offset_from,
            to: token.offset_to,
        });
    }
    out
}

/// The distinct analysed terms of a query, in order of first appearance.
///
/// Query syntax (quotes, `+`, `-`, `field:`) is ignored here; terms after a
/// `-` are dropped so negated words are never highlighted.
#[must_use]
pub fn query_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for word in query.split_whitespace() {
        if word.starts_with('-') {
            continue;
        }
        for token in tokens(word) {
            if seen.insert(token.term.clone()) {
                out.push(token.term);
            }
        }
    }
    out
}

/// Builds a highlighted snippet of `text` for the analysed `terms`.
///
/// The window starts a little before the first match and holds at most
/// [`SNIPPET_CHARS`] characters, cut at word boundaries, with `…` marking
/// cuts. Without a match it is the start of the text.
#[must_use]
pub fn snippet(text: &str, terms: &[String]) -> Vec<Fragment> {
    let wanted: HashSet<&str> = terms.iter().map(String::as_str).collect();
    let matches: Vec<(usize, usize)> = tokens(text)
        .into_iter()
        .filter(|token| wanted.contains(token.term.as_str()))
        .map(|token| (token.from, token.to))
        .collect();

    let first = matches.first().map_or(0, |(from, _)| *from);
    let start = window_start(text, first);
    let end = window_end(text, start);

    let mut fragments = Vec::new();
    let mut cursor = start;
    if start > 0 {
        push(&mut fragments, "…", false);
    }
    for (from, to) in matches {
        if from < cursor || to > end {
            continue;
        }
        push(
            &mut fragments,
            text.get(cursor..from).unwrap_or_default(),
            false,
        );
        push(&mut fragments, text.get(from..to).unwrap_or_default(), true);
        cursor = to;
    }
    push(
        &mut fragments,
        text.get(cursor..end).unwrap_or_default(),
        false,
    );
    if end < text.len() {
        push(&mut fragments, "…", false);
    }
    fragments
}

/// Appends a fragment, merging with the previous one of the same kind.
fn push(fragments: &mut Vec<Fragment>, text: &str, hit: bool) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = fragments.last_mut()
        && last.hit == hit
    {
        last.text.push_str(text);
        return;
    }
    fragments.push(Fragment {
        text: text.to_owned(),
        hit,
    });
}

/// A byte offset at a word boundary up to `LEAD_CHARS` before `first`.
fn window_start(text: &str, first: usize) -> usize {
    let before = text.get(..first).unwrap_or_default();
    if before.chars().count() <= LEAD_CHARS {
        return 0;
    }
    let (cut, _) = before
        .char_indices()
        .rev()
        .nth(LEAD_CHARS - 1)
        .unwrap_or((0, ' '));
    // Move forward to the start of the next word.
    let rest = text.get(cut..).unwrap_or_default();
    rest.char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map_or(cut, |(i, c)| (cut + i + c.len_utf8()).min(first))
}

/// A byte offset at a word boundary at most `SNIPPET_CHARS` after `start`.
fn window_end(text: &str, start: usize) -> usize {
    let rest = text.get(start..).unwrap_or_default();
    match rest.char_indices().nth(SNIPPET_CHARS) {
        None => text.len(),
        Some((limit, _)) => {
            let head = rest.get(..limit).unwrap_or_default();
            let cut = head
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
                .map_or(limit, |(i, _)| i);
            start + cut
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(fragments: &[Fragment]) -> String {
        fragments
            .iter()
            .map(|f| {
                if f.hit {
                    format!("[{}]", f.text)
                } else {
                    f.text.clone()
                }
            })
            .collect()
    }

    #[test]
    fn terms_are_stemmed_folded_and_deduplicated() {
        assert_eq!(query_terms("Running runs Café -excluded"), ["run", "cafe"]);
    }

    #[test]
    fn snippets_mark_stemmed_matches() {
        let terms = query_terms("deploy");
        let snippet =
            snippet("We deployed the gateway; deploys are weekly.", &terms);
        assert_eq!(
            render(&snippet),
            "We [deployed] the gateway; [deploys] are weekly."
        );
    }

    #[test]
    fn long_text_is_windowed_around_the_first_match() {
        let text =
            format!("{} needle {}", "hay ".repeat(50), "straw ".repeat(80));
        let rendered = render(&snippet(&text, &query_terms("needle")));
        assert!(rendered.starts_with('…'));
        assert!(rendered.ends_with('…'));
        assert!(rendered.contains("[needle]"));
        assert!(rendered.chars().count() <= SNIPPET_CHARS + 2);
    }

    #[test]
    fn no_match_shows_the_start() {
        let rendered = render(&snippet("short text", &query_terms("absent")));
        assert_eq!(rendered, "short text");
    }

    #[test]
    fn multibyte_text_never_splits_a_character() {
        let text = "é".repeat(400);
        let rendered = render(&snippet(&text, &[]));
        assert!(rendered.ends_with('…'));
    }
}
