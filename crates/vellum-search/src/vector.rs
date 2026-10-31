// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

//! Embeddings: the [`Embedder`] port and the compact vector encoding.
//!
//! Vectors are unit length, so cosine similarity is a dot product. Stored
//! vectors are int8-quantised with one f32 scale (`dims + 4` bytes), which
//! keeps a 384-dimension model at about 390 bytes per block.

use async_trait::async_trait;

use crate::error::{Result, SearchError};

/// Turns text into unit-length vectors.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// A stable identifier of the model, stored with the index so a model
    /// change triggers a rebuild.
    fn model_id(&self) -> &str;

    /// The vector length.
    fn dimensions(&self) -> usize;

    /// Embeds a batch of passages (blocks).
    async fn embed_passages(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Embeds a query.
    async fn embed_query(&self, text: &str) -> Result<Vec<f32>>;
}

/// Scales `vector` to unit length; a zero vector stays zero.
pub fn normalise(vector: &mut [f32]) {
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > f32::EPSILON {
        for x in vector.iter_mut() {
            *x /= norm;
        }
    }
}

/// Encodes a vector as int8 values followed by the f32 scale (little-endian).
#[must_use]
pub fn quantise(vector: &[f32]) -> Vec<u8> {
    let max = vector.iter().fold(0.0_f32, |m, x| m.max(x.abs()));
    let scale = if max > 0.0 { max / 127.0 } else { 1.0 };
    let mut out = Vec::with_capacity(vector.len() + 4);
    for x in vector {
        // Clamped to the i8 range first, so the casts cannot wrap.
        #[allow(clippy::cast_possible_truncation)]
        let q = (x / scale).round().clamp(-127.0, 127.0) as i8;
        out.push(q.to_le_bytes()[0]);
    }
    out.extend_from_slice(&scale.to_le_bytes());
    out
}

/// The dot product of a query vector with a quantised stored vector.
///
/// # Errors
/// [`SearchError::Corrupt`] when the stored length does not match.
pub fn dot_quantised(query: &[f32], stored: &[u8]) -> Result<f32> {
    let (values, scale) = stored
        .split_last_chunk::<4>()
        .ok_or_else(|| SearchError::Corrupt("vector too short".into()))?;
    if values.len() != query.len() {
        return Err(SearchError::Corrupt(format!(
            "vector has {} dimensions, query has {}",
            values.len(),
            query.len()
        )));
    }
    let scale = f32::from_le_bytes(*scale);
    let sum: f32 = values
        .iter()
        .zip(query)
        .map(|(q, x)| f32::from(i8::from_le_bytes([*q])) * x)
        .sum();
    Ok(sum * scale)
}

/// Checks that a batch of vectors matches the expected shape.
///
/// # Errors
/// [`SearchError::Embedding`] on a count or dimension mismatch.
pub fn check_shape(
    vectors: &[Vec<f32>],
    count: usize,
    dimensions: usize,
) -> Result<()> {
    if vectors.len() != count {
        return Err(SearchError::Embedding(format!(
            "expected {count} vectors, got {}",
            vectors.len()
        )));
    }
    if let Some(bad) = vectors.iter().find(|v| v.len() != dimensions) {
        return Err(SearchError::Embedding(format!(
            "expected {dimensions} dimensions, got {}",
            bad.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn normalise_makes_unit_vectors_and_keeps_zero() {
        let mut v = vec![3.0, 4.0];
        normalise(&mut v);
        assert!((v[0] - 0.6).abs() < 1e-6 && (v[1] - 0.8).abs() < 1e-6);
        let mut zero = vec![0.0; 3];
        normalise(&mut zero);
        assert_eq!(zero, [0.0; 3]);
    }

    #[test]
    fn wrong_lengths_are_reported() {
        assert!(dot_quantised(&[1.0], &[1, 2]).is_err());
        assert!(dot_quantised(&[1.0, 2.0], &quantise(&[1.0])).is_err());
        assert!(check_shape(&[vec![1.0]], 2, 1).is_err());
        assert!(check_shape(&[vec![1.0]], 1, 2).is_err());
        assert!(check_shape(&[vec![1.0]], 1, 1).is_ok());
    }

    proptest! {
        #[test]
        fn quantised_dot_is_close_to_exact(
            a in prop::collection::vec(-1.0_f32..1.0, 16),
            b in prop::collection::vec(-1.0_f32..1.0, 16),
        ) {
            let mut a = a;
            let mut b = b;
            normalise(&mut a);
            normalise(&mut b);
            let exact: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
            let approx = dot_quantised(&a, &quantise(&b)).unwrap();
            prop_assert!((exact - approx).abs() < 0.03, "{exact} vs {approx}");
        }
    }
}
