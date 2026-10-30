// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Shared test helpers: a small Markdown codec and a scripted editor.

#![allow(dead_code, missing_docs, clippy::missing_panics_doc)]

pub mod socket;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use vellum_collab::memory::{MemoryFiles, MemoryUpdateStore};
use vellum_collab::protocol::{self, MSG_DONE, MSG_PEERS, PeerEntry};
use vellum_collab::tree::{self, CONTENT_FIELD};
use vellum_collab::{
    Access, CollabConfig, Connection, DocTree, Element, Hub, JoinRequest,
    MarkdownCodec, Node, Outbound, RoomKey, TextRun,
};
use vellum_kernel::{DocId, Error, ErrorKind, Result, SpaceId, UserId};
use yrs::sync::{Message, SyncMessage};
use yrs::types::xml::{
    XmlElementPrelim, XmlFragment as _, XmlOut, XmlTextPrelim,
};
use yrs::updates::decoder::Decode as _;
use yrs::{Any, Doc, ReadTxn as _, Text as _, Transact as _, Update};

/// A codec for the subset these tests edit: headings and paragraphs whose
/// text may be bold, and string front matter. Canonical by construction:
/// `to_markdown(parse_markdown(md))` is a fixed point.
#[derive(Debug, Default)]
pub struct TestCodec;

impl MarkdownCodec for TestCodec {
    fn to_markdown(&self, doc: &DocTree) -> Result<String> {
        let mut out = String::new();
        if !doc.meta.is_empty() {
            out.push_str("---\n");
            for (key, value) in &doc.meta {
                let Any::String(value) = value else {
                    return Err(Error::new(
                        ErrorKind::InvalidInput,
                        "string front matter only",
                    ));
                };
                let _infallible = writeln!(out, "{key}: {value}");
            }
            out.push_str("---\n\n");
        }
        let blocks: Vec<String> = doc
            .content
            .iter()
            .map(block_markdown)
            .collect::<Result<_>>()?;
        out.push_str(&blocks.join("\n\n"));
        out.push('\n');
        Ok(out)
    }

    fn parse_markdown(&self, markdown: &str) -> Result<DocTree> {
        let mut meta = BTreeMap::new();
        let mut body = markdown.strip_suffix('\n').unwrap_or(markdown);
        if let Some(rest) = body.strip_prefix("---\n")
            && let Some((head, tail)) = rest.split_once("---\n\n")
        {
            for line in head.lines() {
                if let Some((key, value)) = line.split_once(": ") {
                    let _previous = meta
                        .insert(key.to_owned(), Any::from(value.to_owned()));
                }
            }
            body = tail;
        }
        let content = body.split("\n\n").map(parse_block).collect();
        Ok(DocTree { meta, content })
    }
}

fn block_markdown(node: &Node) -> Result<String> {
    let Node::Element(element) = node else {
        return Err(Error::new(ErrorKind::InvalidInput, "blocks only"));
    };
    let inline = inline_markdown(&element.children);
    match element.name.as_str() {
        "paragraph" => Ok(inline),
        "heading" => {
            let level = match element.attrs.get("level") {
                Some(Any::Number(n)) => {
                    usize::try_from(n.as_i64().unwrap_or(1)).unwrap_or(1)
                }
                _ => 1,
            };
            Ok(format!("{} {inline}", "#".repeat(level.clamp(1, 6))))
        }
        other => Err(Error::new(
            ErrorKind::InvalidInput,
            format!("no block {other}"),
        )),
    }
}

fn inline_markdown(children: &[Node]) -> String {
    let mut out = String::new();
    for child in children {
        if let Node::Text(runs) = child {
            for run in runs {
                if run.text.is_empty() {
                    continue;
                }
                if run.marks.contains_key("bold") {
                    let _infallible = write!(out, "**{}**", run.text);
                } else {
                    out.push_str(&run.text);
                }
            }
        }
    }
    out
}

