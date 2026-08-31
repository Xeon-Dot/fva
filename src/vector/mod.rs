//! Vector storage for semantic code search.

#[cfg(feature = "lancedb")]
mod lancedb;

#[cfg(feature = "lancedb")]
pub use lancedb::LanceDbVectorStore;

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
