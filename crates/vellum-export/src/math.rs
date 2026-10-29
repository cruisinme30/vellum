// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! `TeX` maths to Typst maths (print PDF) and to OMML (Word equations).
//!
//! Pages store maths as `TeX` between `$$` lines. The print backend needs
//! Typst maths syntax and Word needs Office Math Markup, so formulas are
//! parsed once into a small tree and written out both ways. The parser
//! covers what documents use: fractions, roots, scripts, Greek letters,
//! operators and relations, big operators, accents, fonts, `\text`,
//! `\left…\right`, and the matrix and `cases` environments. Anything else
//! is reported as unsupported and the formula is shown as its source.

use std::fmt::Write as _;

/// A parsed formula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A sequence of atoms.
    Row(Vec<Self>),
    /// A single-letter variable, shown in italics.
    Var(char),
    /// Digits and a decimal point.
    Num(String),
    /// A symbol, operator or relation: Typst name and Unicode character.
    Sym(&'static str, char),
    /// An upright function name such as `sin`.
    Func(String),
    /// `\text{…}` and friends.
    Text(String),
    /// `\frac{a}{b}`
    Frac(Box<Self>, Box<Self>),
    /// `\sqrt[n]{a}`
    Root(Option<Box<Self>>, Box<Self>),
    /// Base with optional subscript and superscript.
    Script {
        /// The base.
        base: Box<Self>,
        /// `_` part.
        sub: Option<Box<Self>>,
        /// `^` part.
        sup: Option<Box<Self>>,
    },
    /// An accent over a body: Typst function name and OMML character.
    Accent(&'static str, char, Box<Self>),
    /// A font style: `bold`, `upright`, `cal`, `bb`.
    Font(&'static str, Box<Self>),
    /// `\left( … \right)` with the delimiters as written (`.` for none).
    Fenced(String, String, Box<Self>),
    /// A matrix: delimiter pair and rows of cells.
    Matrix(&'static str, &'static str, Vec<Vec<Self>>),
    /// `\begin{cases}` rows of cells.
    Cases(Vec<Vec<Self>>),
}

/// Why a formula could not be converted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported maths: {0}")]
pub struct MathError(pub String);

/// Parses `TeX` maths.
///
/// # Errors
///
/// [`MathError`] for unknown commands, unbalanced braces or formulas
/// longer than 10 000 characters.
pub fn parse(tex: &str) -> Result<Node, MathError> {
    if tex.len() > 10_000 {
        return Err(MathError("formula is longer than 10 000 characters".into()));
    }
    let tokens = lex(tex)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
        depth: 0,
    };
    let node = parser.row(&[])?;
    if let Some(tok) = parser.peek() {
        return Err(MathError(format!("unexpected `{}`", tok.text())));
    }
    Ok(node)
}

/// Converts `TeX` maths to Typst maths markup.
///
/// # Errors
///
/// [`MathError`] when [`parse`] fails.
pub fn to_typst(tex: &str) -> Result<String, MathError> {
    let mut out = String::new();
    typst(&parse(tex)?, &mut out);
    Ok(out.trim().to_owned())
}

/// Converts `TeX` maths to an OMML `<m:oMath>` element for Word.
///
/// # Errors
///
/// [`MathError`] when [`parse`] fails.
pub fn to_omml(tex: &str) -> Result<String, MathError> {
    let mut out = String::from("<m:oMath>");
    omml(&parse(tex)?, &mut out);
    out.push_str("</m:oMath>");
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Cmd(String),
    Char(char),
    Open,
    Close,
    Sub,
    Sup,
    Amp,
    NewRow,
    Space,
}

impl Tok {
    fn text(&self) -> String {
        match self {
            Self::Cmd(c) => format!("\\{c}"),
            Self::Char(c) => c.to_string(),
            Self::Open => "{".into(),
            Self::Close => "}".into(),
            Self::Sub => "_".into(),
            Self::Sup => "^".into(),
            Self::Amp => "&".into(),
            Self::NewRow => "\\\\".into(),
            Self::Space => " ".into(),
        }
    }
}

fn lex(tex: &str) -> Result<Vec<Tok>, MathError> {
    let mut tokens = Vec::new();
    let mut chars = tex.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek().copied() {
                Some('\\') => {
                    let _ = chars.next();
                    tokens.push(Tok::NewRow);
                }
                Some(n) if n.is_ascii_alphabetic() => {
                    let mut name = String::new();
                    while let Some(&a) = chars.peek() {
                        if a.is_ascii_alphabetic() {
                            name.push(a);
                            let _ = chars.next();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Tok::Cmd(name));
                }
                Some(n) => {
                    let _ = chars.next();
                    tokens.push(Tok::Cmd(n.to_string()));
                }
                None => return Err(MathError("trailing backslash".into())),
            },
            '{' => tokens.push(Tok::Open),
            '}' => tokens.push(Tok::Close),
            '_' => tokens.push(Tok::Sub),
            '^' => tokens.push(Tok::Sup),
            '&' => tokens.push(Tok::Amp),
            c if c.is_whitespace() => {
                if tokens.last() != Some(&Tok::Space) {
                    tokens.push(Tok::Space);
                }
            }
            c => tokens.push(Tok::Char(c)),
        }
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Tok],
    pos: usize,
    depth: usize,
}

