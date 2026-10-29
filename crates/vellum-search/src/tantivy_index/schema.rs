// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! The tantivy schema and the mapping between records and documents.

use tantivy::schema::{
    BytesOptions, FAST, Field, INDEXED, IndexRecordOption, NumericOptions, STORED, STRING, Schema,
    TextFieldIndexing, TextOptions, Value,
};
use tantivy::tokenizer::{
    AsciiFoldingFilter, LowerCaser, RemoveLongFilter, SimpleTokenizer, TextAnalyzer, TokenStream,
};
use tantivy::{Index, TantivyDocument};
use vellum_kernel::{BlockId, DocId, SpaceId};

use crate::backend::IndexEntry;
use crate::error::{Result, SearchError};
use crate::model::{BlockKind, DocKind, StoredBlock, TitleHit};
use crate::text::{self, ANALYZER};
use crate::vector;

/// Record type of a block.
pub const REC_BLOCK: &str = "b";
/// Record type of a document's title record.
pub const REC_DOC: &str = "d";
/// The section key of blocks before the first heading.
pub const NO_SECTION: u64 = u64::MAX;
/// The vector fast field.
pub const VEC: &str = "vec";
/// The unstemmed analyzer used for title prefixes.
const PLAIN: &str = "vellum_plain";

/// Field handles.
#[derive(Debug, Clone, Copy)]
pub struct Fields {
    pub rec: Field,
    pub space: Field,
    pub doc: Field,
    pub block: Field,
    pub position: Field,
    pub section: Field,
    pub doc_kind: Field,
    pub block_kind: Field,
    pub title: Field,
    pub title_plain: Field,
    pub path: Field,
    pub headings: Field,
    pub body: Field,
    pub owner: Field,
    pub modified: Field,
    pub ancestor: Field,
    pub trashed: Field,
    pub vec: Field,
}

fn text(analyzer: &str, stored: bool) -> TextOptions {
    let options = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(analyzer)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );
    if stored {
        options.set_stored()
    } else {
        options
    }
}

/// Builds the schema.
pub fn build() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let fields = Fields {
        rec: b.add_text_field("rec", STRING),
        space: b.add_text_field("space", STRING | STORED),
        doc: b.add_text_field("doc", STRING | STORED),
        block: b.add_u64_field("block", INDEXED | STORED),
        position: b.add_u64_field("position", NumericOptions::default().set_stored()),
        section: b.add_u64_field("section", INDEXED | STORED),
        doc_kind: b.add_text_field("doc_kind", STRING | STORED),
        block_kind: b.add_text_field("block_kind", STRING | STORED),
        title: b.add_text_field("title", text(ANALYZER, true)),
        title_plain: b.add_text_field("title_plain", text(PLAIN, false)),
        path: b.add_text_field("path", TextOptions::default().set_stored()),
        headings: b.add_text_field("headings", text(ANALYZER, true)),
        body: b.add_text_field("body", text(ANALYZER, true)),
        owner: b.add_text_field("owner", STRING),
        modified: b.add_i64_field("modified", INDEXED | FAST),
        ancestor: b.add_text_field("ancestor", STRING),
        trashed: b.add_u64_field("trashed", INDEXED),
        vec: b.add_bytes_field(VEC, BytesOptions::default().set_fast()),
    };
    (b.build(), fields)
}

fn plain_analyzer() -> TextAnalyzer {
    TextAnalyzer::builder(SimpleTokenizer::default())
        .filter(RemoveLongFilter::limit(40))
        .filter(LowerCaser)
        .filter(AsciiFoldingFilter)
        .build()
}

/// Registers the analyzers on an index.
pub fn register(index: &Index) {
    index.tokenizers().register(ANALYZER, text::analyzer());
    index.tokenizers().register(PLAIN, plain_analyzer());
}

/// Lower-cased, ASCII-folded, unstemmed words of `input`: the vocabulary of
/// title prefixes, and the safe word list the Postgres query builder uses.
pub fn plain_tokens(input: &str) -> Vec<String> {
    let mut analyzer = plain_analyzer();
    let mut stream = analyzer.token_stream(input);
    let mut out = Vec::new();
    while stream.advance() {
        out.push(stream.token().text.clone());
    }
    out
}

impl Fields {
    /// Looks the fields up in an existing index's schema.
    pub fn resolve(schema: &Schema) -> Result<Self> {
        let get = |name: &str| {
            schema
                .get_field(name)
                .map_err(|_| SearchError::Corrupt(format!("index has no `{name}` field")))
        };
        Ok(Self {
            rec: get("rec")?,
            space: get("space")?,
            doc: get("doc")?,
            block: get("block")?,
            position: get("position")?,
            section: get("section")?,
            doc_kind: get("doc_kind")?,
            block_kind: get("block_kind")?,
            title: get("title")?,
            title_plain: get("title_plain")?,
            path: get("path")?,
            headings: get("headings")?,
            body: get("body")?,
            owner: get("owner")?,
            modified: get("modified")?,
            ancestor: get("ancestor")?,
            trashed: get("trashed")?,
            vec: get(VEC)?,
        })
    }
}

