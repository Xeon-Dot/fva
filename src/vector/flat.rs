//! Flat in-memory vector store with disk persistence.
//!
//! Uses brute-force cosine similarity — fast for <100k chunks,
//! zero external dependencies, LanceDB-compatible interface.
//!
//! Features a token-based inverted index for pre-filtering:
//! query tokens are hashed into buckets, and only entries sharing
//! at least one bucket with the query are scored.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use parking_lot::RwLock;
use rayon::prelude::*;
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

// ---------------------------------------------------------------------------
// Inverted-index helpers — same tokenisation scheme as LocalEmbedder
// ---------------------------------------------------------------------------

/// Extract feature-hash values from a piece of text, using the same
/// approach as `LocalEmbedder::hash_embed` so that the inverted index
/// and embedding space agree on which tokens are important.
fn token_hashes(text: &str) -> Vec<u64> {
    let lower = text.to_lowercase();
    let mut hashes = Vec::new();

    // Word-level tokens (same split as LocalEmbedder)
    for token in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
        if token.len() < 2 {
            continue;
        }
        // Multi-hash: two different hash values per token for better bucket
        // distribution and fewer false-positive collisions.
        let mut h1 = DefaultHasher::new();
        token.hash(&mut h1);
        hashes.push(h1.finish());

        let mut h2 = DefaultHasher::new();
        0x42u8.hash(&mut h2);
        token.hash(&mut h2);
        hashes.push(h2.finish());
    }

    // Character trigrams for typo tolerance
    let bytes = lower.as_bytes();
    if bytes.len() >= 3 {
        for w in bytes.windows(3) {
            let mut h = DefaultHasher::new();
            w.hash(&mut h);
            hashes.push(h.finish());
        }
    }

    hashes
}

// ---------------------------------------------------------------------------
// FlatVectorStore
// ---------------------------------------------------------------------------

pub struct FlatVectorStore {
    path: PathBuf,
    dimensions: usize,
    entries: RwLock<Vec<StoredVector>>,
    by_file: RwLock<HashMap<String, Vec<usize>>>,
    /// Token-based inverted index: hash(token) → entry indices that contain
    /// that token. Used by `search_with_text` for pre-filtering candidates.
    token_index: RwLock<HashMap<u64, Vec<usize>>>,
}