impl Parser<'_> {
    /// The next token that is not a space.
    fn peek(&self) -> Option<&Tok> {
        self.tokens
            .get(self.pos..)?
            .iter()
            .find(|tok| **tok != Tok::Space)
    }

    /// Consumes and returns the next token that is not a space.
    fn next(&mut self) -> Option<Tok> {
        while self.tokens.get(self.pos) == Some(&Tok::Space) {
            self.pos += 1;
        }
        self.raw_next()
    }

    /// Consumes the next token, spaces included.
    fn raw_next(&mut self) -> Option<Tok> {
        let tok = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        tok
    }

    /// Atoms until a closing brace, `&`, `\\`, `\right` or `\end` (not
    /// consumed) or the end of input.
    fn row(&mut self, stops: &[&str]) -> Result<Node, MathError> {
        self.depth += 1;
        if self.depth > 100 {
            return Err(MathError("formula is nested too deeply".into()));
        }
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None | Some(Tok::Close | Tok::Amp | Tok::NewRow) => break,
                Some(Tok::Cmd(c)) if c == "right" || c == "end" => break,
                Some(Tok::Cmd(c)) if stops.contains(&c.as_str()) => break,
                Some(Tok::Sub | Tok::Sup) => {
                    let base = items.pop().unwrap_or(Node::Row(Vec::new()));
                    items.push(self.scripts(base)?);
                }
                _ => {
                    let atom = self.atom()?;
                    items.push(atom);
                }
            }
        }
        self.depth -= 1;
        Ok(if items.len() == 1 {
            items.pop().unwrap_or(Node::Row(Vec::new()))
        } else {
            Node::Row(items)
        })
    }

    fn scripts(&mut self, base: Node) -> Result<Node, MathError> {
        let (mut sub, mut sup) = match base {
            Node::Script { base, sub, sup } => {
                return self.scripts_onto(*base, sub, sup);
            }
            _ => (None, None),
        };
        while let Some(tok @ (Tok::Sub | Tok::Sup)) = self.peek().cloned() {
            let _ = self.next();
            let arg = Box::new(self.argument()?);
            if tok == Tok::Sub {
                sub = Some(arg);
            } else {
                sup = Some(arg);
            }
        }
        Ok(Node::Script {
            base: Box::new(base),
            sub,
            sup,
        })
    }

    fn scripts_onto(
        &mut self,
        base: Node,
        mut sub: Option<Box<Node>>,
        mut sup: Option<Box<Node>>,
    ) -> Result<Node, MathError> {
        while let Some(tok @ (Tok::Sub | Tok::Sup)) = self.peek().cloned() {
            let _ = self.next();
            let arg = Box::new(self.argument()?);
            if tok == Tok::Sub {
                sub = Some(arg);
            } else {
                sup = Some(arg);
            }
        }
        Ok(Node::Script {
            base: Box::new(base),
            sub,
            sup,
        })
    }

    /// A braced group or a single atom.
    fn argument(&mut self) -> Result<Node, MathError> {
        if self.peek() == Some(&Tok::Open) {
            let _ = self.next();
            let inner = self.row(&[])?;
            self.expect_close()?;
            Ok(inner)
        } else {
            self.atom()
        }
    }

    fn expect_close(&mut self) -> Result<(), MathError> {
        match self.next() {
            Some(Tok::Close) => Ok(()),
            _ => Err(MathError("missing `}`".into())),
        }
    }

    /// Raw text inside braces, for `\text{…}` and environment names.
    fn braced_text(&mut self) -> Result<String, MathError> {
        if self.next() != Some(Tok::Open) {
            return Err(MathError("expected `{`".into()));
        }
        let mut text = String::new();
        let mut level = 0usize;
        loop {
            match self.raw_next() {
                Some(Tok::Close) if level == 0 => return Ok(text),
                Some(Tok::Close) => {
                    level -= 1;
                    text.push('}');
                }
                Some(Tok::Open) => {
                    level += 1;
                    text.push('{');
                }
                Some(Tok::Cmd(c)) if c == " " || c == "," => text.push(' '),
                Some(Tok::Cmd(c)) if c.len() == 1 => text.push_str(&c),
                Some(tok) => text.push_str(&tok.text()),
                None => return Err(MathError("missing `}`".into())),
            }
        }
    }

    fn atom(&mut self) -> Result<Node, MathError> {
        match self.next() {
            Some(Tok::Open) => {
                let inner = self.row(&[])?;
                self.expect_close()?;
                Ok(inner)
            }
            Some(Tok::Char(c)) => Ok(self.char_atom(c)),
            Some(Tok::Cmd(name)) => self.command(&name),
            Some(tok) => Err(MathError(format!("unexpected `{}`", tok.text()))),
            None => Err(MathError("formula ends early".into())),
        }
    }

    fn char_atom(&mut self, c: char) -> Node {
        if c.is_ascii_digit() || c == '.' {
            let mut text = c.to_string();
            while let Some(Tok::Char(d)) = self.tokens.get(self.pos) {
                if d.is_ascii_digit() || *d == '.' {
                    text.push(*d);
                    self.pos += 1;
                } else {
                    break;
                }
            }
            return Node::Num(text);
        }
        if c.is_alphabetic() {
            return Node::Var(c);
        }
        let sym = match c {
            '+' => ("plus", '+'),
            '-' => ("minus", '−'),
            '=' => ("eq", '='),
            '<' => ("lt", '<'),
            '>' => ("gt", '>'),
            '(' => ("paren.l", '('),
            ')' => ("paren.r", ')'),
            '[' => ("bracket.l", '['),
            ']' => ("bracket.r", ']'),
            '|' => ("bar.v", '|'),
            '/' => ("slash", '/'),
            ',' => ("comma", ','),
            ';' => ("semi", ';'),
            ':' => ("colon", ':'),
            '!' => ("excl", '!'),
            '\'' => ("prime", '′'),
            '*' => ("ast.op", '∗'),
            other => ("", other),
        };
        Node::Sym(sym.0, sym.1)
    }

    #[allow(clippy::too_many_lines)] // one table of TeX commands
    fn command(&mut self, name: &str) -> Result<Node, MathError> {
        if let Some((typst, unicode)) = symbol(name) {
            return Ok(Node::Sym(typst, unicode));
        }
        Ok(match name {
            "frac" | "dfrac" | "tfrac" => {
                let num = self.argument()?;
                let den = self.argument()?;
                Node::Frac(Box::new(num), Box::new(den))
            }
            "sqrt" => {
                let index = if self.peek() == Some(&Tok::Char('[')) {
                    let _ = self.next();
                    let mut items = Vec::new();
                    while self.peek() != Some(&Tok::Char(']')) {
                        if self.peek().is_none() {
                            return Err(MathError("missing `]`".into()));
                        }
                        items.push(self.atom()?);
                    }
                    let _ = self.next();
                    Some(Box::new(Node::Row(items)))
                } else {
                    None
                };
                Node::Root(index, Box::new(self.argument()?))
            }
            "text" | "textrm" | "mathrm" if name != "mathrm" => Node::Text(self.braced_text()?),
            "mathrm" | "operatorname" => Node::Func(self.braced_text()?),
            "mathbf" | "boldsymbol" | "bm" => Node::Font("bold", Box::new(self.argument()?)),
            "mathit" => Node::Font("italic", Box::new(self.argument()?)),
            "mathcal" => Node::Font("cal", Box::new(self.argument()?)),
            "mathbb" => Node::Font("bb", Box::new(self.argument()?)),
            "hat" | "widehat" => Node::Accent("hat", '\u{302}', Box::new(self.argument()?)),
            "bar" | "overline" => Node::Accent("overline", '\u{305}', Box::new(self.argument()?)),
            "vec" => Node::Accent("arrow", '\u{20D7}', Box::new(self.argument()?)),
            "dot" => Node::Accent("dot", '\u{307}', Box::new(self.argument()?)),
            "ddot" => Node::Accent("dot.double", '\u{308}', Box::new(self.argument()?)),
            "tilde" | "widetilde" => Node::Accent("tilde", '\u{303}', Box::new(self.argument()?)),
            "sin" | "cos" | "tan" | "cot" | "sec" | "csc" | "log" | "ln" | "exp" | "lim"
            | "max" | "min" | "sup" | "inf" | "det" | "arg" | "deg" | "dim" | "gcd" | "ker"
            | "Pr" | "sinh" | "cosh" | "tanh" | "arcsin" | "arccos" | "arctan" | "mod" => {
                Node::Func(name.to_owned())
            }
            "left" => {
                let open = self.delimiter()?;
                let body = self.row(&[])?;
                match self.next() {
                    Some(Tok::Cmd(c)) if c == "right" => {}
                    _ => return Err(MathError("`\\left` without `\\right`".into())),
                }
                let close = self.delimiter()?;
                Node::Fenced(open, close, Box::new(body))
            }
            "begin" => self.environment()?,
            "," | ";" | ":" | " " | "quad" | "qquad" | "!" => Node::Text(" ".into()),
            "{" => Node::Sym("brace.l", '{'),
            "}" => Node::Sym("brace.r", '}'),
            "%" => Node::Sym("percent", '%'),
            "$" => Node::Sym("dollar", '$'),
            "_" => Node::Sym("underscore", '_'),
            "#" => Node::Sym("hash", '#'),
            "&" => Node::Sym("amp", '&'),
            "displaystyle" | "textstyle" | "limits" | "nolimits" => Node::Row(Vec::new()),
            other => return Err(MathError(format!("`\\{other}`"))),
        })
    }

    fn delimiter(&mut self) -> Result<String, MathError> {
        match self.next() {
            Some(Tok::Char(c)) => Ok(c.to_string()),
            Some(Tok::Cmd(c)) => Ok(match c.as_str() {
                "{" | "lbrace" => "{".into(),
                "}" | "rbrace" => "}".into(),
                "langle" => "⟨".into(),
                "rangle" => "⟩".into(),
                "|" | "Vert" => "‖".into(),
                "lvert" | "rvert" => "|".into(),
                "lfloor" => "⌊".into(),
                "rfloor" => "⌋".into(),
                "lceil" => "⌈".into(),
                "rceil" => "⌉".into(),
                other => return Err(MathError(format!("delimiter `\\{other}`"))),
            }),
            _ => Err(MathError("missing delimiter".into())),
        }
    }

    fn environment(&mut self) -> Result<Node, MathError> {
        let env = self.braced_text()?;
        let (open, close) = match env.as_str() {
            "matrix" | "cases" | "aligned" | "align" | "align*" | "gathered" => ("", ""),
            "pmatrix" => ("(", ")"),
            "bmatrix" => ("[", "]"),
            "Bmatrix" => ("{", "}"),
            "vmatrix" => ("|", "|"),
            "Vmatrix" => ("‖", "‖"),
            other => return Err(MathError(format!("environment `{other}`"))),
        };
        let mut rows = Vec::new();
        let mut row = Vec::new();
        loop {
            row.push(self.row(&[])?);
            match self.next() {
                Some(Tok::Amp) => {}
                Some(Tok::NewRow) => rows.push(std::mem::take(&mut row)),
                Some(Tok::Cmd(c)) if c == "end" => {
                    let _ = self.braced_text()?;
                    break;
                }
                _ => return Err(MathError(format!("`\\begin{{{env}}}` is not closed"))),
            }
        }
        if row.iter().any(|cell| cell != &Node::Row(Vec::new())) {
            rows.push(row);
        }
        Ok(if env == "cases" {
            Node::Cases(rows)
        } else {
            Node::Matrix(open, close, rows)
        })
    }
}

