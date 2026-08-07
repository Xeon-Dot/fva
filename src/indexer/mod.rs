//! Background indexer: file watching, AST parsing, chunking, embedding, graph.

pub mod chunker;
pub mod parser;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ignore::WalkBuilder;
use parking_lot::RwLock;
use rayon::prelude::*;

use self::chunker::{chunk_file, CodeChunk};
use self::parser::{AstParser, is_indexable};
use self::store::{ChunkStore, IndexStats, safe_relative_path};
use crate::config::IndexerConfig;
use crate::embedding::Embedder;
use crate::error::{FvaError, Result};
use crate::graph::CallGraphStore;
use crate::vector::VectorStore;

/// Shared indexer state.
#[derive(Clone)]
pub struct Indexer {
    root: PathBuf,
    config: IndexerConfig,
    sandbox: bool,
    store: Arc<ChunkStore>,
    parser: Arc<RwLock<AstParser>>,
    scanning: Arc<RwLock<bool>>,
    embedder: Arc<dyn Embedder>,
    vectors: Arc<dyn VectorStore>,
    graph: Arc<CallGraphStore>,
}

/// Sync phase result: one entry per indexed file, ready for the async commit phase.
type ParsedFile = (String, blake3::Hash, Vec<CodeChunk>, Vec<Vec<f32>>);

impl Indexer {
    pub fn new(
        root: PathBuf,
        config: IndexerConfig,
        sandbox: bool,
        embedder: Arc<dyn Embedder>,
        vectors: Arc<dyn VectorStore>,
        graph: Arc<CallGraphStore>,
    ) -> Self {
        let root = dunce::canonicalize(&root).unwrap_or(root);
        let root = dunce::simplified(&root).to_path_buf();
        Self {
            root,
            config,
            sandbox,
            store: Arc::new(ChunkStore::new()),
            parser: Arc::new(RwLock::new(AstParser::new())),
            scanning: Arc::new(RwLock::new(false)),
            embedder,
            vectors,
            graph,
        }
    }

    pub fn store(&self) -> Arc<ChunkStore> {
        self.store.clone()
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn is_scanning(&self) -> bool {
        *self.scanning.read()
    }

    pub fn stats(&self) -> IndexStats {
        self.store.stats()
    }

    /// Full index scan of the project.
    pub async fn index_all(&self) -> Result<usize> {
        *self.scanning.write() = true;
        let result = self.index_all_inner().await;
        *self.scanning.write() = false;
        if result.is_ok() {
            let _ = self.vectors.persist();
            let _ = self.graph.persist();
        }
        result
    }

    async fn index_all_inner(&self) -> Result<usize> {
        let files = self.collect_files()?;
        tracing::info!("indexing {} source files", files.len());

        // Phase 1 (rayon, sync, CPU-bound): read -> parse -> chunk -> embed
        let parsed: Vec<ParsedFile> = files
            .par_iter()
            .filter_map(|f| {
                match self.parse_and_embed(f) {
                    Ok(Some(v)) => Some(v),
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!("failed to index {}: {e}", f.display());
                        None
                    }
                }
            })
            .collect();

        // Phase 2 (async): per-file vector upserts, sequential
        for (relative, hash, chunks, vectors) in &parsed {
            self.commit_file(relative, hash, chunks, vectors).await;
        }

        tracing::info!(
            "indexed {} files, {} chunks, {} symbols, {} vectors, {} graph edges",
            self.store.stats().indexed_files,
            self.store.stats().total_chunks,
            self.store.stats().total_symbols,
            self.vectors.stats().total_vectors,
            self.graph.stats().edges
        );

        Ok(parsed.iter().map(|(_, _, c, _)| c.len()).sum())
    }

    fn parse_and_embed(
        &self,
        file_path: &Path,
    ) -> Result<Option<ParsedFile>> {
        if !is_indexable(file_path) {
            return Ok(None);
        }

        if self.sandbox && !file_path.starts_with(&self.root) {
            return Err(FvaError::Indexer(format!(
                "sandbox violation: {} outside {}",
                file_path.display(),
                self.root.display()
            )));
        }

        let relative = safe_relative_path(&self.root, file_path).ok_or_else(|| {
            FvaError::Indexer(format!("path outside root: {}", file_path.display()))
        })?;

        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        let content_hash = blake3::hash(source.as_bytes());
        if !self.store.needs_reindex(&relative, &content_hash) {
            return Ok(None);
        }

        let chunks = {
            let parser = self.parser.read();
            chunk_file(
                &parser,
                file_path,
                &relative,
                &source,
                self.config.max_file_size,
            )?
        };
        if chunks.is_empty() {
            return Ok(None);
        }
        let texts = crate::vector::chunk_texts(&chunks);
        let vectors = self.embedder.embed(&texts)?;
        Ok(Some((relative, content_hash, chunks, vectors)))
    }

    async fn commit_file(
        &self,
        relative: &str,
        hash: &blake3::Hash,
        chunks: &[CodeChunk],
        vectors: &[Vec<f32>],
    ) {
        let _ = self.vectors.remove_file(relative).await;
        self.store.upsert_file(relative, chunks.to_vec(), hash);
        let _ = self.vectors.upsert_chunks(chunks, vectors).await;
        let _ = self.graph.index_chunks(chunks);
    }

    /// Incrementally index a single file.
    pub async fn index_file(&self, file_path: &Path) -> Result<usize> {
        let Some((relative, hash, chunks, vectors)) = self.parse_and_embed(file_path)? else {
            return Ok(0);
        };
        self.commit_file(&relative, &hash, &chunks, &vectors).await;
        Ok(chunks.len())
    }

    pub fn collect_files(&self) -> Result<Vec<PathBuf>> {
        let mut builder = WalkBuilder::new(&self.root);
        builder.git_ignore(self.config.respect_gitignore);
        builder.git_global(self.config.respect_gitignore);
        builder.hidden(false);
        builder.follow_links(false);

        let mut files = Vec::new();
        for entry in builder.build().flatten() {
            let path = entry.path();
            if !path.is_file() || !is_indexable(path) {
                continue;
            }
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            if size <= self.config.max_file_size {
                files.push(path.to_path_buf());
            }
        }

        Ok(files)
    }

    /// Run initial index in background task.
    pub fn spawn_background_index(self: &Arc<Self>) {
        let indexer = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(e) = indexer.index_all().await {
                tracing::error!("background index failed: {e}");
            }
        });
    }

    /// Wait for background index to complete.
    pub fn wait_for_index(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while self.is_scanning() {
            if start.elapsed() > timeout {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        true
    }
}
