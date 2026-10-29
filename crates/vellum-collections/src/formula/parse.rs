// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Lexer and precedence-climbing parser.

use super::FormulaError;
use super::ast::{BinOp, Expr, Func, Node, Span, UnOp};

/// Longest formula source accepted, in characters.
pub const MAX_SOURCE_CHARS: usize = 4096;

/// Deepest nesting of parentheses, calls and operators accepted.
pub const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Number(f64),
    Text(String),
    Ident(String),
    Op(BinOp),
    Minus,
    LParen,
    RParen,
    Comma,
    Not,
    True,
    False,
    End,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    span: Span,
}

fn lex(src: &str) -> Result<Vec<Token>, FormulaError> {
    let chars: Vec<char> = src.chars().collect();
    if chars.len() > MAX_SOURCE_CHARS {
        return Err(FormulaError::new(
            format!("formula is longer than {MAX_SOURCE_CHARS} characters"),
            Span::new(MAX_SOURCE_CHARS, chars.len()),
        ));
    }
    let mut out = Vec::new();
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied();
    while let Some(c) = at(i) {
        let start = i;
        let single = |tok| (tok, 1);
        let (tok, len) = match c {
            c if c.is_whitespace() => {
                i += 1;
                continue;
            }
            '(' => single(Tok::LParen),
            ')' => single(Tok::RParen),
            ',' => single(Tok::Comma),
            '+' => single(Tok::Op(BinOp::Add)),
            '-' | '−' => single(Tok::Minus),
            '*' | '×' => single(Tok::Op(BinOp::Mul)),
            '/' | '÷' => single(Tok::Op(BinOp::Div)),
            '%' => single(Tok::Op(BinOp::Rem)),
            '≠' => single(Tok::Op(BinOp::Ne)),
            '≤' => single(Tok::Op(BinOp::Le)),
            '≥' => single(Tok::Op(BinOp::Ge)),
            '=' if at(i + 1) == Some('=') => (Tok::Op(BinOp::Eq), 2),
            '!' if at(i + 1) == Some('=') => (Tok::Op(BinOp::Ne), 2),
            '<' if at(i + 1) == Some('=') => (Tok::Op(BinOp::Le), 2),
            '>' if at(i + 1) == Some('=') => (Tok::Op(BinOp::Ge), 2),
            '<' => single(Tok::Op(BinOp::Lt)),
            '>' => single(Tok::Op(BinOp::Gt)),
            '=' => {
                return Err(FormulaError::new(
                    "use == to compare",
                    Span::new(start, start + 1),
                ));
            }
            '"' | '\'' => lex_string(&chars, i, c)?,
            c if c.is_ascii_digit()
                || (c == '.' && at(i + 1).is_some_and(|d| d.is_ascii_digit())) =>
            {
                lex_number(&chars, i)?
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut j = i;
                while at(j).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    j += 1;
                }
                let word: String = chars.get(i..j).unwrap_or_default().iter().collect();
                let tok = match word.to_ascii_lowercase().as_str() {
                    "and" => Tok::Op(BinOp::And),
                    "or" => Tok::Op(BinOp::Or),
                    "not" => Tok::Not,
                    "true" => Tok::True,
                    "false" => Tok::False,
                    _ => Tok::Ident(word),
                };
                (tok, j - i)
            }
            other => {
                return Err(FormulaError::new(
                    format!("unexpected character {other:?}"),
                    Span::new(start, start + 1),
                ));
            }
        };
        i += len;
        out.push(Token {
            tok,
            span: Span::new(start, i),
        });
    }
    out.push(Token {
        tok: Tok::End,
        span: Span::new(chars.len(), chars.len()),
    });
    Ok(out)
}

fn lex_string(chars: &[char], start: usize, quote: char) -> Result<(Tok, usize), FormulaError> {
    let mut text = String::new();
    let mut i = start + 1;
    loop {
        match chars.get(i).copied() {
            None => {
                return Err(FormulaError::new(
                    "text is missing its closing quote",
                    Span::new(start, chars.len()),
                ));
            }
            Some(c) if c == quote => break,
            Some('\\') => {
                let esc = chars.get(i + 1).copied();
                text.push(match esc {
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some(c @ ('"' | '\'' | '\\')) => c,
                    _ => {
                        return Err(FormulaError::new(
                            "unknown escape; use \\\" \\' \\\\ \\n or \\t",
                            Span::new(i, (i + 2).min(chars.len())),
                        ));
                    }
                });
                i += 2;
            }
            Some(c) => {
                text.push(c);
                i += 1;
            }
        }
    }
    Ok((Tok::Text(text), i + 1 - start))
}

