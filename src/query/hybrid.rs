//! Hybrid query engine: FFF + Vector + Graph fusion.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::config::QueryConfig;
use crate::embedding::Embedder;
use crate::fff::FffEngine;
use crate::graph::CallGraphStore;
use crate::indexer::chunker::CodeChunk;
use crate::indexer::store::ChunkStore;
use crate::util::{HasScore, sort_by_score};
use crate::vector::VectorStore;

/// A fused search result with multi-signal scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridHit {
    pub chunk_id: String,
    pub relative_path: String,
    pub symbol_name: String,
    pub symbol_kind: String,
    pub language: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub score: f32,
    pub fff_score: f32,
    pub vector_score: f32,
    pub graph_score: f32,
    pub sources: Vec<String>,
}

impl HybridHit {
    pub fn from_chunk(chunk: &CodeChunk) -> Self {
        Self {
            chunk_id: chunk.id.clone(),
            relative_path: chunk.relative_path.clone(),
            symbol_name: chunk.symbol_name.clone(),
            symbol_kind: chunk.symbol_kind.clone(),
            language: chunk.language.clone(),
            start_line: chunk.start_line,
            end_line: chunk.end_line,
            content: chunk.content.clone(),
            score: 0.0,
            fff_score: 0.0,
            vector_score: 0.0,
            graph_score: 0.0,
            sources: Vec::new(),
        }
    }
}

impl HasScore for HybridHit {
    fn score(&self) -> f32 {
        self.score
    }
}

/// Hybrid search response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HybridSearchResult {
    pub hits: Vec<HybridHit>,
    pub total_candidates: usize,
    pub query: String,
}

pub struct HybridQueryEngine {
    fff: FffEngine,
    store: Arc<ChunkStore>,
    vectors: Arc<dyn VectorStore>,
    graph: Arc<CallGraphStore>,
    embedder: Arc<dyn Embedder>,
    config: QueryConfig,
}

impl HybridQueryEngine {
    pub fn new(
        fff: FffEngine,
        store: Arc<ChunkStore>,
        vectors: Arc<dyn VectorStore>,
        graph: Arc<CallGraphStore>,
        embedder: Arc<dyn Embedder>,
        config: QueryConfig,
    ) -> Self {
        Self {
            fff,
            store,
            vectors,
            graph,
            embedder,
            config,
        }
    }

    /// Stage 1+2+3 fused search.
    pub fn hybrid_search(&self, query: &str, limit: usize) -> HybridSearchResult {
        let mut candidates: HashMap<String, HybridHit> = HashMap::new();

        // Stage 1: FFF file prefilter
        if let Ok(fff_result) = self.fff.find_files(query, 0, limit * 3) {
            for (rank, path) in fff_result.paths.iter().enumerate() {
                let fff_score = 1.0 - (rank as f32 / (fff_result.paths.len().max(1) as f32));
                for chunk in self.store.chunks_for_file(path) {
                    self.merge_hit(
                        &mut candidates,
                        &chunk,
                        fff_score * self.config.fff_weight,
                        0.0,
                        0.0,
                        "fff",
                    );
                }
            }
        }

        // Stage 1b: Text chunk search
        for chunk in self.store.search_chunks(query) {
            self.merge_hit(
                &mut candidates,
                &chunk,
                0.5 * self.config.fff_weight,
                0.0,
                0.0,
                "text",
            );
        }

        // Stage 2: Vector semantic search
        if let Ok(query_vec) = self.embedder.embed_one(query)
            && let Ok(vector_hits) = self.vectors.search(&query_vec, limit * 5)
        {
            for hit in vector_hits {
                if let Some(chunk) = self.find_chunk(&hit.chunk_id) {
                    self.merge_hit(
                        &mut candidates,
                        &chunk,
                        0.0,
                        hit.score * self.config.vector_weight,
                        0.0,
                        "vector",
                    );
                }
            }
        }

        // Stage 3: Graph boost for matching symbols
        let graph_symbols = self.graph.find_symbol_nodes(query);
        for sym in &graph_symbols {
            let callers = self.graph.callers(&sym.name, 1);
            let callees = self.graph.callees(&sym.name, 1);
            for related in callers.iter().chain(callees.iter()) {
                for chunk in self.store.find_symbol(&related.name) {
                    self.merge_hit(
                        &mut candidates,
                        &chunk,
                        0.0,
                        0.0,
                        0.8 * self.config.graph_weight,
                        "graph",
                    );
                }
            }
            for chunk in self.store.find_symbol(&sym.name) {
                self.merge_hit(
                    &mut candidates,
                    &chunk,
                    0.0,
                    0.0,
                    1.0 * self.config.graph_weight,
                    "graph",
                );
            }
        }

        let total = candidates.len();
        let mut hits: Vec<HybridHit> = candidates.into_values().collect();
        sort_by_score(&mut hits);
        hits.truncate(limit);

        HybridSearchResult {
            hits,
            total_candidates: total,
            query: query.to_string(),
        }
    }

    /// Semantic search (vector-only with chunk enrichment).
    pub fn semantic_search(&self, query: &str, limit: usize) -> HybridSearchResult {
        let mut hits = Vec::new();

        if let Ok(query_vec) = self.embedder.embed_one(query)
            && let Ok(vector_hits) = self.vectors.search(&query_vec, limit)
        {
            for vh in vector_hits {
                if let Some(chunk) = self.find_chunk(&vh.chunk_id) {
                    let mut hit = HybridHit::from_chunk(&chunk);
                    hit.score = vh.score;
                    hit.vector_score = vh.score;
                    hit.sources = vec!["vector".into()];
                    hits.push(hit);
                }
            }
        }

        HybridSearchResult {
            total_candidates: hits.len(),
            hits,
            query: query.to_string(),
        }
    }

    fn find_chunk(&self, chunk_id: &str) -> Option<CodeChunk> {
        self.store
            .all_chunks()
            .into_iter()
            .find(|c| c.id == chunk_id)
    }

    fn merge_hit(
        &self,
        candidates: &mut HashMap<String, HybridHit>,
        chunk: &CodeChunk,
        fff_score: f32,
        vector_score: f32,
        graph_score: f32,
        source: &str,
    ) {
        let entry = candidates
            .entry(chunk.id.clone())
            .or_insert_with(|| HybridHit::from_chunk(chunk));

        entry.fff_score = entry.fff_score.max(fff_score);
        entry.vector_score = entry.vector_score.max(vector_score);
        entry.graph_score = entry.graph_score.max(graph_score);
        entry.score = entry.fff_score + entry.vector_score + entry.graph_score;
        if !entry.sources.contains(&source.to_string()) {
            entry.sources.push(source.to_string());
        }
    }
}
