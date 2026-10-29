// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The editor's node tree, as `y-prosemirror` stores it in a Yjs document.
//!
//! A document has two roots:
//!
//! | Root | Yjs type | Holds |
//! | --- | --- | --- |
//! | [`CONTENT_FIELD`] | `XmlFragment` | the body: one `XmlElement` per node, named after the node type, with node attributes as element attributes; inline content is an `XmlText` whose formatting attributes are the marks |
//! | [`META_FIELD`] | `Map` | front matter, one key per field |
//!
//! [`DocTree`] is the plain-data copy of that structure that the
//! [`crate::MarkdownCodec`] converts to and from Markdown. Text offsets are
//! UTF-16 code units, as in Yjs, so Rust and browser peers agree.

use std::collections::BTreeMap;

use yrs::types::ToJson as _;
use yrs::types::text::YChange;
use yrs::types::xml::{XmlElementPrelim, XmlFragment as _, XmlOut, XmlTextPrelim};
use yrs::{
    Any, Doc, Map as _, MapRef, OffsetKind, Options, Out, ReadTxn, Text as _, Transact,
    TransactionMut, Xml as _, XmlElementRef, XmlFragmentRef, XmlTextRef,
};

/// Name of the `XmlFragment` root holding the body. The editor binds
/// `y-prosemirror` to this field.
pub const CONTENT_FIELD: &str = "content";

/// Name of the `Map` root holding front matter.
pub const META_FIELD: &str = "meta";

/// Largest top-level block count for which [`replace`] computes a minimal
/// edit; beyond it the changed middle is replaced wholesale.
const LCS_CELL_LIMIT: usize = 4_000_000;

/// A document as plain data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DocTree {
    /// Front matter fields.
    pub meta: BTreeMap<String, Any>,
    /// Top-level nodes of the body.
    pub content: Vec<Node>,
}

/// A node of the body.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A block or inline node with a type name (`paragraph`, `heading`).
    Element(Element),
    /// A run of inline text.
    Text(Vec<TextRun>),
}

/// A typed node with attributes and children.
#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    /// Node type name.
    pub name: String,
    /// Node attributes (`level` for a heading, `id` for a block ID).
    pub attrs: BTreeMap<String, Any>,
    /// Child nodes.
    pub children: Vec<Node>,
}

/// Text with one set of marks.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// The characters.
    pub text: String,
    /// Mark name to mark attributes (an empty map for marks without any).
    pub marks: BTreeMap<String, Any>,
}

impl Element {
    /// An element without attributes.
    #[must_use]
    pub fn new(name: impl Into<String>, children: Vec<Node>) -> Self {
        Self {
            name: name.into(),
            attrs: BTreeMap::new(),
            children,
        }
    }

    /// Adds an attribute.
    #[must_use]
    pub fn with_attr(mut self, key: impl Into<String>, value: impl Into<Any>) -> Self {
        let _previous = self.attrs.insert(key.into(), value.into());
        self
    }
}

impl TextRun {
    /// Unmarked text.
    #[must_use]
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            marks: BTreeMap::new(),
        }
    }
}

impl DocTree {
    /// The form [`read`] returns: empty text runs dropped and neighbouring
    /// runs with the same marks joined.
    #[must_use]
    pub fn normalized(&self) -> Self {
        Self {
            meta: self.meta.clone(),
            content: self.content.iter().map(Node::normalized).collect(),
        }
    }
}

impl Node {
    fn normalized(&self) -> Self {
        match self {
            Self::Element(element) => Self::Element(Element {
                name: element.name.clone(),
                attrs: element.attrs.clone(),
                children: element.children.iter().map(Self::normalized).collect(),
            }),
            Self::Text(runs) => {
                let mut out: Vec<TextRun> = Vec::with_capacity(runs.len());
                for run in runs.iter().filter(|run| !run.text.is_empty()) {
                    match out.last_mut() {
                        Some(last) if last.marks == run.marks => last.text.push_str(&run.text),
                        _ => out.push(run.clone()),
                    }
                }
                Self::Text(out)
            }
        }
    }
}

/// Options for every document Vellum creates: UTF-16 offsets, as in Yjs.
#[must_use]
pub fn doc_options() -> Options {
    Options {
        offset_kind: OffsetKind::Utf16,
        ..Options::default()
    }
}

/// A new, empty document with [`doc_options`].
#[must_use]
pub fn new_doc() -> Doc {
    Doc::with_options(doc_options())
}

fn roots(doc: &Doc) -> (XmlFragmentRef, MapRef) {
    (
        doc.get_or_insert_xml_fragment(CONTENT_FIELD),
        doc.get_or_insert_map(META_FIELD),
    )
}

/// Reads the whole document as a [`DocTree`].
#[must_use]
pub fn read(doc: &Doc) -> DocTree {
    let (content, meta) = roots(doc);
    let txn = doc.transact();
    DocTree {
        meta: meta
            .iter(&txn)
            .map(|(key, value)| (key.to_owned(), value.to_json(&txn)))
            .collect(),
        content: read_children(&txn, &content),
    }
}