/// Symbols: `TeX` command → (Typst name, Unicode).
#[allow(clippy::too_many_lines)] // a lookup table
fn symbol(name: &str) -> Option<(&'static str, char)> {
    Some(match name {
        "alpha" => ("alpha", 'α'),
        "beta" => ("beta", 'β'),
        "gamma" => ("gamma", 'γ'),
        "delta" => ("delta", 'δ'),
        "epsilon" => ("epsilon.alt", 'ϵ'),
        "varepsilon" => ("epsilon", 'ε'),
        "zeta" => ("zeta", 'ζ'),
        "eta" => ("eta", 'η'),
        "theta" => ("theta", 'θ'),
        "vartheta" => ("theta.alt", 'ϑ'),
        "iota" => ("iota", 'ι'),
        "kappa" => ("kappa", 'κ'),
        "lambda" => ("lambda", 'λ'),
        "mu" => ("mu", 'μ'),
        "nu" => ("nu", 'ν'),
        "xi" => ("xi", 'ξ'),
        "pi" => ("pi", 'π'),
        "varpi" => ("pi.alt", 'ϖ'),
        "rho" => ("rho", 'ρ'),
        "sigma" => ("sigma", 'σ'),
        "tau" => ("tau", 'τ'),
        "upsilon" => ("upsilon", 'υ'),
        "phi" => ("phi.alt", 'ϕ'),
        "varphi" => ("phi", 'φ'),
        "chi" => ("chi", 'χ'),
        "psi" => ("psi", 'ψ'),
        "omega" => ("omega", 'ω'),
        "Gamma" => ("Gamma", 'Γ'),
        "Delta" => ("Delta", 'Δ'),
        "Theta" => ("Theta", 'Θ'),
        "Lambda" => ("Lambda", 'Λ'),
        "Xi" => ("Xi", 'Ξ'),
        "Pi" => ("Pi", 'Π'),
        "Sigma" => ("Sigma", 'Σ'),
        "Phi" => ("Phi", 'Φ'),
        "Psi" => ("Psi", 'Ψ'),
        "Omega" => ("Omega", 'Ω'),
        "cdot" => ("dot.op", '⋅'),
        "times" => ("times", '×'),
        "div" => ("div", '÷'),
        "pm" => ("plus.minus", '±'),
        "mp" => ("minus.plus", '∓'),
        "ast" => ("ast.op", '∗'),
        "circ" => ("compose", '∘'),
        "le" | "leq" => ("lt.eq", '≤'),
        "ge" | "geq" => ("gt.eq", '≥'),
        "ne" | "neq" => ("eq.not", '≠'),
        "approx" => ("approx", '≈'),
        "equiv" => ("equiv", '≡'),
        "sim" => ("tilde.op", '∼'),
        "propto" => ("prop", '∝'),
        "ll" => ("lt.double", '≪'),
        "gg" => ("gt.double", '≫'),
        "in" => ("in", '∈'),
        "notin" => ("in.not", '∉'),
        "subset" => ("subset", '⊂'),
        "subseteq" => ("subset.eq", '⊆'),
        "supset" => ("supset", '⊃'),
        "cup" => ("union", '∪'),
        "cap" => ("sect", '∩'),
        "emptyset" | "varnothing" => ("emptyset", '∅'),
        "forall" => ("forall", '∀'),
        "exists" => ("exists", '∃'),
        "neg" | "lnot" => ("not", '¬'),
        "land" | "wedge" => ("and", '∧'),
        "lor" | "vee" => ("or", '∨'),
        "to" | "rightarrow" => ("arrow.r", '→'),
        "leftarrow" | "gets" => ("arrow.l", '←'),
        "Rightarrow" | "implies" => ("arrow.r.double", '⇒'),
        "Leftarrow" => ("arrow.l.double", '⇐'),
        "leftrightarrow" => ("arrow.l.r", '↔'),
        "Leftrightarrow" | "iff" => ("arrow.l.r.double", '⇔'),
        "mapsto" => ("arrow.r.bar", '↦'),
        "infty" => ("infinity", '∞'),
        "partial" => ("partial", '∂'),
        "nabla" => ("nabla", '∇'),
        "ldots" | "dots" => ("dots.h", '…'),
        "cdots" => ("dots.h.c", '⋯'),
        "vdots" => ("dots.v", '⋮'),
        "ddots" => ("dots.down", '⋱'),
        "prime" => ("prime", '′'),
        "degree" => ("degree", '°'),
        "sum" => ("sum", '∑'),
        "prod" => ("product", '∏'),
        "int" => ("integral", '∫'),
        "iint" => ("integral.double", '∬'),
        "oint" => ("integral.cont", '∮'),
        "bigcup" => ("union.big", '⋃'),
        "bigcap" => ("sect.big", '⋂'),
        "mid" => ("divides", '∣'),
        "parallel" => ("parallel", '∥'),
        "perp" => ("perp", '⊥'),
        "angle" => ("angle", '∠'),
        "langle" => ("angle.l", '⟨'),
        "rangle" => ("angle.r", '⟩'),
        "lfloor" => ("floor.l", '⌊'),
        "rfloor" => ("floor.r", '⌋'),
        "lceil" => ("ceil.l", '⌈'),
        "rceil" => ("ceil.r", '⌉'),
        "hbar" => ("planck.reduce", 'ℏ'),
        "ell" => ("ell", 'ℓ'),
        _ => return None,
    })
}