impl FlatVectorStore {
    /// Open (or create) a persistent vector store at `path`.
    ///
    /// If a `vectors.bin` snapshot exists and its dimension matches, it is
    /// loaded and both `by_file` and `token_index` are rebuilt from it.
    pub fn open(path: PathBuf, dimensions: usize) -> Result<Self> {
        std::fs::create_dir_all(path.parent().unwrap_or(&path))?;

        let store = Self {
            path: path.clone(),
            dimensions,
            entries: RwLock::new(Vec::new()),
            by_file: RwLock::new(HashMap::new()),
            token_index: RwLock::new(HashMap::new()),
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

    /// Populate in-memory state from a deserialised snapshot, rebuilding
    /// both the file index and the token inverted index.
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
        self.rebuild_token_index();
    }

    /// Rebuild the token inverted index from all current entries.
    fn rebuild_token_index(&self) {
        let entries = self.entries.read();
        let mut token_idx = self.token_index.write();
        token_idx.clear();
        for (entry_idx, entry) in entries.iter().enumerate() {
            for hash in token_hashes(&format!(
                "{} {} {}",
                entry.symbol_name, entry.content_preview, entry.relative_path
            )) {
                token_idx.entry(hash).or_default().push(entry_idx);
            }
        }
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

// ---------------------------------------------------------------------------
// VectorStore trait implementation
// ---------------------------------------------------------------------------

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
        let mut token_idx = self.token_index.write();

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

            // Index tokens for this chunk — same hashing as LocalEmbedder.
            for hash in token_hashes(&format!(
                "{} {} {}",
                chunk.symbol_name, chunk.symbol_kind, chunk.relative_path
            )) {
                token_idx.entry(hash).or_default().push(idx);
            }
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

        // Release guards so rebuild can acquire its own locks
        drop(entries);
        drop(by_file);

        // Rebuild the token index — the swap_remove loop above may have
        // shifted entry indices, making the old token_index stale.
        self.rebuild_token_index();

        Ok(())
    }

    fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>> {
        let entries = self.entries.read();
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let n = entries.len();
        let query = query_vector;

        // Parallel score computation using rayon
        let mut scored: Vec<(f32, usize)> = Vec::with_capacity(n);
        {
            let scores: Vec<f32> = entries
                .par_iter()
                .map(|e| cosine_similarity(query, &e.vector))
                .collect();
            for (i, score) in scores.into_iter().enumerate() {
                scored.push((score, i));
            }
        }

        Ok(reduce_top_k(scored, &entries, limit))
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

// ---------------------------------------------------------------------------
// Extended methods on FlatVectorStore
// ---------------------------------------------------------------------------

impl FlatVectorStore {
    /// Search with token-index pre-filtering.
    ///
    /// When the inverted index is populated, this method first narrows
    /// candidates to entries whose token buckets overlap with the query
    /// text, then computes cosine similarity only for those candidates.
    /// This dramatically reduces the number of distance computations for
    /// focused queries (often 10-20x fewer entries to score).
    ///
    /// Falls back to a full scan (via `search()`) when:
    ///   * the token index is empty (first use after creation, or loaded
    ///     from old persistence that didn't include the index)
    ///   * too many entries match the query tokens (>80% of total), since
    ///     the pre-filter overhead wouldn't be worth it
    ///   * no candidates matched at all
    pub fn search_with_text(
        &self,
        query_text: &str,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<VectorHit>> {
        let entries = self.entries.read();
        if entries.is_empty() {
            return Ok(Vec::new());
        }

        let n = entries.len();
        let token_idx = self.token_index.read();

        // Graceful fallback: token index hasn't been built yet (first use
        // after creation or loaded from old persistence without index).
        if token_idx.is_empty() {
            drop(token_idx);
            return self.search(query_vector, limit);
        }

        // Pre-filter: union all entry indices that share token buckets
        // with the query text.
        let query_hashes = token_hashes(query_text);
        let mut candidate_set: Vec<usize> = Vec::new();
        for hash in &query_hashes {
            if let Some(indices) = token_idx.get(hash) {
                candidate_set.extend_from_slice(indices);
            }
        }
        candidate_set.sort_unstable();
        candidate_set.dedup();

        let m = candidate_set.len();

        // If too many candidates match (or everything matched), fall back
        // to the full parallel scan — the pre-filter overhead isn't worth it.
        if m > n * 80 / 100 || m == n || m == 0 {
            drop(token_idx);
            return self.search(query_vector, limit);
        }

        drop(token_idx); // Release before parallel iteration

        // Score only the pre-filtered candidates in parallel
        let mut scored: Vec<(f32, usize)> = Vec::with_capacity(m);
        {
            let scores: Vec<f32> = candidate_set
                .par_iter()
                .map(|&idx| cosine_similarity(query_vector, &entries[idx].vector))
                .collect();
            for (&idx, &score) in candidate_set.iter().zip(scores.iter()) {
                scored.push((score, idx));
            }
        }

        Ok(reduce_top_k(scored, &entries, limit))
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Select the top-k scored entries and materialise `VectorHit` objects.
///
/// Uses `select_nth_unstable_by` for an O(m) partial sort, then sorts only
/// the k-sized suffix descending by score. Avoids allocating full
/// `VectorHit` objects (with string clones) for every entry.
fn reduce_top_k(
    mut scored: Vec<(f32, usize)>,
    entries: &[StoredVector],
    limit: usize,
) -> Vec<VectorHit> {
    let m = scored.len();
    let k = limit.min(m);

    if k == 0 {
        return Vec::new();
    }

    // Partial select: positions [m-k..) hold the top-k scores
    scored.select_nth_unstable_by(m - k, |a, b| {
        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Sort the k-window descending by score
    scored[m - k..]
        .sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Materialise only the top k hits
    let mut hits: Vec<VectorHit> = Vec::with_capacity(k);
    for &(score, idx) in &scored[m - k..] {
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

    hits
}
