//! Hybrid query engine: FFF + Vector + Graph + BM25 fusion.

use std::collections::HashMap;
use std::sync::Arc;

use super::bm25::Bm25Index;
use crate::config::QueryConfig;
use crate::embedding::Embedder;
use crate::fff::FffEngine;
use crate::graph::CallGraphStore;
use crate::indexer::chunker::CodeChunk;
use crate::indexer::store::ChunkStore;
use crate::vector::{VectorHit, LanceDbVectorStore};

/// A fused search result with multi-signal scoring.
#[derive(Debug, Clone)]
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
    pub bm25_score: f32,
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
            bm25_score: 0.0,
            sources: Vec::new(),
        }
    }

    /// Build from VectorHit metadata when the full CodeChunk is not available
    /// (e.g. before AST indexing completes — vector store is persistent, ChunkStore is not).
    pub fn from_vector_hit(vh: &VectorHit) -> Self {
        Self {
            chunk_id: vh.chunk_id.clone(),
            relative_path: vh.relative_path.clone(),
            symbol_name: vh.symbol_name.clone(),
            symbol_kind: vh.symbol_kind.clone(),
            language: vh.language.clone(),
            start_line: vh.start_line,
            end_line: vh.end_line,
            content: vh.content_preview.clone(),
            score: vh.score,
            fff_score: 0.0,
            vector_score: vh.score,
            graph_score: 0.0,
            bm25_score: 0.0,
            sources: vec!["vector".into()],
        }
    }
}

/// Per-signal weighted scores merged into a [`HybridHit`].
#[derive(Debug, Clone, Copy, Default)]
struct SignalScores {
    fff: f32,
    vector: f32,
    graph: f32,
    bm25: f32,
}

/// Hybrid search response.
#[derive(Debug, Clone)]
pub struct HybridSearchResult {
    pub hits: Vec<HybridHit>,
    pub total_candidates: usize,
    pub query: String,
}

pub struct HybridQueryEngine {
    fff: FffEngine,
    store: Arc<ChunkStore>,
    vectors: Arc<LanceDbVectorStore>,
    graph: Arc<CallGraphStore>,
    bm25: Arc<Bm25Index>,
    embedder: Arc<dyn Embedder>,
    config: QueryConfig,
}

impl HybridQueryEngine {
    pub fn new(
        fff: FffEngine,
        store: Arc<ChunkStore>,
        vectors: Arc<LanceDbVectorStore>,
        graph: Arc<CallGraphStore>,
        bm25: Arc<Bm25Index>,
        embedder: Arc<dyn Embedder>,
        config: QueryConfig,
    ) -> Self {
        Self {
            fff,
            store,
            vectors,
            graph,
            bm25,
            embedder,
            config,
        }
    }

    /// Stage 1+2+3 fused search.
    pub async fn hybrid_search(&self, query: &str, limit: usize) -> HybridSearchResult {
        let mut candidates: HashMap<String, HybridHit> = HashMap::new();

        // Stage 1: FFF file prefilter
        if let Ok(fff_result) = self.fff.find_files(query, 0, limit * 3) {
            for (rank, path) in fff_result.paths.iter().enumerate() {
                let fff_score = 1.0 - (rank as f32 / (fff_result.paths.len().max(1) as f32));
                for chunk in self.store.chunks_for_file(path) {
                    self.merge_hit(
                        &mut candidates,
                        &chunk,
                        SignalScores {
                            fff: fff_score * self.config.fff_weight,
                            ..Default::default()
                        },
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
                SignalScores {
                    fff: 0.5 * self.config.fff_weight,
                    ..Default::default()
                },
                "text",
            );
        }

        // Stage 1c: BM25 lexical search
        let bm25_hits = self.bm25.search(query, limit * 5);
        let bm25_max = bm25_hits
            .iter()
            .map(|(_, s)| *s)
            .fold(0.0f32, f32::max)
            .max(f32::EPSILON);
        for (chunk_id, raw) in bm25_hits {
            if let Some(chunk) = self.store.chunk_by_id(&chunk_id) {
                self.merge_hit(
                    &mut candidates,
                    &chunk,
                    SignalScores {
                        bm25: raw / bm25_max * self.config.bm25_weight,
                        ..Default::default()
                    },
                    "bm25",
                );
            }
        }
        // Stage 2: Vector semantic search
        if let Ok(query_vec) = self.embedder.embed_one(query)
            && let Ok(vector_hits) = self.vectors.search(&query_vec, limit * 5).await
        {
            for hit in vector_hits {
                match self.store.chunk_by_id(&hit.chunk_id) {
                    Some(chunk) => {
                        self.merge_hit(
                            &mut candidates,
                            &chunk,
                            SignalScores {
                                vector: hit.score * self.config.vector_weight,
                                ..Default::default()
                            },
                            "vector",
                        );
                    }
                    None => {
                        // ChunkStore not yet populated (e.g. background index still running).
                        // Fall back to VectorHit metadata — it has everything we need.
                        let hybrid = HybridHit::from_vector_hit(&hit);
                        candidates.entry(hybrid.chunk_id.clone()).or_insert(hybrid);
                    }
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
                        SignalScores {
                            graph: 0.8 * self.config.graph_weight,
                            ..Default::default()
                        },
                        "graph",
                    );
                }
            }
            for chunk in self.store.find_symbol(&sym.name) {
                self.merge_hit(
                    &mut candidates,
                    &chunk,
                    SignalScores {
                        graph: 1.0 * self.config.graph_weight,
                        ..Default::default()
                    },
                    "graph",
                );
            }
        }

        let total = candidates.len();
        let mut hits: Vec<HybridHit> = candidates.into_values().collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);

        HybridSearchResult {
            hits,
            total_candidates: total,
            query: query.to_string(),
        }
    }

    /// Semantic search (vector-only with chunk enrichment).
    pub async fn semantic_search(&self, query: &str, limit: usize) -> HybridSearchResult {
        let mut hits = Vec::new();

        if let Ok(query_vec) = self.embedder.embed_one(query)
            && let Ok(vector_hits) = self.vectors.search(&query_vec, limit).await
        {
            for vh in vector_hits {
                let hit = match self.store.chunk_by_id(&vh.chunk_id) {
                    Some(chunk) => {
                        let mut h = HybridHit::from_chunk(&chunk);
                        h.score = vh.score;
                        h.vector_score = vh.score;
                        h.sources = vec!["vector".into()];
                        h
                    }
                    None => HybridHit::from_vector_hit(&vh),
                };
                hits.push(hit);
            }
        }

        HybridSearchResult {
            total_candidates: hits.len(),
            hits,
            query: query.to_string(),
        }
    }

    fn merge_hit(
        &self,
        candidates: &mut HashMap<String, HybridHit>,
        chunk: &CodeChunk,
        scores: SignalScores,
        source: &str,
    ) {
        let entry = candidates
            .entry(chunk.id.clone())
            .or_insert_with(|| HybridHit::from_chunk(chunk));

        entry.fff_score = entry.fff_score.max(scores.fff);
        entry.vector_score = entry.vector_score.max(scores.vector);
        entry.graph_score = entry.graph_score.max(scores.graph);
        entry.bm25_score = entry.bm25_score.max(scores.bm25);
        entry.score = entry.fff_score + entry.vector_score + entry.graph_score + entry.bm25_score;
        if !entry.sources.contains(&source.to_string()) {
            entry.sources.push(source.to_string());
        }
    }
}
