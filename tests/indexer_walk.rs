use std::path::PathBuf;
use std::sync::Arc;

use fva::config::{Config, IndexerConfig};
use fva::embedding::build_embedder;
use fva::graph::CallGraphStore;
use fva::indexer::Indexer;
use fva::indexer::parser::is_indexable;
use fva::query::Bm25Index;
use fva::vector::LanceDbVectorStore;
use ignore::WalkBuilder;

async fn test_indexer() -> Indexer {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let config = Config::default();
    let embedder = build_embedder(&config.embedding).unwrap();
    let data_dir = root.join(".fva-test");
    let vectors = LanceDbVectorStore::open(data_dir.join("vectors"), embedder.dimensions())
        .await
        .unwrap();
    let graph = Arc::new(CallGraphStore::open(&data_dir).unwrap());
    let bm25 = Arc::new(Bm25Index::new().unwrap());

    Indexer::new(
        root,
        IndexerConfig::default(),
        true,
        embedder,
        Arc::new(vectors),
        graph,
        bm25,
    )
}

#[test]
fn ignore_walk_finds_rust_files() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut builder = WalkBuilder::new(&root);
    builder.git_ignore(true);
    builder.git_global(true);
    builder.hidden(false);

    let mut rs_files = Vec::new();
    for entry in builder.build().flatten() {
        let path = entry.path();
        if path.is_file() && is_indexable(path) {
            rs_files.push(path.to_path_buf());
        }
    }

    assert!(!rs_files.is_empty(), "ignore walk should find .rs files");
}

#[tokio::test(flavor = "multi_thread")]
async fn indexer_finds_rust_sources() {
    let indexer = test_indexer().await;
    let count = indexer.index_all().await.expect("index_all should succeed");
    let stats = indexer.stats();

    assert!(
        stats.indexed_files > 0,
        "expected indexed files, stats={stats:?}"
    );
    assert!(stats.total_chunks > 0, "expected chunks, count={count}");
}