fn parse_block(block: &str) -> Node {
    let hashes = block.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes)
        && let Some(text) =
            block.get(hashes..).and_then(|rest| rest.strip_prefix(' '))
    {
        return Node::Element(
            Element::new("heading", vec![Node::Text(parse_inline(text))])
                .with_attr("level", i64::try_from(hashes).unwrap_or(1)),
        );
    }
    Node::Element(Element::new(
        "paragraph",
        vec![Node::Text(parse_inline(block))],
    ))
}

fn parse_inline(text: &str) -> Vec<TextRun> {
    let mut runs = Vec::new();
    for (index, part) in text.split("**").enumerate() {
        if part.is_empty() {
            continue;
        }
        let mut run = TextRun::plain(part);
        if index % 2 == 1 {
            let _previous = run
                .marks
                .insert("bold".to_owned(), Any::Map(Arc::default()));
        }
        runs.push(run);
    }
    runs
}

pub fn room() -> RoomKey {
    RoomKey {
        space: SpaceId::generate(),
        doc: DocId::generate(),
    }
}

pub struct Fixture {
    pub hub: Hub,
    pub store: Arc<MemoryUpdateStore>,
    pub files: Arc<MemoryFiles>,
}

pub fn fixture(config: CollabConfig) -> Fixture {
    let store = Arc::new(MemoryUpdateStore::new());
    let files = Arc::new(MemoryFiles::new());
    let hub =
        Hub::new(store.clone(), files.clone(), Arc::new(TestCodec), config);
    Fixture { hub, store, files }
}

impl Fixture {
    /// Another node over the same storage.
    pub fn second_node(&self, config: CollabConfig) -> Hub {
        Hub::new(
            self.store.clone(),
            self.files.clone(),
            Arc::new(TestCodec),
            config,
        )
    }
}

/// A scripted editor: a local Yjs document synced through a connection.
pub struct Editor {
    pub user: UserId,
    pub doc: Doc,
    pub connection: Connection,
    pub peers: Vec<PeerEntry>,
    pub dones: usize,
    pub closed: Option<Outbound>,
    pub awareness: Vec<yrs::sync::awareness::AwarenessUpdate>,
}

pub const WAIT: Duration = Duration::from_secs(5);

impl Editor {
    pub async fn join(hub: &Hub, room: RoomKey, access: Access) -> Self {
        Self::join_with(hub, room, access, UserId::generate(), tree::new_doc())
            .await
    }

    pub async fn join_with(
        hub: &Hub,
        room: RoomKey,
        access: Access,
        user: UserId,
        doc: Doc,
    ) -> Self {
        let connection = hub
            .join(JoinRequest {
                room,
                user,
                access,
                traceparent: None,
            })
            .await
            .expect("join");
        let editor = Self {
            user,
            doc,
            connection,
            peers: Vec::new(),
            dones: 0,
            closed: None,
            awareness: Vec::new(),
        };
        let state_vector = editor.doc.transact().state_vector();
        editor
            .send(&[Message::Sync(SyncMessage::SyncStep1(state_vector))])
            .await;
        editor
    }

    pub async fn send(&self, messages: &[Message]) {
        self.connection
            .send(protocol::encode(messages))
            .await
            .expect("send");
    }

