// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Property tests: the node tree survives Yjs unchanged, concurrent edits
//! converge to one document, and that document has one Markdown form.

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::collections::BTreeMap;

use proptest::prelude::*;
use support::{TestCodec, sync_docs};
use vellum_collab::tree::{self, CONTENT_FIELD};
use vellum_collab::{DocTree, Element, MarkdownCodec as _, Node, TextRun};
use yrs::types::xml::{XmlElementPrelim, XmlFragment as _, XmlOut, XmlTextPrelim};
use yrs::{Any, Doc, Text as _, Transact as _, Xml as _};

// ---------------------------------------------------------------------------
// Arbitrary trees.

fn any_scalar() -> impl Strategy<Value = Any> {
    prop_oneof![
        Just(Any::Null),
        any::<bool>().prop_map(Any::from),
        (-(1_i64 << 50)..(1_i64 << 50)).prop_map(Any::from),
        (-1.0e9_f64..1.0e9).prop_map(Any::from),
        "[a-zA-Z0-9 éß中]{0,12}".prop_map(Any::from),
    ]
}

fn any_value() -> impl Strategy<Value = Any> {
    prop_oneof![
        3 => any_scalar(),
        1 => prop::collection::vec(any_scalar(), 0..4).prop_map(Any::from),
        1 => prop::collection::hash_map("[a-z]{1,5}", any_scalar(), 0..4).prop_map(|m| {
            Any::from(m.into_iter().collect::<std::collections::HashMap<String, Any>>())
        }),
    ]
}

fn marks() -> impl Strategy<Value = BTreeMap<String, Any>> {
    prop::collection::btree_map(
        prop_oneof![Just("bold"), Just("italic"), Just("link"), Just("code")]
            .prop_map(str::to_owned),
        prop_oneof![
            Just(Any::Map(std::sync::Arc::default())),
            "[a-z]{1,8}".prop_map(|href| {
                let mut attrs = std::collections::HashMap::new();
                let _previous = attrs.insert("href".to_owned(), Any::from(href));
                Any::from(attrs)
            }),
        ],
        0..3,
    )
}

fn text_node() -> impl Strategy<Value = Node> {
    prop::collection::vec(
        ("[a-z 😀éß]{0,10}", marks()).prop_map(|(text, marks)| TextRun { text, marks }),
        0..4,
    )
    .prop_map(Node::Text)
}

fn node() -> impl Strategy<Value = Node> {
    let leaf = prop_oneof![
        text_node(),
        "[a-z]{1,8}".prop_map(|name| Node::Element(Element::new(name, Vec::new()))),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        (
            "[a-z_]{1,10}",
            prop::collection::btree_map("[a-z]{1,6}", any_value(), 0..3),
            prop::collection::vec(inner, 0..4),
        )
            .prop_map(|(name, attrs, children)| {
                Node::Element(Element {
                    name,
                    attrs,
                    children,
                })
            })
    })
}

fn doc_tree() -> impl Strategy<Value = DocTree> {
    (
        prop::collection::btree_map("[a-z]{1,6}", any_value(), 0..3),
        prop::collection::vec(node(), 0..6),
    )
        .prop_map(|(meta, content)| DocTree { meta, content })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// A tree written into Yjs and synced to another replica reads back
    /// unchanged (up to joining equal-mark text runs).
    #[test]
    fn trees_survive_yjs(tree in doc_tree()) {
        let local = tree::new_doc();
        tree::write(&local, &tree);
        let remote = tree::new_doc();
        sync_docs(&local, &remote);
        prop_assert_eq!(tree::read(&local), tree.normalized());
        prop_assert_eq!(tree::read(&remote), tree.normalized());
    }

    /// Replacing a document's tree with another reads back as the other.
    #[test]
    fn replace_reaches_the_target(before in doc_tree(), after in doc_tree()) {
        let doc = tree::new_doc();
        tree::write(&doc, &before);
        let _update = tree::replace(&doc, &after);
        prop_assert_eq!(tree::read(&doc), after.normalized());
    }
}

