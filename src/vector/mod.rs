//! Vector storage for semantic code search.

mod flat;

pub use flat::{FlatVectorStore, VectorHit, VectorStats};

use std::path::Path;
use std::sync::Arc;

use crate::config::VectorConfig;
use crate::embedding::Embedder;
use crate::error::{FvaError, Result};
use crate::indexer::chunker::CodeChunk;

/// Vector store trait.
pub trait VectorStore: Send + Sync {
    fn upsert_chunks(&self, chunks: &[CodeChunk], vectors: &[Vec<f32>]) -> Result<()>;
    fn remove_file(&self, relative_path: &str) -> Result<()>;
    fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>>;
    fn stats(&self) -> VectorStats;
    fn persist(&self) -> Result<()>;
}

/// Build vector store from configuration.
pub fn build_vector_store(
    config: &VectorConfig,
    data_dir: &Path,
    dimensions: usize,
) -> Result<Arc<dyn VectorStore>> {
    match config.backend.as_str() {
        "flat" | "lancedb" => {
            // LanceDB feature can be added later; flat store is the default production path.
            let path = if Path::new(&config.db_path).is_absolute() {
                Path::new(&config.db_path).to_path_buf()
            } else {
                data_dir.join(&config.db_path)
            };
            Ok(Arc::new(FlatVectorStore::open(path, dimensions)?))
        }
        #[cfg(feature = "lancedb")]
        "lancedb-native" => Err(FvaError::Other(
            "native LanceDB the model not yet wired — use the model = \"flat\"".into(),
        )),
        #[cfg(not(feature = "lancedb"))]
        "lancedb-native" => Err(FvaError::Config(
            "lancedb-native requires building with --features lancedb".into(),
        )),
        other => Err(FvaError::Config(format!("unknown vector the model: {other}"))),
    }
}

/// Embed chunks and upsert into vector store.
pub fn index_chunks(
    embedder: &dyn Embedder,
    store: &dyn VectorStore,
    chunks: &[CodeChunk],
) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }

    // Build rich embedding texts that encode as much signal as possible
    // for the (potentially hash-based) embedder to exploit.
    let texts: Vec<String> = chunks
        .iter()
        .map(|c| {
            // Replicate the symbol name and kind to increase their weight
            // in the embedding, since they are the most discriminative signals.
            let heading = format!(
                "{} {} {} in {}",
                c.symbol_kind, c.symbol_name, c.language, c.relative_path
            );

            // Extract the first line of content (signature line) as a separate
            // signal, since it often contains parameter types and return types
            // that are very informative.
            let first_line = c.content.lines().next().unwrap_or("");

            format!("{}\n{}\n{}", heading, first_line, c.content)
        })
        .collect();

    let vectors = embedder.embed(&texts)?;
    store.upsert_chunks(chunks, &vectors)?;
    Ok(chunks.len())
}
