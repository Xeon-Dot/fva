//! In-memory chunk store with file-hash tracking for incremental updates.

use std::collections::HashMap;
use std::path::Path;

use blake3::Hash;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use super::chunker::CodeChunk;

/// Per-file index metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileIndexMeta {
    pub relative_path: String,
    pub content_hash: String,
    pub chunk_count: usize,
    pub language: String,
    pub indexed_at: u64,
}

#[derive(Default)]
struct ChunkStoreInner {
    chunks_by_file: HashMap<String, Vec<CodeChunk>>,
    chunks_by_symbol: HashMap<String, Vec<String>>,
    chunks_by_id: HashMap<String, CodeChunk>,
    file_hashes: HashMap<String, String>,
    file_meta: HashMap<String, FileIndexMeta>,
    search_blobs: HashMap<String, String>,
}

fn make_search_blob(chunk: &CodeChunk) -> String {
    format!("{} {} {}", chunk.symbol_name, chunk.relative_path, chunk.content).to_lowercase()
}

fn remove_chunk_from_indices(inner: &mut ChunkStoreInner, chunk: &CodeChunk) {
    let key = chunk.symbol_name.to_lowercase();
    if let Some(ids) = inner.chunks_by_symbol.get_mut(&key) {
        ids.retain(|id| id != &chunk.id);
        if ids.is_empty() {
            inner.chunks_by_symbol.remove(&key);
        }
    }
    inner.chunks_by_id.remove(&chunk.id);
    inner.search_blobs.remove(&chunk.id);
}

/// Thread-safe chunk store (Phase 1: in-memory; Phase 2+: persist to LanceDB).
#[derive(Default)]
pub struct ChunkStore {
    inner: RwLock<ChunkStoreInner>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_file(&self, relative_path: &str, chunks: Vec<CodeChunk>, content_hash: &Hash) {
        let hash_str = content_hash.to_hex().to_string();
        let mut inner = self.inner.write();

        if let Some(old_chunks) = inner.chunks_by_file.get(relative_path) {
            let old_ids: Vec<(String, String)> = old_chunks
                .iter()
                .map(|c| (c.id.clone(), c.symbol_name.to_lowercase()))
                .collect();
            for (id, sym_key) in &old_ids {
                if let Some(ids) = inner.chunks_by_symbol.get_mut(sym_key) {
                    ids.retain(|i| i != id);
                    if ids.is_empty() {
                        inner.chunks_by_symbol.remove(sym_key);
                    }
                }
                inner.chunks_by_id.remove(id);
                inner.search_blobs.remove(id);
            }
        }

        for chunk in &chunks {
            inner
                .chunks_by_symbol
                .entry(chunk.symbol_name.to_lowercase())
                .or_default()
                .push(chunk.id.clone());
            inner
                .search_blobs
                .insert(chunk.id.clone(), make_search_blob(chunk));
            inner.chunks_by_id.insert(chunk.id.clone(), chunk.clone());
        }

        let language = chunks
            .first()
            .map(|c| c.language.clone())
            .unwrap_or_else(|| "unknown".to_string());

        let meta = FileIndexMeta {
            relative_path: relative_path.to_string(),
            content_hash: hash_str.clone(),
            chunk_count: chunks.len(),
            language,
            indexed_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };

        inner
            .chunks_by_file
            .insert(relative_path.to_string(), chunks);
        inner.file_hashes.insert(relative_path.to_string(), hash_str);
        inner.file_meta.insert(relative_path.to_string(), meta);
    }

    pub fn remove_file(&self, relative_path: &str) {
        let mut inner = self.inner.write();
        if let Some(chunks) = inner.chunks_by_file.remove(relative_path) {
            for chunk in &chunks {
                remove_chunk_from_indices(&mut inner, chunk);
            }
        }
        inner.file_hashes.remove(relative_path);
        inner.file_meta.remove(relative_path);
    }

    pub fn needs_reindex(&self, relative_path: &str, content_hash: &Hash) -> bool {
        let hash_str = content_hash.to_hex().to_string();
        self.inner
            .read()
            .file_hashes
            .get(relative_path)
            .map(|h| h != &hash_str)
            .unwrap_or(true)
    }

    pub fn chunks_for_file(&self, relative_path: &str) -> Vec<CodeChunk> {
        self.inner
            .read()
            .chunks_by_file
            .get(relative_path)
            .cloned()
            .unwrap_or_default()
    }

    pub fn find_symbol(&self, symbol: &str) -> Vec<CodeChunk> {
        let key = symbol.to_lowercase();
        let inner = self.inner.read();
        let Some(ids) = inner.chunks_by_symbol.get(&key) else {
            return Vec::new();
        };
        ids.iter()
            .filter_map(|id| inner.chunks_by_id.get(id).cloned())
            .collect()
    }

    /// O(1) chunk lookup by ID.
    pub fn chunk_by_id(&self, chunk_id: &str) -> Option<CodeChunk> {
        self.inner.read().chunks_by_id.get(chunk_id).cloned()
    }

    pub fn search_chunks(&self, query: &str) -> Vec<CodeChunk> {
        let query_lower = query.to_lowercase();
        let inner = self.inner.read();

        inner
            .chunks_by_file
            .values()
            .flatten()
            .filter(|c| {
                inner
                    .search_blobs
                    .get(&c.id)
                    .map(|blob| blob.contains(&query_lower))
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    }

    pub fn all_chunks(&self) -> Vec<CodeChunk> {
        self.inner
            .read()
            .chunks_by_file
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// Clear content hashes so the next index pass re-processes all files (benchmark only).
    pub fn invalidate_hashes(&self) {
        self.inner.write().file_hashes.clear();
    }

    pub fn stats(&self) -> IndexStats {
        let inner = self.inner.read();
        let total_chunks: usize = inner.chunks_by_file.values().map(|v| v.len()).sum();
        let total_symbols = inner.chunks_by_symbol.len();

        IndexStats {
            indexed_files: inner.chunks_by_file.len(),
            total_chunks,
            total_symbols,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub indexed_files: usize,
    pub total_chunks: usize,
    pub total_symbols: usize,
}

/// Resolve a relative path safely within the project root.
pub fn safe_relative_path(root: &Path, file: &Path) -> Option<String> {
    let root_canon = dunce::canonicalize(root).ok()?;
    let file_canon = dunce::canonicalize(file).ok()?;
    let root = dunce::simplified(root_canon.as_path());
    let file = dunce::simplified(file_canon.as_path());
    file.strip_prefix(root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
}