fn read_children<T: ReadTxn>(txn: &T, parent: &impl yrs::types::xml::XmlFragment) -> Vec<Node> {
    let mut nodes = Vec::new();
    for child in parent.children(txn) {
        match child {
            XmlOut::Element(element) => nodes.push(Node::Element(read_element(txn, &element))),
            XmlOut::Text(text) => nodes.push(Node::Text(read_text(txn, &text))),
            XmlOut::Fragment(fragment) => nodes.extend(read_children(txn, &fragment)),
        }
    }
    nodes
}

fn read_element<T: ReadTxn>(txn: &T, element: &XmlElementRef) -> Element {
    Element {
        name: element.tag().to_string(),
        attrs: element
            .attributes(txn)
            .map(|(key, value)| (key.to_owned(), value.to_json(txn)))
            .collect(),
        children: read_children(txn, element),
    }
}

/// Reads text runs in normal form: no empty runs, neighbours with equal
/// marks joined (Yjs may split them across items from different clients).
fn read_text<T: ReadTxn>(txn: &T, text: &XmlTextRef) -> Vec<TextRun> {
    let runs = raw_runs(txn, text);
    let Node::Text(runs) = Node::Text(runs).normalized() else {
        return Vec::new();
    };
    runs
}

fn raw_runs<T: ReadTxn>(txn: &T, text: &XmlTextRef) -> Vec<TextRun> {
    text.diff(txn, YChange::identity)
        .into_iter()
        .filter_map(|chunk| {
            let Out::Any(Any::String(value)) = chunk.insert else {
                return None;
            };
            let marks = chunk
                .attributes
                .map(|attrs| {
                    attrs
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.clone()))
                        .collect()
                })
                .unwrap_or_default();
            Some(TextRun {
                text: value.to_string(),
                marks,
            })
        })
        .collect()
}

/// Replaces the whole document with `tree`, in one transaction.
pub fn write(doc: &Doc, tree: &DocTree) {
    let (content, meta) = roots(doc);
    let mut txn = doc.transact_mut();
    let len = content.len(&txn);
    if len > 0 {
        content.remove_range(&mut txn, 0, len);
    }
    for (index, node) in (0_u32..).zip(&tree.content) {
        insert_node(&mut txn, &content, index, node);
    }
    write_meta(&mut txn, &meta, &tree.meta);
    drop(txn);
}

/// Edits the document into `tree` and returns the Yjs update (lib0 v1).
///
/// Uses as few CRDT operations as the top-level blocks allow: unchanged
/// blocks keep their identity, so concurrent edits inside them survive.
#[must_use]
pub fn replace(doc: &Doc, tree: &DocTree) -> Vec<u8> {
    let current = read(doc);
    merge(doc, &current, tree)
}

/// Applies the change from `base` to `theirs` onto the document, block by
/// block, and returns the Yjs update (lib0 v1).
///
/// This is how an external edit of the Markdown file enters a live room:
/// `base` is the version the file held, `theirs` what it holds now. Blocks
/// the file changed are replaced, unless editors changed the same block in
/// the meantime; then both versions are kept, the file's first, so nothing
/// is lost (never last-write-wins). Front matter fields the file changed
/// are taken from the file.
#[must_use]
pub fn merge(doc: &Doc, base: &DocTree, theirs: &DocTree) -> Vec<u8> {
    let current = read(doc);
    let (content, meta) = roots(doc);
    let mut txn = doc.transact_mut();

    let mut fields = current.meta.clone();
    for key in base.meta.keys().chain(theirs.meta.keys()) {
        if base.meta.get(key) != theirs.meta.get(key) {
            match theirs.meta.get(key) {
                Some(value) => drop(fields.insert(key.clone(), value.clone())),
                None => drop(fields.remove(key)),
            }
        }
    }
    write_meta(&mut txn, &meta, &fields);

    let base_blocks = &base.content;
    let to_theirs = common_pairs(base_blocks, &theirs.content);
    let to_current = common_pairs(base_blocks, &current.content);
    let in_current: Vec<Option<usize>> = {
        let mut map = vec![None; base_blocks.len()];
        for (b, c) in &to_current {
            if let Some(slot) = map.get_mut(*b) {
                *slot = Some(*c);
            }
        }
        map
    };
    // Where a base block sits in the document, if editors left it alone;
    // a block after `anchor` goes right after the nearest such block.
    let gap_after = |anchor: Option<usize>| -> usize {
        anchor
            .and_then(|a| {
                (0..=a)
                    .rev()
                    .find_map(|b| in_current.get(b).copied().flatten())
            })
            .map_or(0, |c| c + 1)
    };

    let mut deletions = vec![false; current.content.len()];
    let mut insertions: Vec<Vec<&Node>> = vec![Vec::new(); current.content.len() + 1];
    let mut kept_base = vec![false; base_blocks.len()];
    for (b, _) in &to_theirs {
        if let Some(slot) = kept_base.get_mut(*b) {
            *slot = true;
        }
    }
    for (b, kept) in kept_base.iter().enumerate() {
        if !kept
            && let Some(Some(c)) = in_current.get(b)
            && let Some(slot) = deletions.get_mut(*c)
        {
            *slot = true;
        }
    }
    let mut anchor: Option<usize> = None;
    let mut pairs = to_theirs.iter().peekable();
    for (t, node) in theirs.content.iter().enumerate() {
        if let Some((b, _)) = pairs.next_if(|(_, pt)| *pt == t) {
            anchor = Some(*b);
            continue;
        }
        if let Some(slot) = insertions.get_mut(gap_after(anchor)) {
            slot.push(node);
        }
    }

    for index in (0..=current.content.len()).rev() {
        let position = u32::try_from(index).unwrap_or(u32::MAX);
        if deletions.get(index).copied().unwrap_or(false) {
            content.remove(&mut txn, position);
        }
        if let Some(nodes) = insertions.get(index) {
            for (offset, node) in (0_u32..).zip(nodes) {
                insert_node(&mut txn, &content, position + offset, node);
            }
        }
    }
    txn.encode_update_v1()
}

