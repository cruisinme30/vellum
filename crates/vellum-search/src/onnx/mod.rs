// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Local sentence embeddings from an ONNX model (R-AI-7), with no external
//! service and no native runtime: inference runs on tract, pure Rust.
//!
//! Any BERT-style sentence-embedding export works (MiniLM, BGE, E5): the
//! model takes `input_ids`, `attention_mask` and optionally
//! `token_type_ids`, and returns token states (mean-pooled here over the
//! attention mask) or an already pooled sentence vector. The operator
//! points the server at `model.onnx` and `vocab.txt`; nothing is
//! downloaded. The model loads on first use and stays resident only while
//! hybrid search is enabled (R-LT-2).

mod wordpiece;

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::OnceCell;
use tract_onnx::prelude::{
    Framework as _, InferenceModelExt as _, IntoRunnable as _, TValue, Tensor, TypedRunnableModel,
    tvec,
};

pub use self::wordpiece::WordPiece;
use crate::error::{Result, SearchError};
use crate::vector::{self, Embedder};

/// Where the model lives and how to call it.
#[derive(Debug, Clone)]
pub struct OnnxConfig {
    /// The ONNX file.
    pub model: PathBuf,
    /// The WordPiece `vocab.txt`.
    pub vocab: PathBuf,
    /// A stable name of the model, stored with the index.
    pub model_id: String,
    /// The vector size the model produces.
    pub dimensions: usize,
    /// The longest input, in tokens (most models accept 256 or 512).
    pub max_tokens: usize,
    /// Prepended to queries (E5 wants `query: `).
    pub query_prefix: String,
    /// Prepended to passages (E5 wants `passage: `).
    pub passage_prefix: String,
}

struct Loaded {
    model: Arc<TypedRunnableModel>,
    vocab: WordPiece,
    inputs: Vec<Input>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Ids,
    Mask,
    TypeIds,
}

/// An [`Embedder`] backed by a local ONNX model.
#[derive(Clone)]
pub struct OnnxEmbedder {
    config: Arc<OnnxConfig>,
    loaded: Arc<OnceCell<Arc<Loaded>>>,
}

impl std::fmt::Debug for OnnxEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnnxEmbedder")
            .field("config", &self.config)
            .field("loaded", &self.loaded.initialized())
            .finish()
    }
}

fn model_error(error: impl std::fmt::Display) -> SearchError {
    SearchError::Embedding(error.to_string())
}

impl OnnxEmbedder {
    /// An embedder that loads `config.model` on first use.
    #[must_use]
    pub fn new(config: OnnxConfig) -> Self {
        Self {
            config: Arc::new(config),
            loaded: Arc::new(OnceCell::new()),
        }
    }

    async fn loaded(&self) -> Result<Arc<Loaded>> {
        let config = Arc::clone(&self.config);
        self.loaded
            .get_or_try_init(|| async move {
                tokio::task::spawn_blocking(move || load(&config).map(Arc::new))
                    .await
                    .map_err(model_error)?
            })
            .await
            .cloned()
    }

    async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let loaded = self.loaded().await?;
        let max_tokens = self.config.max_tokens;
        let dimensions = self.config.dimensions;
        tokio::task::spawn_blocking(move || run(&loaded, &texts, max_tokens, dimensions))
            .await
            .map_err(model_error)?
    }
}

fn load(config: &OnnxConfig) -> Result<Loaded> {
    let vocab = WordPiece::from_vocab(&std::fs::read_to_string(&config.vocab)?)?;
    let model = tract_onnx::onnx()
        .model_for_path(&config.model)
        .map_err(model_error)?;
    let mut inputs = Vec::new();
    for outlet in model.input_outlets().map_err(model_error)? {
        let name = model.node(outlet.node).name.to_lowercase();
        inputs.push(if name.contains("mask") {
            Input::Mask
        } else if name.contains("type") {
            Input::TypeIds
        } else if name.contains("ids") {
            Input::Ids
        } else {
            return Err(SearchError::Embedding(format!(
                "unexpected model input `{name}`"
            )));
        });
    }
    if !inputs.contains(&Input::Ids) {
        return Err(SearchError::Embedding(
            "the model has no input_ids input".into(),
        ));
    }
    let model = model
        .into_optimized()
        .and_then(|m| m.into_runnable())
        .map_err(model_error)?;
    tracing::info!(model = %config.model_id, "loaded the embedding model");
    Ok(Loaded {
        model,
        vocab,
        inputs,
    })
}