// ---------------------------------------------------------------------------
// Concurrent editing over the Markdown subset the test codec speaks.

#[derive(Debug, Clone)]
enum Op {
    Type {
        block: usize,
        at: usize,
        text: String,
        bold: bool,
    },
    Delete {
        block: usize,
        at: usize,
        len: usize,
    },
    AddBlock {
        at: usize,
        heading: Option<u8>,
        text: String,
    },
    RemoveBlock {
        at: usize,
    },
    /// The Markdown file changed on disk: merged into this replica.
    External {
        markdown: String,
    },
    Sync {
        with: usize,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        6 => (any::<usize>(), any::<usize>(), "[a-z ]{1,6}", any::<bool>())
            .prop_map(|(block, at, text, bold)| Op::Type { block, at, text, bold }),
        2 => (any::<usize>(), any::<usize>(), 1..4_usize)
            .prop_map(|(block, at, len)| Op::Delete { block, at, len }),
        2 => (any::<usize>(), prop::option::of(1..4_u8), "[a-z]{1,6}")
            .prop_map(|(at, heading, text)| Op::AddBlock { at, heading, text }),
        1 => any::<usize>().prop_map(|at| Op::RemoveBlock { at }),
        1 => prop::collection::vec("(# )?[a-z]{1,5}( \\*\\*[a-z]{1,4}\\*\\*)?", 1..4)
            .prop_map(|blocks| Op::External { markdown: format!("{}\n", blocks.join("\n\n")) }),
        2 => any::<usize>().prop_map(|with| Op::Sync { with }),
    ]
}

fn block_text(doc: &Doc, block: usize) -> Option<(yrs::XmlTextRef, u32)> {
    let content = doc.get_or_insert_xml_fragment(CONTENT_FIELD);
    let txn = doc.transact();
    let len = content.len(&txn);
    if len == 0 {
        return None;
    }
    let index = u32::try_from(block % len as usize).unwrap();
    let Some(XmlOut::Element(element)) = content.get(&txn, index) else {
        return None;
    };
    let Some(XmlOut::Text(text)) = element.get(&txn, 0) else {
        return None;
    };
    let text_len = text.len(&txn);
    drop(txn);
    Some((text, text_len))
}