const fn is_big_operator(node: &Node) -> Option<char> {
    match node {
        Node::Sym(_, c @ ('∑' | '∏' | '∫' | '∬' | '∮' | '⋃' | '⋂')) => Some(*c),
        _ => None,
    }
}

/// Writes Typst maths. Every atom is separated by a space so that letter
/// runs stay single-letter variables rather than becoming identifiers.
fn typst(node: &Node, out: &mut String) {
    match node {
        Node::Row(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                typst(item, out);
            }
        }
        Node::Var(c) => out.push(*c),
        Node::Num(n) => out.push_str(n),
        Node::Sym("", c) => {
            let _ = write!(out, "\"{}\"", escape_typst_string(&c.to_string()));
        }
        Node::Sym(name, _) => out.push_str(name),
        Node::Func(name) => {
            let _ = write!(out, "op(\"{}\")", escape_typst_string(name));
        }
        Node::Text(text) => {
            let _ = write!(out, "\"{}\"", escape_typst_string(text));
        }
        Node::Frac(num, den) => {
            out.push_str("frac(");
            typst(num, out);
            out.push_str(", ");
            typst(den, out);
            out.push(')');
        }
        Node::Root(None, body) => {
            out.push_str("sqrt(");
            typst(body, out);
            out.push(')');
        }
        Node::Root(Some(index), body) => {
            out.push_str("root(");
            typst(index, out);
            out.push_str(", ");
            typst(body, out);
            out.push(')');
        }
        Node::Script { base, sub, sup } => {
            out.push_str("attach(");
            typst(base, out);
            if let Some(sub) = sub {
                out.push_str(", b: ");
                typst(sub, out);
            }
            if let Some(sup) = sup {
                out.push_str(", t: ");
                typst(sup, out);
            }
            out.push(')');
        }
        Node::Accent(name, _, body) | Node::Font(name, body) => {
            let _ = write!(out, "{name}(");
            typst(body, out);
            out.push(')');
        }
        Node::Fenced(open, close, body) => {
            out.push_str("lr(");
            typst_delim(open, out);
            out.push(' ');
            typst(body, out);
            out.push(' ');
            typst_delim(close, out);
            out.push(')');
        }
        Node::Matrix(open, close, rows) => {
            let delim = match (*open, *close) {
                ("", "") => "#none".to_owned(),
                ("‖", _) => "\"‖\"".to_owned(),
                (o, _) => format!("\"{o}\""),
            };
            let _ = write!(out, "mat(delim: {delim}, ");
            typst_rows(rows, out);
            out.push(')');
        }
        Node::Cases(rows) => {
            out.push_str("cases(");
            for (i, row) in rows.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                for (j, cell) in row.iter().enumerate() {
                    if j > 0 {
                        out.push_str(" & ");
                    }
                    typst(cell, out);
                }
            }
            out.push(')');
        }
    }
}

