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

use self::chunker::chunk_file;
use self::parser::{AstParser, is_indexable};
use self::store::{ChunkStore, IndexStats, safe_relative_path};
use crate::config::IndexerConfig;
use crate::embedding::Embedder;
use crate::error::{FvaError, Result};
use crate::graph::CallGraphStore;
use crate::vector::{VectorStore, index_chunks};

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
    pub fn index_all(&self) -> Result<usize> {
        *self.scanning.write() = true;
        let result = self.index_all_inner();
        *self.scanning.write() = false;
        if result.is_ok() {
            let _ = self.vectors.persist();
            let _ = self.graph.persist();
        }
        result
    }

    fn index_all_inner(&self) -> Result<usize> {
        let files = self.collect_files()?;
        tracing::info!("indexing {} source files", files.len());

        let indexed: usize = files
            .par_iter()
            .map(|file_path| match self.index_file(file_path) {
                Ok(n) => n,
                Err(e) => {
                    tracing::warn!("failed to index {}: {e}", file_path.display());
                    0
                }
            })
            .sum();

        tracing::info!(
            "indexed {} files, {} chunks, {} symbols, {} vectors, {} graph edges",
            self.store.stats().indexed_files,
            self.store.stats().total_chunks,
            self.store.stats().total_symbols,
            self.vectors.stats().total_vectors,
            self.graph.stats().edges
        );

        Ok(indexed)
    }

    /// Incrementally index a single file.
    pub fn index_file(&self, file_path: &Path) -> Result<usize> {
        if !is_indexable(file_path) {
            return Ok(0);
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
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => return Ok(0),
            Err(e) => return Err(e.into()),
        };

        let content_hash = blake3::hash(source.as_bytes());
        if !self.store.needs_reindex(&relative, &content_hash) {
            return Ok(0);
        }

        // Remove stale vector/graph data
        let _ = self.vectors.remove_file(&relative);
        let _ = self.graph.remove_file(&relative);

        let parser = self.parser.read();
        let chunks = chunk_file(
            &parser,
            file_path,
            &relative,
            &source,
            self.config.max_file_size,
        )?;

        let count = chunks.len();
        self.store
            .upsert_file(&relative, chunks.clone(), &content_hash);

        // Phase 2: embed + vector index
        let _ = index_chunks(self.embedder.as_ref(), self.vectors.as_ref(), &chunks);

        // Phase 3: call graph
        let _ = self.graph.index_chunks(&chunks);

        Ok(count)
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

    /// Run initial index in background thread.
    pub fn spawn_background_index(self: &Arc<Self>) {
        let indexer = Arc::clone(self);
        std::thread::spawn(move || {
            if let Err(e) = indexer.index_all() {
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