/// Index pairs of a longest common subsequence of two block lists. Past
/// [`LCS_CELL_LIMIT`] only the common prefix and suffix are matched.
fn common_pairs(old: &[Node], new: &[Node]) -> Vec<(usize, usize)> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let old_rest = old.get(prefix..).unwrap_or_default();
    let new_rest = new.get(prefix..).unwrap_or_default();
    let suffix = old_rest
        .iter()
        .rev()
        .zip(new_rest.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old_mid = old_rest.get(..old_rest.len() - suffix).unwrap_or_default();
    let new_mid = new_rest.get(..new_rest.len() - suffix).unwrap_or_default();

    let mut pairs: Vec<(usize, usize)> = (0..prefix).map(|i| (i, i)).collect();
    pairs.extend(
        middle_pairs(old_mid, new_mid)
            .into_iter()
            .map(|(o, n)| (o + prefix, n + prefix)),
    );
    let (old_tail, new_tail) = (prefix + old_mid.len(), prefix + new_mid.len());
    pairs.extend((0..suffix).map(|i| (old_tail + i, new_tail + i)));
    pairs
}

fn middle_pairs(old: &[Node], new: &[Node]) -> Vec<(usize, usize)> {
    let (rows, cols) = (old.len(), new.len());
    if rows == 0 || cols == 0 || rows.saturating_mul(cols) > LCS_CELL_LIMIT {
        return Vec::new();
    }
    // lengths[r][c] = LCS of old[r..] and new[c..], flattened row-major.
    let width = cols + 1;
    let at = |row: usize, col: usize| row * width + col;
    let mut lengths = vec![0_u32; (rows + 1) * width];
    for row in (0..rows).rev() {
        for col in (0..cols).rev() {
            let value = if old.get(row) == new.get(col) {
                cell(&lengths, at(row + 1, col + 1)) + 1
            } else {
                cell(&lengths, at(row + 1, col)).max(cell(&lengths, at(row, col + 1)))
            };
            if let Some(slot) = lengths.get_mut(at(row, col)) {
                *slot = value;
            }
        }
    }
    let mut pairs = Vec::new();
    let (mut row, mut col) = (0, 0);
    while row < rows && col < cols {
        if old.get(row) == new.get(col) {
            pairs.push((row, col));
            row += 1;
            col += 1;
        } else if cell(&lengths, at(row + 1, col)) >= cell(&lengths, at(row, col + 1)) {
            row += 1;
        } else {
            col += 1;
        }
    }
    pairs
}

fn cell(lengths: &[u32], index: usize) -> u32 {
    lengths.get(index).copied().unwrap_or(0)
}

fn write_meta(txn: &mut TransactionMut, meta: &MapRef, fields: &BTreeMap<String, Any>) {
    let stale: Vec<String> = meta
        .iter(txn)
        .filter(|(key, value)| fields.get(*key) != Some(&value.to_json(txn)))
        .map(|(key, _)| key.to_owned())
        .collect();
    for key in &stale {
        let _removed = meta.remove(txn, key);
    }
    for (key, value) in fields {
        if meta
            .get(txn, key)
            .map(|current| current.to_json(txn))
            .as_ref()
            != Some(value)
        {
            let _previous = meta.insert(txn, key.as_str(), value.clone());
        }
    }
}

fn insert_node(
    txn: &mut TransactionMut,
    parent: &impl yrs::types::xml::XmlFragment,
    index: u32,
    node: &Node,
) {
    match node {
        Node::Element(element) => {
            let target = parent.insert(txn, index, XmlElementPrelim::empty(element.name.as_str()));
            for (key, value) in &element.attrs {
                let _unused = target.insert_attribute(txn, key.as_str(), value.clone());
            }
            for (child_index, child) in (0_u32..).zip(&element.children) {
                insert_node(txn, &target, child_index, child);
            }
        }
        Node::Text(runs) => {
            let target = parent.insert(txn, index, XmlTextPrelim::new(""));
            for run in runs {
                let end = target.len(txn);
                let attrs = run
                    .marks
                    .iter()
                    .map(|(key, value)| (key.as_str().into(), value.clone()))
                    .collect();
                target.insert_with_attributes(txn, end, &run.text, attrs);
            }
        }
    }
}
