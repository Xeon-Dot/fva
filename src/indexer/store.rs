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

fn remove_symbols_from_index(sym_index: &mut HashMap<String, Vec<String>>, chunks: &[CodeChunk]) {
    for chunk in chunks {
        if let Some(ids) = sym_index.get_mut(&chunk.symbol_name.to_lowercase()) {
            ids.retain(|id| id != &chunk.id);
            if ids.is_empty() {
                sym_index.remove(&chunk.symbol_name.to_lowercase());
            }
        }
    }
}

/// Thread-safe chunk store (Phase 1: in-memory; Phase 2+: persist to LanceDB).
#[derive(Default)]
pub struct ChunkStore {
    chunks_by_file: RwLock<HashMap<String, Vec<CodeChunk>>>,
    chunks_by_symbol: RwLock<HashMap<String, Vec<String>>>,
    chunks_by_id: RwLock<HashMap<String, CodeChunk>>,
    file_hashes: RwLock<HashMap<String, String>>,
    file_meta: RwLock<HashMap<String, FileIndexMeta>>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert_file(&self, relative_path: &str, chunks: Vec<CodeChunk>, content_hash: &Hash) {
        let hash_str = content_hash.to_hex().to_string();

        // Remove old symbol index entries for this file
        if let Some(old_chunks) = self.chunks_by_file.read().get(relative_path) {
            let mut sym_index = self.chunks_by_symbol.write();
            let mut id_index = self.chunks_by_id.write();
            remove_symbols_from_index(&mut sym_index, old_chunks);
            for old in old_chunks {
                id_index.remove(&old.id);
            }
        }

        // Update symbol index and id index
        {
            let mut sym_index = self.chunks_by_symbol.write();
            let mut id_index = self.chunks_by_id.write();
            for chunk in &chunks {
                sym_index
                    .entry(chunk.symbol_name.to_lowercase())
                    .or_default()
                    .push(chunk.id.clone());
                id_index.insert(chunk.id.clone(), chunk.clone());
            }
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

        self.chunks_by_file
            .write()
            .insert(relative_path.to_string(), chunks);
        self.file_hashes
            .write()
            .insert(relative_path.to_string(), hash_str);
        self.file_meta
            .write()
            .insert(relative_path.to_string(), meta);
    }

    pub fn remove_file(&self, relative_path: &str) {
        if let Some(chunks) = self.chunks_by_file.write().remove(relative_path) {
            let mut sym_index = self.chunks_by_symbol.write();
            let mut id_index = self.chunks_by_id.write();
            remove_symbols_from_index(&mut sym_index, &chunks);
            for chunk in &chunks {
                id_index.remove(&chunk.id);
            }
        }
        self.file_hashes.write().remove(relative_path);
        self.file_meta.write().remove(relative_path);
    }

    pub fn needs_reindex(&self, relative_path: &str, content_hash: &Hash) -> bool {
        let hash_str = content_hash.to_hex().to_string();
        self.file_hashes
            .read()
            .get(relative_path)
            .map(|h| h != &hash_str)
            .unwrap_or(true)
    }

    pub fn chunks_for_file(&self, relative_path: &str) -> Vec<CodeChunk> {
        self.chunks_by_file
            .read()
            .get(relative_path)
            .cloned()
            .unwrap_or_default()
    }

    pub fn find_symbol(&self, symbol: &str) -> Vec<CodeChunk> {
        let key = symbol.to_lowercase();
        let ids = self
            .chunks_by_symbol
            .read()
            .get(&key)
            .cloned()
            .unwrap_or_default();

        let id_index = self.chunks_by_id.read();
        ids.iter()
            .filter_map(|id| id_index.get(id).cloned())
            .collect()
    }

    /// O(1) chunk lookup by ID.
    pub fn chunk_by_id(&self, chunk_id: &str) -> Option<CodeChunk> {
        self.chunks_by_id.read().get(chunk_id).cloned()
    }

    /// Search chunks by text match with a limit.
    /// Uses the symbol index first for faster matching where possible,
    /// then falls back to content scan for remaining results.
    pub fn search_chunks(&self, query: &str, limit: usize) -> Vec<CodeChunk> {
        let query_lower = query.to_lowercase();
        if limit == 0 {
            return vec![];
        }

        let id_index = self.chunks_by_id.read();

        // Phase 1: check symbol index first (fast path)
        if let Some(symbol_ids) = self.chunks_by_symbol.read().get(&query_lower) {
            let mut results: Vec<CodeChunk> = symbol_ids
                .iter()
                .filter_map(|id| id_index.get(id).cloned())
                .collect();
            if results.len() >= limit {
                results.truncate(limit);
                return results;
            }

            // Phase 2: content scan for remaining
            let existing: std::collections::HashSet<&str> =
                results.iter().map(|c| c.id.as_str()).collect();
            let remaining = limit - results.len();

            let files = self.chunks_by_file.read();
            let mut extra: Vec<CodeChunk> = files
                .values()
                .flatten()
                .filter(|c| {
                    !existing.contains(c.id.as_str())
                        && (c.content.to_lowercase().contains(&query_lower)
                            || c.relative_path.to_lowercase().contains(&query_lower))
                })
                .take(remaining)
                .cloned()
                .collect();
            results.append(&mut extra);
            return results;
        }

        // Phase 2 only: no symbol match found, content scan only
        let files = self.chunks_by_file.read();
        files
            .values()
            .flatten()
            .filter(|c| {
                c.symbol_name.to_lowercase().contains(&query_lower)
                    || c.content.to_lowercase().contains(&query_lower)
                    || c.relative_path.to_lowercase().contains(&query_lower)
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub fn all_chunks(&self) -> Vec<CodeChunk> {
        self.chunks_by_file
            .read()
            .values()
            .flatten()
            .cloned()
            .collect()
    }

    /// Clear content hashes so the next index pass re-processes all files (benchmark only).
    pub fn invalidate_hashes(&self) {
        self.file_hashes.write().clear();
    }

    pub fn stats(&self) -> IndexStats {
        let files = self.chunks_by_file.read();
        let total_chunks: usize = files.values().map(|v| v.len()).sum();
        let total_symbols = self.chunks_by_symbol.read().len();

        IndexStats {
            indexed_files: files.len(),
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