/// Tokenises, pads, runs the model and pools one unit vector per text.
fn run(
    loaded: &Loaded,
    texts: &[String],
    max_tokens: usize,
    dimensions: usize,
) -> Result<Vec<Vec<f32>>> {
    let encodings: Vec<Vec<i64>> = texts
        .iter()
        .map(|t| loaded.vocab.encode(t, max_tokens).ids)
        .collect();
    let batch = encodings.len();
    let seq = encodings.iter().map(Vec::len).max().unwrap_or(0);
    let mut ids = vec![loaded.vocab.pad; batch * seq];
    let mut mask = vec![0_i64; batch * seq];
    for (row, encoding) in encodings.iter().enumerate() {
        for (col, id) in encoding.iter().enumerate() {
            if let Some(slot) = ids.get_mut(row * seq + col) {
                *slot = *id;
            }
            if let Some(slot) = mask.get_mut(row * seq + col) {
                *slot = 1;
            }
        }
    }
    let types = vec![0_i64; batch * seq];
    let tensor = |data: &[i64]| Tensor::from_shape(&[batch, seq], data).map_err(model_error);
    let mut values = tvec![];
    for input in &loaded.inputs {
        let t = match input {
            Input::Ids => tensor(&ids)?,
            Input::Mask => tensor(&mask)?,
            Input::TypeIds => tensor(&types)?,
        };
        values.push(TValue::from(t));
    }
    let outputs = loaded.model.run(values).map_err(model_error)?;
    let output = outputs
        .first()
        .ok_or_else(|| SearchError::Embedding("the model returned nothing".into()))?;
    let view = output.to_plain_array_view::<f32>().map_err(model_error)?;
    let shape = view.shape().to_vec();
    let flat: Vec<f32> = view.iter().copied().collect();
    let mut out = Vec::with_capacity(batch);
    match shape.as_slice() {
        // Token states: mean-pool over real tokens.
        [b, s, d] if *b == batch && *s == seq && *d == dimensions => {
            for row in 0..batch {
                let mut pooled = vec![0.0_f32; dimensions];
                let mut count = 0.0_f32;
                for col in 0..seq {
                    if mask.get(row * seq + col) != Some(&1) {
                        continue;
                    }
                    count += 1.0;
                    let start = (row * seq + col) * dimensions;
                    for (acc, x) in pooled
                        .iter_mut()
                        .zip(flat.get(start..start + dimensions).unwrap_or_default())
                    {
                        *acc += x;
                    }
                }
                if count > 0.0 {
                    for x in &mut pooled {
                        *x /= count;
                    }
                }
                vector::normalise(&mut pooled);
                out.push(pooled);
            }
        }
        // Already pooled.
        [b, d] if *b == batch && *d == dimensions => {
            for row in flat.chunks(dimensions) {
                let mut v = row.to_vec();
                vector::normalise(&mut v);
                out.push(v);
            }
        }
        other => {
            return Err(SearchError::Embedding(format!(
                "model output shape {other:?} does not match batch {batch} × {dimensions} dimensions"
            )));
        }
    }
    Ok(out)
}

#[async_trait]
impl Embedder for OnnxEmbedder {
    fn model_id(&self) -> &str {
        &self.config.model_id
    }

    fn dimensions(&self) -> usize {
        self.config.dimensions
    }

    async fn embed_passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let prefix = &self.config.passage_prefix;
        self.embed(texts.iter().map(|t| format!("{prefix}{t}")).collect())
            .await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        let prefixed = format!("{}{text}", self.config.query_prefix);
        self.embed(vec![prefixed])
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| SearchError::Embedding("no query vector".into()))
    }
}

#[cfg(test)]
mod tests;