fn apply(replicas: &[Doc], peer: usize, op: &Op) {
    let Some(doc) = replicas.get(peer) else {
        return;
    };
    match op {
        Op::Type {
            block,
            at,
            text,
            bold,
        } => {
            if let Some((target, len)) = block_text(doc, *block) {
                let index = u32::try_from(*at % (len as usize + 1)).unwrap();
                let mut txn = doc.transact_mut();
                let mut attrs = yrs::types::Attrs::new();
                if *bold {
                    let _previous =
                        attrs.insert("bold".into(), Any::Map(std::sync::Arc::default()));
                }
                target.insert_with_attributes(&mut txn, index, text, attrs);
            }
        }
        Op::Delete { block, at, len } => {
            if let Some((target, text_len)) = block_text(doc, *block)
                && text_len > 0
            {
                let start = u32::try_from(*at % text_len as usize).unwrap();
                let count = u32::try_from(*len).unwrap().min(text_len - start);
                target.remove_range(&mut doc.transact_mut(), start, count);
            }
        }
        Op::AddBlock { at, heading, text } => {
            let content = doc.get_or_insert_xml_fragment(CONTENT_FIELD);
            let mut txn = doc.transact_mut();
            let len = content.len(&txn);
            let index = u32::try_from(*at % (len as usize + 1)).unwrap();
            let element = match heading {
                Some(level) => {
                    let element =
                        content.insert(&mut txn, index, XmlElementPrelim::empty("heading"));
                    let _unused = element.insert_attribute(&mut txn, "level", i64::from(*level));
                    element
                }
                None => content.insert(&mut txn, index, XmlElementPrelim::empty("paragraph")),
            };
            let target = element.insert(&mut txn, 0, XmlTextPrelim::new(""));
            target.insert(&mut txn, 0, text);
            drop(txn);
        }
        Op::RemoveBlock { at } => {
            let content = doc.get_or_insert_xml_fragment(CONTENT_FIELD);
            let mut txn = doc.transact_mut();
            let len = content.len(&txn);
            if len > 0 {
                content.remove(&mut txn, u32::try_from(*at % len as usize).unwrap());
            }
            drop(txn);
        }
        Op::External { markdown } => {
            let base = tree::read(doc);
            let theirs = TestCodec.parse_markdown(markdown).unwrap();
            let _update = tree::merge(doc, &base, &theirs);
        }
        Op::Sync { with } => {
            let other = with % replicas.len();
            if other != peer
                && let Some(other) = replicas.get(other)
            {
                sync_docs(doc, other);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Three editors start from the same Markdown, edit concurrently with
    /// partial syncs in between, then sync fully: every replica holds the
    /// same tree and the same Markdown, and that Markdown is canonical
    /// (parsing and serializing it again changes nothing).
    #[test]
    fn concurrent_edits_converge_to_one_markdown(
        start in prop::collection::vec("(# )?[a-z]{1,5}", 0..4),
        script in prop::collection::vec((0..3_usize, op()), 1..40),
    ) {
        let initial = if start.is_empty() { "\n".to_owned() } else { format!("{}\n", start.join("\n\n")) };
        let seed = tree::new_doc();
        tree::write(&seed, &TestCodec.parse_markdown(&initial).unwrap());
        let replicas: Vec<Doc> = (0..3).map(|_| tree::new_doc()).collect();
        for replica in &replicas {
            sync_docs(&seed, replica);
        }

        for (peer, op) in &script {
            apply(&replicas, *peer, op);
        }
        for _ in 0..2 {
            for a in 0..replicas.len() {
                for b in (a + 1)..replicas.len() {
                    sync_docs(&replicas[a], &replicas[b]);
                }
            }
        }

        let trees: Vec<DocTree> = replicas.iter().map(tree::read).collect();
        let markdown: Vec<String> = trees.iter().map(|t| TestCodec.to_markdown(t).unwrap()).collect();
        prop_assert_eq!(&trees[0], &trees[1]);
        prop_assert_eq!(&trees[1], &trees[2]);
        prop_assert_eq!(&markdown[0], &markdown[1]);
        prop_assert_eq!(&markdown[1], &markdown[2]);

        // CRDT → Markdown → CRDT → Markdown is a fixed point.
        let reloaded = tree::new_doc();
        tree::write(&reloaded, &TestCodec.parse_markdown(&markdown[0]).unwrap());
        prop_assert_eq!(TestCodec.to_markdown(&tree::read(&reloaded)).unwrap(), markdown[0].clone());
    }

    /// An external edit merged into a live document keeps every block the
    /// editors changed concurrently.
    #[test]
    fn external_edits_keep_concurrent_typing(
        blocks in prop::collection::vec("[a-z]{1,5}", 2..6),
        typed in 0..6_usize,
        changed in 0..6_usize,
    ) {
        let base_md = format!("{}\n", blocks.join("\n\n"));
        let base = TestCodec.parse_markdown(&base_md).unwrap();
        let doc = tree::new_doc();
        tree::write(&doc, &base);

        let typed = typed % blocks.len();
        let changed = changed % blocks.len();
        if let Some((target, len)) = block_text(&doc, typed) {
            target.insert(&mut doc.transact_mut(), len, "TYPED");
        }
        let mut theirs_blocks = blocks;
        theirs_blocks[changed] = format!("{} **ext**", theirs_blocks[changed]);
        let theirs = TestCodec.parse_markdown(&format!("{}\n", theirs_blocks.join("\n\n"))).unwrap();
        let _update = tree::merge(&doc, &base, &theirs);

        let merged = TestCodec.to_markdown(&tree::read(&doc)).unwrap();
        prop_assert!(merged.contains("TYPED"), "{}", merged);
        prop_assert!(merged.contains("**ext**"), "{}", merged);
    }
}