/// Fields shared by a document's block records and its title record.
fn common(f: &Fields, entry: &IndexEntry, rec: &str) -> TantivyDocument {
    let doc = &entry.doc;
    let mut d = TantivyDocument::default();
    d.add_text(f.rec, rec);
    d.add_text(f.space, doc.space.to_string());
    d.add_text(f.doc, doc.doc.to_string());
    d.add_text(f.doc_kind, doc.kind.as_str());
    d.add_text(f.title, &doc.title);
    d.add_text(f.path, &doc.path);
    if let Some(owner) = doc.owner {
        d.add_text(f.owner, owner.to_string());
    }
    d.add_i64(f.modified, doc.modified.unix_ms());
    for ancestor in &doc.ancestors {
        d.add_text(f.ancestor, ancestor.to_string());
    }
    d.add_u64(f.trashed, u64::from(doc.trashed));
    d
}

/// The tantivy documents for one index entry: a title record and one per block.
pub fn documents(f: &Fields, entry: &IndexEntry) -> Result<Vec<TantivyDocument>> {
    if let Some(vectors) = &entry.vectors
        && vectors.len() != entry.doc.blocks.len()
    {
        return Err(SearchError::Embedding(format!(
            "{} vectors for {} blocks",
            vectors.len(),
            entry.doc.blocks.len()
        )));
    }
    let mut out = Vec::with_capacity(entry.doc.blocks.len() + 1);
    let mut title = common(f, entry, REC_DOC);
    title.add_text(f.title_plain, &entry.doc.title);
    out.push(title);
    for (position, block) in entry.doc.blocks.iter().enumerate() {
        let mut d = common(f, entry, REC_BLOCK);
        d.add_u64(f.block, block.id.as_u64());
        d.add_u64(f.position, u64::try_from(position).unwrap_or(u64::MAX));
        d.add_u64(f.section, block.section.map_or(NO_SECTION, BlockId::as_u64));
        d.add_text(f.block_kind, block.kind.as_str());
        for heading in &block.heading_path {
            d.add_text(f.headings, heading);
        }
        d.add_text(f.body, &block.text);
        if let Some(v) = entry.vectors.as_ref().and_then(|vs| vs.get(position)) {
            d.add_bytes(f.vec, &vector::quantise(v));
        }
        out.push(d);
    }
    Ok(out)
}

fn str_field<'a>(d: &'a TantivyDocument, field: Field, name: &str) -> Result<&'a str> {
    d.get_first(field)
        .and_then(|v| v.as_str())
        .ok_or_else(|| SearchError::Corrupt(format!("record has no `{name}`")))
}

fn u64_field(d: &TantivyDocument, field: Field, name: &str) -> Result<u64> {
    d.get_first(field)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| SearchError::Corrupt(format!("record has no `{name}`")))
}

fn parse<T: std::str::FromStr>(text: &str, name: &str) -> Result<T> {
    text.parse()
        .map_err(|_| SearchError::Corrupt(format!("bad `{name}` value")))
}

/// Reads a stored block record back.
pub fn read_block(f: &Fields, d: &TantivyDocument) -> Result<StoredBlock> {
    let block = BlockId::from_u64(u64_field(d, f.block, "block")?)
        .ok_or_else(|| SearchError::Corrupt("block id out of range".into()))?;
    let section = match u64_field(d, f.section, "section")? {
        NO_SECTION => None,
        raw => Some(
            BlockId::from_u64(raw)
                .ok_or_else(|| SearchError::Corrupt("section id out of range".into()))?,
        ),
    };
    Ok(StoredBlock {
        space: parse::<SpaceId>(str_field(d, f.space, "space")?, "space")?,
        doc: parse::<DocId>(str_field(d, f.doc, "doc")?, "doc")?,
        block,
        position: u32::try_from(u64_field(d, f.position, "position")?).unwrap_or(u32::MAX),
        section,
        doc_kind: parse::<DocKind>(str_field(d, f.doc_kind, "doc_kind")?, "doc_kind")?,
        block_kind: parse::<BlockKind>(str_field(d, f.block_kind, "block_kind")?, "block_kind")?,
        title: str_field(d, f.title, "title")?.to_owned(),
        path: str_field(d, f.path, "path")?.to_owned(),
        heading_path: d
            .get_all(f.headings)
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        text: str_field(d, f.body, "body")?.to_owned(),
    })
}

/// Reads a stored title record back.
pub fn read_title(f: &Fields, d: &TantivyDocument) -> Result<TitleHit> {
    Ok(TitleHit {
        space: parse::<SpaceId>(str_field(d, f.space, "space")?, "space")?,
        doc: parse::<DocId>(str_field(d, f.doc, "doc")?, "doc")?,
        kind: parse::<DocKind>(str_field(d, f.doc_kind, "doc_kind")?, "doc_kind")?,
        title: str_field(d, f.title, "title")?.to_owned(),
        path: str_field(d, f.path, "path")?.to_owned(),
    })
}
