//! Vector storage for semantic code search.

#[cfg(feature = "lancedb")]
mod lancedb;

#[cfg(feature = "lancedb")]
pub use lancedb::LanceDbVectorStore;

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;

use crate::config::VectorConfig;
use crate::embedding::Embedder;
use crate::error::{FvaError, Result};
use crate::indexer::chunker::CodeChunk;

/// A vector search hit.
#[derive(Debug, Clone)]
pub struct VectorHit {
    pub chunk_id: String,
    pub relative_path: String,
    pub symbol_name: String,
    pub symbol_kind: String,
    pub language: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content_preview: String,
    pub score: f32,
}

#[derive(Debug, Clone, Default)]
pub struct VectorStats {
    pub total_vectors: usize,
    pub dimensions: usize,
}

/// Vector store trait.
///
/// Async because the native LanceDB backend exposes a tokio-based API.
/// Methods use explicit `Pin<Box<dyn Future + Send>>` signatures because
/// `async fn` in traits is not dyn-compatible on the current toolchain.
pub trait VectorStore: Send + Sync {
    fn upsert_chunks(
        &self,
        chunks: &[CodeChunk],
        vectors: &[Vec<f32>],
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
    fn remove_file(
        &self,
        relative_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>>;
    fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<VectorHit>>> + Send + '_>>;
    fn stats(&self) -> VectorStats;
    fn persist(&self) -> Result<()>;
}

/// Build vector store from configuration.
#[cfg_attr(not(feature = "lancedb"), allow(unused_variables))]
pub async fn build_vector_store(
    config: &VectorConfig,
    data_dir: &Path,
    dimensions: usize,
) -> Result<Arc<dyn VectorStore>> {
    match config.backend.as_str() {
        #[cfg(feature = "lancedb")]
        "lancedb" | "lancedb-native" => {
            let path = if Path::new(&config.db_path).is_absolute() {
                Path::new(&config.db_path).to_path_buf()
            } else {
                data_dir.join(&config.db_path)
            };
            Ok(Arc::new(LanceDbVectorStore::open(path, dimensions).await?))
        }
        #[cfg(not(feature = "lancedb"))]
        "lancedb" | "lancedb-native" => Err(FvaError::Config(
            "lancedb requires building with --features lancedb".into(),
        )),
        other => Err(FvaError::Config(format!("unknown vector backend: {other}"))),
    }
}

/// Build the embedding texts for a set of chunks.
pub(crate) fn chunk_texts(chunks: &[CodeChunk]) -> Vec<String> {
    chunks
        .iter()
        .map(|c| {
            format!(
                "{} {} {}\n{}",
                c.language, c.symbol_kind, c.symbol_name, c.content
            )
        })
        .collect()
}

/// Embed chunks and upsert into vector store.
pub async fn index_chunks(
    embedder: &dyn Embedder,
    store: &dyn VectorStore,
    chunks: &[CodeChunk],
) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }
    let texts = chunk_texts(chunks);
    let vectors = embedder.embed(&texts)?;
    store.upsert_chunks(chunks, &vectors).await?;
    Ok(chunks.len())
}
