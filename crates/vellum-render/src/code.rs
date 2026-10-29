// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Syntax highlighting for code fences.
//!
//! Tokens are classified with Sublime Text grammars (syntect) and written as
//! `<span class="vl-tok-…">` with one of ten classes. The colours come from
//! the theme's `--vl-syntax-*` tokens, so code re-themes with the page
//! (R-TH-3) and the rendered HTML stays theme-independent.

use std::sync::LazyLock;

use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxSet};

use crate::html;

/// Fences larger than this are shown without highlighting, so one huge
/// paste cannot blow the render budget.
const MAX_BYTES: usize = 256 * 1024;
const MAX_LINES: usize = 5_000;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

/// Loads the grammars now instead of on the first code block; servers call
/// it at start-up so the first render meets its latency target.
pub fn warm_up() {
    let _ = LazyLock::force(&SYNTAXES);
}

/// How a code block is laid out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeOptions {
    /// Show line numbers.
    pub numbered: bool,
    /// 1-based line numbers to highlight.
    pub highlighted: Vec<usize>,
}

impl CodeOptions {
    /// True when lines must be wrapped individually.
    const fn per_line(&self) -> bool {
        self.numbered || !self.highlighted.is_empty()
    }
}

/// Parses an `hl="3-4,7"` line list.
#[must_use]
pub fn parse_line_list(spec: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        if let (Ok(a), Ok(b)) = (a.trim().parse::<usize>(), b.trim().parse::<usize>())
            && a <= b
            && b - a <= MAX_LINES
        {
            out.extend(a..=b);
        }
    }
    out
}

/// The token class for a scope stack, from the innermost scope out.
fn class_for(stack: &ScopeStack) -> Option<&'static str> {
    for scope in stack.as_slice().iter().rev() {
        if let Some(class) = class_for_scope(*scope) {
            return Some(class);
        }
    }
    None
}

fn class_for_scope(scope: Scope) -> Option<&'static str> {
    let name = scope.build_string();
    let s = name.as_str();
    let starts = |p: &str| s == p || s.starts_with(&format!("{p}."));
    let class = if starts("punctuation.definition.string")
        || starts("string")
        || starts("constant.character.escape")
    {
        "string"
    } else if starts("punctuation.definition.comment") || starts("comment") {
        "comment"
    } else if starts("constant.numeric") || starts("constant.language") || starts("constant.other")
    {
        "number"
    } else if starts("entity.name.function")
        || starts("support.function")
        || starts("variable.function")
    {
        "function"
    } else if starts("entity.name.tag") {
        "tag"
    } else if starts("entity.other.attribute-name") {
        "attribute"
    } else if starts("storage.type")
        || starts("entity.name.type")
        || starts("entity.name.class")
        || starts("entity.name.struct")
        || starts("entity.name.enum")
        || starts("entity.name.trait")
        || starts("support.type")
        || starts("support.class")
    {
        "type"
    } else if starts("keyword") || starts("storage") {
        "keyword"
    } else if starts("variable") {
        "variable"
    } else if starts("punctuation") {
        "punctuation"
    } else {
        return None;
    };
    Some(class)
}

/// Writes highlighted code (the inside of `<code>`) for `lang`; unknown
/// languages and oversized input are escaped plainly.
pub fn highlight(out: &mut String, lang: &str, code: &str, options: &CodeOptions) {
    let syntax = (!lang.is_empty() && code.len() <= MAX_BYTES && code.lines().count() <= MAX_LINES)
        .then(|| SYNTAXES.find_syntax_by_token(lang))
        .flatten();
    let mut state = syntax.map(ParseState::new);
    let mut stack = ScopeStack::new();
    let highlighted: std::collections::HashSet<usize> =
        options.highlighted.iter().copied().collect();
    let body = code.strip_suffix('\n').unwrap_or(code);
    for (i, line) in body.split('\n').enumerate() {
        let number = i + 1;
        if options.per_line() {
            out.push_str("<span class=\"vl-line");
            if highlighted.contains(&number) {
                out.push_str(" vl-line-hl");
            }
            out.push_str("\" data-line=\"");
            out.push_str(&number.to_string());
            out.push_str("\">");
        }
        let with_newline = format!("{line}\n");
        let ops = state
            .as_mut()
            .and_then(|s| s.parse_line(&with_newline, &SYNTAXES).ok());
        match ops {
            Some(ops) => {
                let mut pos = 0;
                for (at, op) in ops {
                    let at = at.min(line.len());
                    if at > pos {
                        token(
                            out,
                            class_for(&stack),
                            line.get(pos..at).unwrap_or_default(),
                        );
                        pos = at;
                    }
                    let _ = stack.apply(&op);
                }
                if pos < line.len() {
                    token(out, class_for(&stack), line.get(pos..).unwrap_or_default());
                }
            }
            None => html::text(out, line),
        }
        if options.per_line() {
            out.push_str("</span>");
        } else {
            out.push('\n');
        }
    }
}

fn token(out: &mut String, class: Option<&str>, text: &str) {
    if text.is_empty() {
        return;
    }
    match class {
        Some(c) => {
            out.push_str("<span class=\"vl-tok-");
            out.push_str(c);
            out.push_str("\">");
            html::text(out, text);
            out.push_str("</span>");
        }
        None => html::text(out, text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(lang: &str, code: &str) -> String {
        let mut out = String::new();
        highlight(&mut out, lang, code, &CodeOptions::default());
        out
    }

    #[test]
    fn classifies_rust_tokens() {
        let out = render("rust", "fn main() { let x = \"hi\"; // note\n}\n");
        assert!(
            out.contains("<span class=\"vl-tok-keyword\">fn</span>")
                || out.contains("<span class=\"vl-tok-type\">fn</span>"),
            "{out}"
        );
        assert!(
            out.contains("<span class=\"vl-tok-function\">main</span>"),
            "{out}"
        );
        assert!(out.contains("vl-tok-string"), "{out}");
        assert!(out.contains("vl-tok-comment"), "{out}");
    }

    #[test]
    fn escapes_everything_and_falls_back_for_unknown_languages() {
        let out = render("no-such-lang", "<script>alert(1)</script>\n");
        assert_eq!(out, "&lt;script&gt;alert(1)&lt;/script&gt;\n");
        let out = render("html", "<b onclick=\"x\">&amp;</b>\n");
        assert!(!out.contains("<b"), "{out}");
    }

    #[test]
    fn numbers_and_highlights_lines() {
        let mut out = String::new();
        let options = CodeOptions {
            numbered: true,
            highlighted: parse_line_list("2"),
        };
        highlight(&mut out, "", "a\nb\n", &options);
        assert_eq!(
            out,
            "<span class=\"vl-line\" data-line=\"1\">a</span><span class=\"vl-line vl-line-hl\" data-line=\"2\">b</span>"
        );
    }

    #[test]
    fn parses_line_lists() {
        assert_eq!(parse_line_list("3-4, 7,x,9-8"), vec![3, 4, 7]);
    }
}
