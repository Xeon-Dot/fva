//! LanceDB vector store round-trip tests (feature-gated; run with --features lancedb).

mod common;

use std::sync::Arc;
use tempfile::TempDir;

use fva::embedding::{Embedder, LocalEmbedder};
use fva::error::Result;
use fva::vector::{LanceDbVectorStore, VectorStore, index_chunks};
use common::make_chunks;

async fn test_store() -> (Arc<LanceDbVectorStore>, Arc<LocalEmbedder>, TempDir) {
    let embedder = Arc::new(LocalEmbedder::new(256));
    let dir = TempDir::new().expect("tempdir");
    let store = Arc::new(
        LanceDbVectorStore::open(dir.path().join("vectors"), embedder.dimensions())
            .await
            .expect("open lancedb store"),
    );
    (store, embedder, dir)
}

#[tokio::test]
async fn round_trip_upsert_search_remove() -> Result<()> {
    let (store, embedder, _dir) = test_store().await;
    let chunks = make_chunks();
    index_chunks(embedder.as_ref(), store.as_ref(), &chunks).await?;

    let stats = store.stats();
    assert_eq!(stats.total_vectors, chunks.len());

    let query_vec = embedder.embed_one("authenticate user login")?;
    let results = store.search(&query_vec, chunks.len()).await?;
    assert!(!results.is_empty());
    let top: Vec<&str> = results
        .iter()
        .take(3)
        .map(|h| h.symbol_name.as_str())
        .collect();
    let auth_related = ["login_user", "logout_user", "validate_token", "handle_request"];
    assert!(top.iter().any(|n| auth_related.contains(n)), "top3: {top:?}");

    store.remove_file("src/auth.rs").await?;
    assert_eq!(store.stats().total_vectors, chunks.len() - 3);
    let after = store.search(&query_vec, chunks.len()).await?;
    assert!(after.iter().all(|h| !h.relative_path.contains("auth.rs")));

    Ok(())
}

#[tokio::test]
async fn persists_across_reopen() -> Result<()> {
    let embedder = Arc::new(LocalEmbedder::new(256));
    let dir = TempDir::new().expect("tempdir");
    {
        let store = LanceDbVectorStore::open(dir.path().join("vectors"), 256).await?;
        let chunks = make_chunks();
        index_chunks(embedder.as_ref(), &store, &chunks).await?;
    }
    let store = LanceDbVectorStore::open(dir.path().join("vectors"), 256).await?;
    assert_eq!(store.stats().total_vectors, make_chunks().len());
    Ok(())
}

#[tokio::test]
async fn dimension_change_drops_table() -> Result<()> {
    let dir = TempDir::new().expect("tempdir");
    {
        let store = LanceDbVectorStore::open(dir.path().join("vectors"), 256).await?;
        let chunks = make_chunks();
        let embedder = LocalEmbedder::new(256);
        index_chunks(&embedder, &store, &chunks).await?;
    }
    let store = LanceDbVectorStore::open(dir.path().join("vectors"), 512).await?;
    assert_eq!(store.stats().total_vectors, 0);
    Ok(())
}
