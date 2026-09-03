//! FVA engine — orchestrates all subsystems.

use std::path::PathBuf;
use std::sync::Arc;

use crate::config::Config;
use crate::embedding::{Embedder, build_embedder};
use crate::error::Result;
use crate::fff::FffEngine;
use crate::graph::CallGraphStore;
use crate::indexer::Indexer;
use crate::query::{Bm25Index, ContextBuilder, HybridQueryEngine};
use crate::vector::LanceDbVectorStore;
use crate::wiki::WikiStore;

/// Central FVA engine holding all subsystems.
pub struct FvaEngine {
    pub root: PathBuf,
    pub config: Config,
    pub fff: FffEngine,
    pub indexer: Arc<Indexer>,
    pub embedder: Arc<dyn Embedder>,
    pub vectors: Arc<LanceDbVectorStore>,
    pub graph: Arc<CallGraphStore>,
    pub query: HybridQueryEngine,
    pub context: ContextBuilder,
    pub wiki: Arc<WikiStore>,
}

impl FvaEngine {
    pub async fn new(config: Config, root: PathBuf) -> Result<Self> {
        let data_dir = config.resolve_data_dir(&root);

        let fff = FffEngine::new(&root, &config.fff)?;
        let embedder = build_embedder(&config.embedding)?;
        let vectors = Arc::new(LanceDbVectorStore::open(
            if std::path::Path::new(&config.vector.db_path).is_absolute() {
                std::path::PathBuf::from(&config.vector.db_path)
            } else {
                data_dir.join(&config.vector.db_path)
            },
            embedder.dimensions(),
        )
        .await?);
        let graph = Arc::new(CallGraphStore::open(&data_dir)?);
        let bm25 = Arc::new(Bm25Index::new()?);

        let indexer = Arc::new(Indexer::new(
            root.clone(),
            config.indexer.clone(),
            config.sandbox_indexing,
            embedder.clone(),
            vectors.clone(),
            graph.clone(),
            bm25.clone(),
        ));

        let store = indexer.store();
        let query = HybridQueryEngine::new(
            fff.clone(),
            store.clone(),
            vectors.clone(),
            graph.clone(),
            bm25,
            embedder.clone(),
            config.query.clone(),
        );
        let context = ContextBuilder::new(store, graph.clone(), config.query.max_context_tokens);

        let wiki_dir = data_dir.join("wiki");
        let wiki = Arc::new(WikiStore::open(wiki_dir, embedder.clone())?);

        Ok(Self {
            root,
            config,
            fff,
            indexer,
            embedder,
            vectors,
            graph,
            query,
            context,
            wiki,
        })
    }

    pub async fn shutdown(&self) {
        let _ = self.vectors.persist();
        let _ = self.graph.persist();
        let _ = self.wiki.persist();
        self.fff.shutdown();
    }
}
