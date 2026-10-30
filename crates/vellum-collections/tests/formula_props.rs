// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests for the formula language: the parser never panics,
//! printing then parsing gives the same tree, and evaluation is total.

use proptest::prelude::*;
use vellum_collections::formula::{
    BinOp, Env, Expr, Func, Node, Span, UnOp, parse,
};
use vellum_collections::{DateValue, Value};

/// Removes spans so trees compare by structure.
fn strip(node: &Node) -> Node {
    let expr = match &node.expr {
        Expr::Unary(op, e) => Expr::Unary(*op, Box::new(strip(e))),
        Expr::Binary(op, l, r) => {
            Expr::Binary(*op, Box::new(strip(l)), Box::new(strip(r)))
        }
        Expr::Call(f, args) => Expr::Call(*f, args.iter().map(strip).collect()),
        other => other.clone(),
    };
    Node {
        expr,
        span: Span::default(),
    }
}

fn node(expr: Expr) -> Node {
    Node {
        expr,
        span: Span::default(),
    }
}

const BIN_OPS: [BinOp; 13] = [
    BinOp::Add,
    BinOp::Sub,
    BinOp::Mul,
    BinOp::Div,
    BinOp::Rem,
    BinOp::Eq,
    BinOp::Ne,
    BinOp::Lt,
    BinOp::Gt,
    BinOp::Le,
    BinOp::Ge,
    BinOp::And,
    BinOp::Or,
];

fn leaf() -> impl Strategy<Value = Node> {
    prop_oneof![
        (0u32..10_000).prop_map(|n| node(Expr::Number(f64::from(n) / 4.0))),
        "[a-z \"'\\\\é]{0,6}".prop_map(|s| node(Expr::Text(s))),
        any::<bool>().prop_map(|b| node(Expr::Bool(b))),
        prop::sample::select(vec!["a", "b", "c", "Due date"])
            .prop_map(|p| node(Expr::Prop(p.to_owned()))),
    ]
}

fn tree() -> impl Strategy<Value = Node> {
    leaf().prop_recursive(5, 48, 4, |inner| {
        prop_oneof![
            (
                prop::sample::select(vec![UnOp::Neg, UnOp::Not]),
                inner.clone()
            )
                .prop_map(|(op, e)| node(Expr::Unary(op, Box::new(e)))),
            (
                prop::sample::select(BIN_OPS.to_vec()),
                inner.clone(),
                inner.clone()
            )
                .prop_map(|(op, l, r)| node(Expr::Binary(
                    op,
                    Box::new(l),
                    Box::new(r)
                ))),
            (
                prop::sample::select(Func::ALL.to_vec()),
                prop::collection::vec(inner, 0..4)
            )
                .prop_filter_map("arity", |(f, mut args)| {
                    let (lo, hi) = f.arity();
                    if let Some(hi) = hi {
                        args.truncate(hi);
                    }
                    (args.len() >= lo).then(|| node(Expr::Call(f, args)))
                }),
        ]
    })
}

fn value() -> impl Strategy<Value = Value> {
    let scalar = prop_oneof![
        Just(Value::Empty),
        (-1e6f64..1e6).prop_map(Value::Number),
        "[a-zA-Z ]{0,8}".prop_map(Value::Text),
        any::<bool>().prop_map(Value::Bool),
        (2000i32..2100, 1u8..=12, 1u8..=28)
            .prop_filter_map("date", |(y, m, d)| DateValue::ymd(y, m, d)
                .map(Value::Date)),
    ];
    prop_oneof![
        4 => scalar.clone(),
        1 => prop::collection::vec(scalar, 0..4).prop_map(Value::List),
    ]
}

struct Fixed(Vec<Value>);

impl Env for Fixed {
    fn prop(&mut self, key: &str) -> Result<Value, String> {
        let i = match key {
            "a" => 0,
            "b" => 1,
            _ => 2,
        };
        Ok(self.0.get(i).cloned().unwrap_or_default())
    }

    fn now(&self) -> DateValue {
        DateValue::ymd(2026, 10, 29).unwrap_or_else(|| unreachable!())
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn parser_never_panics(src in any::<String>()) {
        let _ = parse(&src);
    }

    #[test]
    fn parser_never_panics_on_formula_like_text(src in "[a-z0-9 \
        ()\",.+*/%<>=!\\-]{0,40}") {
        let _ = parse(&src);
    }

    #[test]
    fn printing_then_parsing_round_trips(t in tree()) {
        let printed = t.to_string();
        let parsed = parse(&printed);
        prop_assert!(parsed.is_ok(), "{printed:?} did not parse: {parsed:?}");
        if let Ok(parsed) = parsed {
            prop_assert_eq!(
                strip(&parsed),
                strip(&t),
                "printed as {}",
                printed
            );
        }
    }

    #[test]
    fn evaluation_is_total_and_deterministic(
        t in tree(),
        vals in prop::collection::vec(value(), 3),
    ) {
        let schema = vellum_collections::Schema::from_yaml(
            "id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E\n\
                properties:\n  - {key: t, name: T, type: title}\n  - {key: a, \
                name: A, type: text}\n  - {key: b, name: B, type: text}\n  - \
                {key: c, name: Due date, type: text}\n",
        );
        prop_assert!(schema.is_ok());
        if let Ok(schema) = schema {
            let src = t.to_string();
            let f = vellum_collections::formula::compile(&src, &schema);
            prop_assert!(f.is_ok(), "{src} did not compile: {f:?}");
            if let Ok(f) = f {
                let first = f.eval(&mut Fixed(vals.clone()));
                let second = f.eval(&mut Fixed(vals));
                prop_assert_eq!(&first, &second);
                if let Ok(Value::Number(n)) = first {
                    prop_assert!(n.is_finite());
                }
            }
        }
    }

    #[test]
    fn addition_commutes(a in -1e9f64..1e9, b in -1e9f64..1e9) {
        let mut env = Fixed(vec![Value::Number(a), Value::Number(b)]);
        let schema = vellum_collections::Schema::from_yaml(
            "id: 01JBQ8Y4ZK6V3W2N5R7T9X1C3E\n\
                properties:\n  - {key: t, name: T, type: title}\n  - {key: a, \
                name: A, type: number}\n  - {key: b, name: B, type: number}\n",
        );
        prop_assert!(schema.is_ok());
        if let Ok(schema) = schema {
            let ab = vellum_collections::formula::compile(
                "A + B",
                &schema
            ).map(|f| f.eval(&mut env));
            let ba = vellum_collections::formula::compile(
                "B + A",
                &schema
            ).map(|f| f.eval(&mut env));
            prop_assert_eq!(ab, ba);
        }
    }
}