fn typst_rows(rows: &[Vec<Node>], out: &mut String) {
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            out.push_str("; ");
        }
        for (j, cell) in row.iter().enumerate() {
            if j > 0 {
                out.push_str(", ");
            }
            typst(cell, out);
        }
    }
}

fn typst_delim(delim: &str, out: &mut String) {
    match delim {
        "." => out.push_str("\"\""),
        "(" | ")" | "[" | "]" | "|" => out.push_str(delim),
        "{" => out.push('{'),
        "}" => out.push('}'),
        other => {
            let _ = write!(out, "\"{}\"", escape_typst_string(other));
        }
    }
}

fn escape_typst_string(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

fn omml_run(text: &str, plain: bool, out: &mut String) {
    out.push_str("<m:r>");
    if plain {
        out.push_str("<m:rPr><m:sty m:val=\"p\"/></m:rPr>");
    }
    let _ = write!(
        out,
        "<m:t xml:space=\"preserve\">{}</m:t></m:r>",
        xml_escape(text)
    );
}

fn omml_e(node: &Node, out: &mut String) {
    out.push_str("<m:e>");
    omml(node, out);
    out.push_str("</m:e>");
}

/// Writes OMML (Office Math Markup Language) for Word.
#[allow(clippy::too_many_lines)] // one arm per node kind
fn omml(node: &Node, out: &mut String) {
    match node {
        Node::Row(items) => {
            let mut i = 0;
            while let Some(item) = items.get(i) {
                // A big operator takes the next atom as its body.
                if let Some(op) = big_op_with_limits(item) {
                    let body = items.get(i + 1);
                    omml_nary(op, body, out);
                    i += if body.is_some() { 2 } else { 1 };
                    continue;
                }
                omml(item, out);
                i += 1;
            }
        }
        Node::Var(c) | Node::Sym(_, c) => omml_run(&c.to_string(), false, out),
        Node::Num(n) => omml_run(n, false, out),
        Node::Func(name) => {
            out.push_str("<m:func><m:fName>");
            omml_run(name, true, out);
            out.push_str("</m:fName><m:e/></m:func>");
        }
        Node::Text(text) => omml_run(text, true, out),
        Node::Frac(num, den) => {
            out.push_str("<m:f><m:num>");
            omml(num, out);
            out.push_str("</m:num><m:den>");
            omml(den, out);
            out.push_str("</m:den></m:f>");
        }
        Node::Root(index, body) => {
            out.push_str("<m:rad>");
            match index {
                None => out.push_str("<m:radPr><m:degHide m:val=\"1\"/></m:radPr><m:deg/>"),
                Some(index) => {
                    out.push_str("<m:deg>");
                    omml(index, out);
                    out.push_str("</m:deg>");
                }
            }
            omml_e(body, out);
            out.push_str("</m:rad>");
        }
        Node::Script { base, sub, sup } => {
            if let Some(op) = is_big_operator(base) {
                omml_nary((op, sub.as_deref(), sup.as_deref()), None, out);
                return;
            }
            let (tag, parts): (&str, Vec<(&str, &Node)>) = match (sub, sup) {
                (Some(s), Some(p)) => ("sSubSup", vec![("sub", s), ("sup", p)]),
                (Some(s), None) => ("sSub", vec![("sub", s)]),
                (None, Some(p)) => ("sSup", vec![("sup", p)]),
                (None, None) => {
                    omml(base, out);
                    return;
                }
            };
            let _ = write!(out, "<m:{tag}>");
            omml_e(base, out);
            for (part, node) in parts {
                let _ = write!(out, "<m:{part}>");
                omml(node, out);
                let _ = write!(out, "</m:{part}>");
            }
            let _ = write!(out, "</m:{tag}>");
        }
        Node::Accent("overline", _, body) => {
            out.push_str("<m:bar><m:barPr><m:pos m:val=\"top\"/></m:barPr>");
            omml_e(body, out);
            out.push_str("</m:bar>");
        }
        Node::Accent(_, chr, body) => {
            let _ = write!(out, "<m:acc><m:accPr><m:chr m:val=\"{chr}\"/></m:accPr>");
            omml_e(body, out);
            out.push_str("</m:acc>");
        }
        Node::Font(style, body) => {
            // OMML styles runs; nested structure keeps the default style.
            let sty = match *style {
                "bold" => Some("b"),
                "italic" => Some("i"),
                _ => None,
            };
            let scr = match *style {
                "cal" => Some("script"),
                "bb" => Some("double-struck"),
                _ => None,
            };
            let mut inner = String::new();
            omml(body, &mut inner);
            let mut props = String::new();
            if let Some(scr) = scr {
                let _ = write!(props, "<m:scr m:val=\"{scr}\"/>");
            }
            if let Some(sty) = sty {
                let _ = write!(props, "<m:sty m:val=\"{sty}\"/>");
            }
            out.push_str(&inner.replace("<m:r>", &format!("<m:r><m:rPr>{props}</m:rPr>")));
        }
        Node::Fenced(open, close, body) => {
            let open = if open == "." { "" } else { open };
            let close = if close == "." { "" } else { close };
            let _ = write!(
                out,
                "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/></m:dPr>",
                xml_escape(open),
                xml_escape(close)
            );
            omml_e(body, out);
            out.push_str("</m:d>");
        }
        Node::Matrix(open, close, rows) => {
            let delimited = !open.is_empty();
            if delimited {
                let _ = write!(
                    out,
                    "<m:d><m:dPr><m:begChr m:val=\"{}\"/><m:endChr m:val=\"{}\"/></m:dPr><m:e>",
                    xml_escape(open),
                    xml_escape(close)
                );
            }
            omml_matrix(rows, out);
            if delimited {
                out.push_str("</m:e></m:d>");
            }
        }
        Node::Cases(rows) => {
            out.push_str("<m:d><m:dPr><m:begChr m:val=\"{\"/><m:endChr m:val=\"\"/></m:dPr><m:e>");
            omml_matrix(rows, out);
            out.push_str("</m:e></m:d>");
        }
    }
}

type BigOp<'a> = (char, Option<&'a Node>, Option<&'a Node>);

fn big_op_with_limits(node: &Node) -> Option<BigOp<'_>> {
    match node {
        Node::Script { base, sub, sup } => {
            is_big_operator(base).map(|op| (op, sub.as_deref(), sup.as_deref()))
        }
        other => is_big_operator(other).map(|op| (op, None, None)),
    }
}

