//! Embedding providers for semantic code search.

mod local;
mod voyage;

use std::sync::Arc;

pub use local::LocalEmbedder;
pub use voyage::VoyageEmbedder;

use crate::config::EmbeddingConfig;
use crate::error::{FvaError, Result};

/// Embedding provider trait.
pub trait Embedder: Send + Sync {
    fn name(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// Embed a single text, returning one vector.
    fn embed_one(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed(&[text.to_string()])?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| FvaError::Other("empty embedding result".into()))
    }
}

/// Build embedder from configuration.
pub fn build_embedder(config: &EmbeddingConfig) -> Result<Arc<dyn Embedder>> {
    let api_key = if config.voyage_api_key.is_empty() {
        std::env::var("VOYAGE_API_KEY").unwrap_or_default()
    } else {
        config.voyage_api_key.clone()
    };

    let embedder: Arc<dyn Embedder> = match config.provider.as_str() {
        "voyage" if !api_key.is_empty() => Arc::new(VoyageEmbedder::new(
            api_key,
            config.model.clone(),
            config.dimensions,
        )?),
        "voyage" => {
            tracing::warn!("voyage provider requested but no API key — falling back to local");
            Arc::new(LocalEmbedder::new(config.dimensions))
        }
        "local" | "none" | "hash" => Arc::new(LocalEmbedder::new(config.dimensions)),
        other => {
            return Err(FvaError::Config(format!(
                "unknown embedding provider: {other}"
            )));
        }
    };

    Ok(embedder)
}

/// L2-normalize a vector in place.
pub fn normalize(v: &mut [f32]) {
    let norm: f32 = dot_product(v, v).sqrt();
    if norm > 1e-10 {
        let inv = 1.0 / norm;
        for x in v.iter_mut() {
            *x *= inv;
        }
    }
}

/// Dot product of two f32 slices (auto-vectorized, 8-wide unrolled).
fn dot_product(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let mut sum = 0.0f32;
    let mut i = 0;

    // 8-wide unrolled loop — helps LLVM generate SIMD
    while i + 8 <= n {
        sum += a[i] * b[i]
            + a[i + 1] * b[i + 1]
            + a[i + 2] * b[i + 2]
            + a[i + 3] * b[i + 3]
            + a[i + 4] * b[i + 4]
            + a[i + 5] * b[i + 5]
            + a[i + 6] * b[i + 6]
            + a[i + 7] * b[i + 7];
        i += 8;
    }
    // Remainder
    while i < n {
        sum += a[i] * b[i];
        i += 1;
    }
    sum
}

/// Cosine similarity between two L2-normalized vectors.
/// Assumes both vectors are pre-normalized; this is just a dot product.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    dot_product(a, b)
}
