//! Flat in-memory vector store with disk persistence.
//!
//! Uses brute-force cosine similarity — fast for <100k chunks,
//! zero external dependencies, LanceDB-compatible interface.

use std::collections::HashMap;
use std::path::PathBuf;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::VectorStore;
use crate::embedding::cosine_similarity;
use crate::error::{FvaError, Result};
use crate::indexer::chunker::CodeChunk;
use crate::util::HasScore;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredVector {
    chunk_id: String,
    relative_path: String,
    symbol_name: String,
    symbol_kind: String,
    language: String,
    start_line: usize,
    end_line: usize,
    content_preview: String,
    vector: Vec<f32>,
}

impl StoredVector {
    fn from_chunk(chunk: &CodeChunk, vector: Vec<f32>) -> Self {
        Self {
            chunk_id: chunk.id.clone(),
            relative_path: chunk.relative_path.clone(),
            symbol_name: chunk.symbol_name.clone(),
            symbol_kind: chunk.symbol_kind.clone(),
            language: chunk.language.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            content_preview: FlatVectorStore::preview(&chunk.content, 200),
            vector,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct VectorSnapshot {
    dimensions: usize,
    entries: Vec<StoredVector>,
}

/// A vector search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl From<&StoredVector> for VectorHit {
    fn from(s: &StoredVector) -> Self {
        Self {
            chunk_id: s.chunk_id.clone(),
            relative_path: s.relative_path.clone(),
            symbol_name: s.symbol_name.clone(),
            symbol_kind: s.symbol_kind.clone(),
            language: s.language.clone(),
            start_line: s.start_line,
            end_line: s.end_line,
            content_preview: s.content_preview.clone(),
            score: 0.0,
        }
    }
}

impl HasScore for VectorHit {
    fn score(&self) -> f32 {
        self.score
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VectorStats {
    pub total_vectors: usize,
    pub dimensions: usize,
}

pub struct FlatVectorStore {
    path: PathBuf,
    dimensions: usize,
    entries: RwLock<Vec<StoredVector>>,
    by_file: RwLock<HashMap<String, Vec<usize>>>,
}

impl FlatVectorStore {
    pub fn open(path: PathBuf, dimensions: usize) -> Result<Self> {
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;

        let store = Self {
            path: path.clone(),
            dimensions,
            entries: RwLock::new(Vec::new()),
            by_file: RwLock::new(HashMap::new()),
        };

        let data_file = path.join("vectors.bin");
        if data_file.exists()
            && let Ok(bytes) = std::fs::read(&data_file)
            && let Ok(snapshot) = bincode::deserialize::<VectorSnapshot>(&bytes)
        {
            if snapshot.dimensions == dimensions {
                store.load_snapshot(snapshot);
                tracing::info!(
                    "loaded {} vectors from {}",
                    store.entries.read().len(),
                    data_file.display()
                );
            } else {
                tracing::warn!(
                    "vector dimensions changed ({} -> {}), re-indexing required",
                    snapshot.dimensions,
                    dimensions
                );
            }
        }

        Ok(store)
    }

    fn load_snapshot(&self, snapshot: VectorSnapshot) {
        let mut by_file: HashMap<String, Vec<usize>> = HashMap::new();
        for (idx, entry) in snapshot.entries.iter().enumerate() {
            by_file
                .entry(entry.relative_path.clone())
                .or_default()
                .push(idx);
        }
        *self.entries.write() = snapshot.entries;
        *self.by_file.write() = by_file;
    }

    fn preview(content: &str, max_len: usize) -> String {
        if content.len() <= max_len {
            return content.to_string();
        }
        let mut end = max_len.min(content.len());
        while end > 0 && !content.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &content[..end])
    }
}

impl VectorStore for FlatVectorStore {
    fn upsert_chunks(&self, chunks: &[CodeChunk], vectors: &[Vec<f32>]) -> Result<()> {
        if chunks.len() != vectors.len() {
            return Err(FvaError::Other(format!(
                "chunk/vector count mismatch: {} vs {}",
                chunks.len(),
                vectors.len()
            )));
        }

        if let Some(path) = chunks.first().map(|c| c.relative_path.clone()) {
            self.remove_file(&path)?;
        }

        let mut entries = self.entries.write();
        let mut by_file = self.by_file.write();

        for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
            if vector.len() != self.dimensions {
                return Err(FvaError::Other(format!(
                    "vector dimension mismatch: expected {}, got {}",
                    self.dimensions,
                    vector.len()
                )));
            }

            let idx = entries.len();
            entries.push(StoredVector::from_chunk(chunk, vector.clone()));
            by_file
                .entry(chunk.relative_path.clone())
                .or_default()
                .push(idx);
        }

        Ok(())
    }

    fn remove_file(&self, relative_path: &str) -> Result<()> {
        let mut entries = self.entries.write();
        let mut by_file = self.by_file.write();

        // O(1) lookup: if the file isn't tracked, nothing to do
        let Some(mut indices) = by_file.remove(relative_path) else {
            return Ok(());
        };

        // Remove entries in reverse index order so earlier removals don't
        // shift indices we haven't processed yet. Use swap_remove (O(1))
        // instead of retain (O(n)), fixing up by_file for any moved entry.
        indices.sort_unstable();
        for &idx in indices.iter().rev() {
            let last = entries.len() - 1;
            if idx == last {
                entries.pop();
            } else {
                entries.swap_remove(idx);
                // The entry that was at `last` is now at `idx` — update its
                // by_file index so future lookups are correct.
                let moved_path = entries[idx].relative_path.clone();
                if let Some(moved_indices) = by_file.get_mut(&moved_path)
                    && let Some(pos) = moved_indices.iter().position(|i| *i == last)
                {
                    moved_indices[pos] = idx;
                }
            }
        }

        Ok(())
    }

    fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>> {
        let entries = self.entries.read();
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        // Compute scores only → find top-k indices → materialise only those hits.
        // This avoids allocating full VectorHit objects (with string clones) for
        // every entry when only a fraction are returned.
        let n = entries.len();
        let mut scored: Vec<(f32, usize)> = Vec::with_capacity(n);
        for (i, e) in entries.iter().enumerate() {
            scored.push((cosine_similarity(query_vector, &e.vector), i));
        }

        // Partial select: pivot at n-k so positions n-k..n hold the top-k scores
        let k = limit.min(n);
        scored.select_nth_unstable_by(n - k, |a, b| {
            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Sort only the top-k portion descending by score
        scored[n - k..]
            .sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        // Materialise only top-k VectorHit objects (avoids N string clones)
        let mut hits: Vec<VectorHit> = Vec::with_capacity(k);
        for &(score, idx) in &scored[n - k..] {
            let e = &entries[idx];
            hits.push(VectorHit {
                chunk_id: e.chunk_id.clone(),
                relative_path: e.relative_path.clone(),
                symbol_name: e.symbol_name.clone(),
                symbol_kind: e.symbol_kind.clone(),
                language: e.language.clone(),
                start_line: e.start_line,
                end_line: e.end_line,
                content_preview: e.content_preview.clone(),
                score,
            });
        }

        Ok(hits)
    }

    fn stats(&self) -> VectorStats {
        VectorStats {
            total_vectors: self.entries.read().len(),
            dimensions: self.dimensions,
        }
    }

    fn persist(&self) -> Result<()> {
        let snapshot = VectorSnapshot {
            dimensions: self.dimensions,
            entries: self.entries.read().clone(),
        };
        let bytes = bincode::serialize(&snapshot)
            .map_err(|e| FvaError::Other(format!("vector serialize: {e}")))?;
        let data_file = self.path.join("vectors.bin");
        std::fs::write(&data_file, bytes)?;
        Ok(())
    }
}