fn omml_nary((op, sub, sup): BigOp<'_>, body: Option<&Node>, out: &mut String) {
    let _ = write!(out, "<m:nary><m:naryPr><m:chr m:val=\"{op}\"/>");
    if sub.is_none() {
        out.push_str("<m:subHide m:val=\"1\"/>");
    }
    if sup.is_none() {
        out.push_str("<m:supHide m:val=\"1\"/>");
    }
    out.push_str("</m:naryPr><m:sub>");
    if let Some(sub) = sub {
        omml(sub, out);
    }
    out.push_str("</m:sub><m:sup>");
    if let Some(sup) = sup {
        omml(sup, out);
    }
    out.push_str("</m:sup><m:e>");
    if let Some(body) = body {
        omml(body, out);
    }
    out.push_str("</m:e></m:nary>");
}

fn omml_matrix(rows: &[Vec<Node>], out: &mut String) {
    out.push_str("<m:m>");
    for row in rows {
        out.push_str("<m:mr>");
        for cell in row {
            omml_e(cell, out);
        }
        out.push_str("</m:mr>");
    }
    out.push_str("</m:m>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_scripts_and_symbols_convert_to_typst() -> Result<(), MathError> {
        assert_eq!(
            to_typst(r"F_1 = 2 \cdot \frac{P \cdot R}{P + R}")?,
            "attach(F, b: 1) eq 2 dot.op frac(P dot.op R, P plus R)"
        );
        assert_eq!(to_typst(r"\sqrt[3]{x}")?, "root(3, x)");
        assert_eq!(to_typst(r"\text{TP rate}")?, "\"TP rate\"");
        Ok(())
    }

    #[test]
    fn environments_and_fences_convert() -> Result<(), MathError> {
        assert_eq!(
            to_typst(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}")?,
            "mat(delim: \"(\", a, b; c, d)"
        );
        assert_eq!(to_typst(r"\left( x \right.")?, "lr(( x \"\")");
        assert!(to_typst(r"\begin{cases} 1 & x > 0 \\ 0 & \text{otherwise} \end{cases}").is_ok());
        Ok(())
    }

    #[test]
    fn unknown_commands_and_bad_input_are_errors() {
        assert!(parse(r"\unknowncommand{x}").is_err());
        assert!(parse(r"\frac{a}{b").is_err());
        assert!(parse(r"\left( x").is_err());
        assert!(parse(&"{".repeat(200)).is_err());
        assert!(parse(&"x".repeat(10_001)).is_err());
    }

    #[test]
    fn omml_uses_office_math_structures() -> Result<(), MathError> {
        let omml = to_omml(r"\sum_{i=1}^{n} x_i^2 + \frac{a}{b} + \sqrt{y}")?;
        assert!(omml.starts_with("<m:oMath><m:nary>"));
        assert!(omml.contains("<m:chr m:val=\"∑\"/>"));
        assert!(omml.contains("<m:sSubSup>"));
        assert!(omml.contains("<m:f><m:num>"));
        assert!(omml.contains("<m:degHide m:val=\"1\"/>"));
        assert!(to_omml("a < b")?.contains("&lt;"));
        Ok(())
    }

    #[test]
    fn omml_escapes_text() -> Result<(), MathError> {
        let omml = to_omml(r"\text{a<b & \}c}")?;
        assert!(omml.contains("a&lt;b"));
        assert!(!omml.contains("a<b"));
        Ok(())
    }
}