    /// Applies whatever the room sent, waiting up to `WAIT` for `until`.
    pub async fn pump_until(
        &mut self,
        mut until: impl FnMut(&Self) -> bool,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if until(self) {
                return true;
            }
            let Ok(next) =
                tokio::time::timeout_at(deadline, self.connection.recv()).await
            else {
                return until(self);
            };
            match next {
                Some(Outbound::Frame(frame)) => self.on_frame(&frame).await,
                Some(other @ Outbound::Close(_)) => {
                    self.closed = Some(other);
                    return until(self);
                }
                None => return until(self),
            }
        }
    }

    /// Applies frames that are already queued, without waiting.
    pub async fn pump(&mut self) {
        while let Ok(Some(next)) = tokio::time::timeout(
            Duration::from_millis(20),
            self.connection.recv(),
        )
        .await
        {
            match next {
                Outbound::Frame(frame) => self.on_frame(&frame).await,
                other @ Outbound::Close(_) => {
                    self.closed = Some(other);
                    return;
                }
            }
        }
    }

    async fn on_frame(&mut self, frame: &[u8]) {
        for message in protocol::decode(frame).expect("frame") {
            match message {
                Message::Sync(SyncMessage::SyncStep1(state_vector)) => {
                    let update = self
                        .doc
                        .transact()
                        .encode_state_as_update_v1(&state_vector);
                    self.send(&[Message::Sync(SyncMessage::SyncStep2(update))])
                        .await;
                }
                Message::Sync(
                    SyncMessage::SyncStep2(update)
                    | SyncMessage::Update(update),
                ) => {
                    let update = Update::decode_v1(&update).expect("update");
                    self.doc
                        .transact_mut()
                        .apply_update(update)
                        .expect("apply");
                }
                Message::Custom(MSG_DONE, _) => self.dones += 1,
                Message::Custom(MSG_PEERS, payload) => {
                    self.peers =
                        protocol::decode_peers(&payload).expect("peers");
                }
                Message::Awareness(update) => self.awareness.push(update),
                _ => {}
            }
        }
    }

    /// Runs a local edit and sends it to the room.
    pub async fn edit(&self, change: impl FnOnce(&Doc)) {
        let before = self.doc.transact().state_vector();
        change(&self.doc);
        let update = self.doc.transact().encode_state_as_update_v1(&before);
        self.send(&[Message::Sync(SyncMessage::Update(update))])
            .await;
    }

    pub fn tree(&self) -> DocTree {
        tree::read(&self.doc)
    }

    pub fn markdown(&self) -> String {
        TestCodec.to_markdown(&self.tree()).expect("markdown")
    }
}

/// Appends `text` to block `index` (creating a paragraph if there is none).
pub fn type_into(doc: &Doc, index: u32, text: &str) {
    let content = doc.get_or_insert_xml_fragment(CONTENT_FIELD);
    let mut txn = doc.transact_mut();
    let block = if let Some(XmlOut::Element(element)) = content.get(&txn, index)
    {
        element
    } else {
        let at = content.len(&txn);
        let element =
            content.insert(&mut txn, at, XmlElementPrelim::empty("paragraph"));
        let _text = element.insert(&mut txn, 0, XmlTextPrelim::new(""));
        element
    };
    if let Some(XmlOut::Text(target)) = block.get(&txn, 0) {
        let end = target.len(&txn);
        target.insert(&mut txn, end, text);
    }
    drop(txn);
}

/// Adds a paragraph at the end.
pub fn add_paragraph(doc: &Doc, text: &str) {
    let content = doc.get_or_insert_xml_fragment(CONTENT_FIELD);
    let mut txn = doc.transact_mut();
    let at = content.len(&txn);
    let element =
        content.insert(&mut txn, at, XmlElementPrelim::empty("paragraph"));
    let target = element.insert(&mut txn, 0, XmlTextPrelim::new(""));
    target.insert(&mut txn, 0, text);
    drop(txn);
}

/// Exchanges state both ways between two documents.
pub fn sync_docs(a: &Doc, b: &Doc) {
    let to_b = a
        .transact()
        .encode_state_as_update_v1(&b.transact().state_vector());
    let to_a = b
        .transact()
        .encode_state_as_update_v1(&a.transact().state_vector());
    b.transact_mut()
        .apply_update(Update::decode_v1(&to_b).expect("decode"))
        .expect("apply");
    a.transact_mut()
        .apply_update(Update::decode_v1(&to_a).expect("decode"))
        .expect("apply");
}
