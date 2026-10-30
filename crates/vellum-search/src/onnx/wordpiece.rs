// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The BERT `WordPiece` tokenizer (uncased), which sentence-embedding models
//! such as `MiniLM`, BGE and E5 use. Pure Rust, from a `vocab.txt`.

use std::collections::HashMap;

use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, TextAnalyzer, TokenStream,
    WhitespaceTokenizer,
};

use crate::error::{Result, SearchError};

/// The longest word `WordPiece` splits; longer ones become `[UNK]`.
const MAX_WORD_CHARS: usize = 100;

/// Token IDs for one text, with `[CLS]` and `[SEP]` around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoding {
    /// Token IDs.
    pub ids: Vec<i64>,
}

/// A `WordPiece` vocabulary.
#[derive(Debug, Clone)]
pub struct WordPiece {
    vocab: HashMap<String, i64>,
    unk: i64,
    cls: i64,
    sep: i64,
    /// The padding token ID.
    pub pad: i64,
}

impl WordPiece {
    /// Reads a `vocab.txt`: one token per line, the line number is the ID.
    ///
    /// # Errors
    /// When a special token (`[UNK]`, `[CLS]`, `[SEP]`, `[PAD]`) is missing.
    pub fn from_vocab(text: &str) -> Result<Self> {
        let vocab: HashMap<String, i64> = text
            .lines()
            .enumerate()
            .map(|(i, token)| {
                (
                    token.trim_end_matches('\r').to_owned(),
                    i64::try_from(i).unwrap_or(i64::MAX),
                )
            })
            .collect();
        let special = |name: &str| {
            vocab.get(name).copied().ok_or_else(|| {
                SearchError::Embedding(format!("vocabulary has no {name}"))
            })
        };
        Ok(Self {
            unk: special("[UNK]")?,
            cls: special("[CLS]")?,
            sep: special("[SEP]")?,
            pad: special("[PAD]")?,
            vocab,
        })
    }

    /// Encodes `text`, keeping at most `max_tokens` tokens including the
    /// two special ones.
    #[must_use]
    pub fn encode(&self, text: &str, max_tokens: usize) -> Encoding {
        let room = max_tokens.max(2) - 2;
        let mut ids = Vec::with_capacity(room.min(64) + 2);
        ids.push(self.cls);
        'words: for word in basic_words(text) {
            for id in self.word_pieces(&word) {
                if ids.len() > room {
                    break 'words;
                }
                ids.push(id);
            }
        }
        ids.push(self.sep);
        Encoding { ids }
    }

    /// Greedy longest-match-first split of one word.
    fn word_pieces(&self, word: &str) -> Vec<i64> {
        let chars: Vec<char> = word.chars().collect();
        if chars.len() > MAX_WORD_CHARS {
            return vec![self.unk];
        }
        let mut out = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut end = chars.len();
            let mut found = None;
            while start < end {
                let piece: String =
                    chars.get(start..end).unwrap_or_default().iter().collect();
                let key = if start > 0 {
                    format!("##{piece}")
                } else {
                    piece
                };
                if let Some(id) = self.vocab.get(&key) {
                    found = Some(*id);
                    break;
                }
                end -= 1;
            }
            match found {
                Some(id) => {
                    out.push(id);
                    start = end;
                }
                None => return vec![self.unk],
            }
        }
        out
    }
}

/// BERT's basic tokenisation: lower-case, strip accents, split on
/// whitespace, and make every punctuation mark and CJK ideograph its own word.
fn basic_words(text: &str) -> Vec<String> {
    let mut analyzer = TextAnalyzer::builder(WhitespaceTokenizer::default())
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build();
    let mut stream = analyzer.token_stream(text);
    let mut words = Vec::new();
    while stream.advance() {
        let mut current = String::new();
        for c in stream.token().text.chars() {
            if c.is_control() {
                continue;
            }
            if is_punctuation(c) || is_cjk(c) {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
                words.push(c.to_string());
            } else {
                current.push(c);
            }
        }
        if !current.is_empty() {
            words.push(current);
        }
    }
    words
}

const fn is_punctuation(c: char) -> bool {
    c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{2000}'..='\u{206F}' | '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FF0F}'
        )
}

const fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
        | '\u{20000}'..='\u{2A6DF}')
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOCAB: &str = "[PAD]\n[UNK]\n[CLS]\n[SEP]\nthe\nnorth\n##star\ngate\n\
        ##way\n,\ncafe\n日\n本";

    #[test]
    fn splits_words_into_pieces_with_specials() {
        let wp = WordPiece::from_vocab(VOCAB).unwrap();
        let enc = wp.encode("The Northstar gateway, Café xyz 日本", 64);
        assert_eq!(enc.ids, [2, 4, 5, 6, 7, 8, 9, 10, 1, 11, 12, 3]);
    }

    #[test]
    fn truncates_to_the_limit() {
        let wp = WordPiece::from_vocab(VOCAB).unwrap();
        let enc = wp.encode("the the the the the", 4);
        assert_eq!(enc.ids, [2, 4, 4, 3]);
    }

    #[test]
    fn missing_specials_are_an_error() {
        assert!(WordPiece::from_vocab("a\nb").is_err());
    }

    #[test]
    fn overlong_words_are_unknown() {
        let wp = WordPiece::from_vocab(VOCAB).unwrap();
        let enc = wp.encode(&"a".repeat(200), 8);
        assert_eq!(enc.ids, [2, 1, 3]);
    }
}