fn lex_number(chars: &[char], start: usize) -> Result<(Tok, usize), FormulaError> {
    let mut i = start;
    let digit = |i: usize| chars.get(i).is_some_and(char::is_ascii_digit);
    while digit(i) {
        i += 1;
    }
    if chars.get(i) == Some(&'.') && digit(i + 1) {
        i += 1;
        while digit(i) {
            i += 1;
        }
    }
    let text: String = chars.get(start..i).unwrap_or_default().iter().collect();
    let span = Span::new(start, i);
    match text.parse::<f64>() {
        Ok(n) if n.is_finite() => Ok((Tok::Number(n), i - start)),
        _ => Err(FormulaError::new("number is out of range", span)),
    }
}

/// Parses formula source into an unresolved tree.
///
/// # Errors
///
/// Returns the first syntax error with its position.
pub fn parse(src: &str) -> Result<Node, FormulaError> {
    let tokens = lex(src)?;
    let mut p = Parser { tokens, pos: 0 };
    if matches!(p.peek().tok, Tok::End) {
        return Err(FormulaError::new("formula is empty", p.peek().span));
    }
    let node = p.expr(0, 0)?;
    let next = p.peek();
    if !matches!(next.tok, Tok::End) {
        return Err(FormulaError::new(
            "expected an operator or the end of the formula",
            next.span,
        ));
    }
    Ok(node)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Token {
        self.tokens
            .get(self.pos)
            .or_else(|| self.tokens.last())
            .cloned()
            .unwrap_or_else(|| Token {
                tok: Tok::End,
                span: Span::default(),
            })
    }

    fn bump(&mut self) -> Token {
        let t = self.peek();
        self.pos += 1;
        t
    }

    fn expect(&mut self, want: &Tok, what: &str) -> Result<Token, FormulaError> {
        let t = self.peek();
        if &t.tok == want {
            Ok(self.bump())
        } else {
            Err(FormulaError::new(format!("expected {what}"), t.span))
        }
    }

    fn expr(&mut self, min_prec: u8, depth: usize) -> Result<Node, FormulaError> {
        if depth > MAX_DEPTH {
            return Err(FormulaError::new(
                format!("formula nests deeper than {MAX_DEPTH} levels"),
                self.peek().span,
            ));
        }
        let mut lhs = self.prefix(depth)?;
        loop {
            let t = self.peek();
            let op = match t.tok {
                Tok::Op(op) => op,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            let prec = op.precedence();
            if prec <= min_prec {
                break;
            }
            let _ = self.bump();
            let rhs = self.expr(prec, depth + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Node::new(Expr::Binary(op, Box::new(lhs), Box::new(rhs)), span);
        }
        Ok(lhs)
    }

    fn prefix(&mut self, depth: usize) -> Result<Node, FormulaError> {
        let t = self.bump();
        let span = t.span;
        Ok(match t.tok {
            Tok::Number(n) => Node::new(Expr::Number(n), span),
            Tok::Text(s) => Node::new(Expr::Text(s), span),
            Tok::True => Node::new(Expr::Bool(true), span),
            Tok::False => Node::new(Expr::Bool(false), span),
            Tok::Minus => {
                let e = self.expr(6, depth + 1)?;
                Node::new(Expr::Unary(UnOp::Neg, Box::new(e.clone())), span.to(e.span))
            }
            Tok::Not => {
                let e = self.expr(2, depth + 1)?;
                Node::new(Expr::Unary(UnOp::Not, Box::new(e.clone())), span.to(e.span))
            }
            Tok::LParen => {
                let e = self.expr(0, depth + 1)?;
                let close = self.expect(&Tok::RParen, "a closing )")?;
                Node::new(e.expr, span.to(close.span))
            }
            Tok::Ident(name) => {
                if matches!(self.peek().tok, Tok::LParen) {
                    self.call(&name, span, depth)?
                } else {
                    Node::new(Expr::Prop(name), span)
                }
            }
            Tok::End => return Err(FormulaError::new("formula ends too early", span)),
            Tok::Op(_) | Tok::RParen | Tok::Comma => {
                return Err(FormulaError::new("expected a value", span));
            }
        })
    }

    fn call(&mut self, name: &str, name_span: Span, depth: usize) -> Result<Node, FormulaError> {
        let _ = self.bump(); // (
        if name.eq_ignore_ascii_case("prop") {
            let arg = self.bump();
            let Tok::Text(prop) = arg.tok else {
                return Err(FormulaError::new(
                    "prop() takes a property name in quotes",
                    arg.span,
                ));
            };
            let close = self.expect(&Tok::RParen, "a closing )")?;
            return Ok(Node::new(Expr::Prop(prop), name_span.to(close.span)));
        }
        let Some(func) = Func::from_name(name) else {
            return Err(FormulaError::new(
                format!("unknown function {name}()"),
                name_span,
            ));
        };
        let mut args = Vec::new();
        if !matches!(self.peek().tok, Tok::RParen) {
            loop {
                args.push(self.expr(0, depth + 1)?);
                if matches!(self.peek().tok, Tok::Comma) {
                    let _ = self.bump();
                } else {
                    break;
                }
            }
        }
        let close = self.expect(&Tok::RParen, "a comma or a closing )")?;
        let span = name_span.to(close.span);
        let (lo, hi) = func.arity();
        if args.len() < lo || hi.is_some_and(|hi| args.len() > hi) {
            let want = match hi {
                Some(hi) if hi == lo => format!("{lo}"),
                Some(hi) => format!("{lo} to {hi}"),
                None => format!("at least {lo}"),
            };
            return Err(FormulaError::new(
                format!(
                    "{}() takes {want} argument{}, got {}",
                    func.name(),
                    if want == "1" { "" } else { "s" },
                    args.len()
                ),
                span,
            ));
        }
        Ok(Node::new(Expr::Call(func, args), span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(src: &str) -> String {
        parse(src).map_or_else(|e| format!("error: {e}"), |n| n.to_string())
    }

    #[test]
    fn precedence_and_printing() {
        assert_eq!(show("1 + 2 * 3"), "1 + 2 * 3");
        assert_eq!(show("(1 + 2) * 3"), "(1 + 2) * 3");
        assert_eq!(show("1 - (2 - 3)"), "1 - (2 - 3)");
        assert_eq!(show("1 - 2 - 3"), "1 - 2 - 3");
        assert_eq!(
            show("not a == b and c"),
            "not prop(\"a\") == prop(\"b\") and prop(\"c\")"
        );
        assert_eq!(show("-x * 2"), "-prop(\"x\") * 2");
        assert_eq!(show("2 × 3 ÷ 4 − 1"), "2 * 3 / 4 - 1");
        assert_eq!(
            show("round(prop(\"Estimate\") * (1 - Progress), 1)"),
            "round(prop(\"Estimate\") * (1 - prop(\"Progress\")), 1)"
        );
        assert_eq!(show("IF(x, 'a', \"b\")"), "if(prop(\"x\"), \"a\", \"b\")");
    }

    #[test]
    fn errors_point_at_the_problem() {
        let e = parse("1 + ").unwrap_err();
        assert_eq!(e.span.start, 4);
        let e = parse("foo(1)").unwrap_err();
        assert!(e.message.contains("unknown function foo"));
        assert_eq!((e.span.start, e.span.end), (0, 3));
        let e = parse("round()").unwrap_err();
        assert!(e.message.contains("1 to 2 arguments"));
        let e = parse("\"open").unwrap_err();
        assert!(e.message.contains("closing quote"));
        let e = parse("a = b").unwrap_err();
        assert!(e.message.contains("=="));
        let e = parse("(1").unwrap_err();
        assert!(e.message.contains(')'));
        let e = parse("1 2").unwrap_err();
        assert_eq!(e.span.start, 2);
        assert!(parse("").is_err());
        assert!(parse(&"(".repeat(200)).is_err());
    }

    #[test]
    fn spans_count_characters_not_bytes() {
        let e = parse("\"é\" + ?").unwrap_err();
        assert_eq!(e.span.start, 6);
    }
}
