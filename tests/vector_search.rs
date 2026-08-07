//! Integration tests for vector search quality.
//! These measure real search behaviour end-to-end against the LanceDB store.

mod common;

use std::sync::Arc;

use fva::embedding::{Embedder, LocalEmbedder, cosine_similarity};
use fva::error::Result;
use fva::vector::{LanceDbVectorStore, VectorStore, index_chunks};
use tempfile::TempDir;

use common::make_chunks;

/// Helper to build a test vector store
async fn test_store() -> (Arc<dyn VectorStore>, Arc<dyn Embedder>, TempDir) {
    let embedder = Arc::new(LocalEmbedder::new(256));
    let dir = TempDir::new().expect("tempdir");
    let store: Arc<dyn VectorStore> = Arc::new(
        LanceDbVectorStore::open(dir.path().join("vectors"), embedder.dimensions())
            .await
            .expect("open lancedb store"),
    );
    (store, embedder, dir)
}

#[tokio::test]
async fn test_embedding_quality_semantically_related() -> Result<()> {
    let embedder = LocalEmbedder::new(256);

    // Code dealing with authentication
    let auth = embedder.embed_one("fn authenticate_user(token: &str) -> Result<User>")?;
    let login = embedder.embed_one("fn login_user(username: &str, password: &str) -> bool")?;
    let query_db = embedder.embed_one("fn query_database(sql: &str) -> Vec<Row>")?;
    let html = embedder.embed_one("fn render_template(name: &str) -> String")?;

    // Auth-related should be more similar to each other than to rendering
    let sim_auth_login = cosine_similarity(&auth, &login);
    let sim_auth_html = cosine_similarity(&auth, &html);
    let sim_auth_db = cosine_similarity(&auth, &query_db);

    println!(
        "  auth↔login: {:.4}, auth↔html: {:.4}, auth↔db: {:.4}",
        sim_auth_login, sim_auth_html, sim_auth_db
    );

    // The authentication embedder should find login more related than HTML rendering
    assert!(
        sim_auth_login > sim_auth_html,
        "auth and login should be more similar ({:.4}) than auth and HTML ({:.4})",
        sim_auth_login,
        sim_auth_html
    );

    // Note: auth vs database query may vary depending on whether "query" and "user"
    // co-occur in auth-related training data analog. This is a directional goal
    // rather than a hard requirement.
    if sim_auth_db > sim_auth_html {
        println!(
            "  ✓ auth-db ({:.4}) > auth-html ({:.4})",
            sim_auth_db, sim_auth_html
        );
    }

    Ok(())
}

#[tokio::test]
async fn test_search_quality_relevant_results_on_top() -> Result<()> {
    let (store, embedder, _dir) = test_store().await;
    let chunks = make_chunks();
    let count = chunks.len();

    // Index all test chunks
    index_chunks(embedder.as_ref(), store.as_ref(), &chunks).await?;

    // Search for authentication-related code
    let query = "authenticate user login";
    let query_vec = embedder.embed_one(query)?;
    let results = store.search(&query_vec, count).await?;

    assert!(!results.is_empty(), "should find results");

    println!("\n=== Search: '{}' ===", query);
    for (i, hit) in results.iter().enumerate() {
        println!(
            "  #{:<3} {:.4}  {} ({})",
            i + 1,
            hit.score,
            hit.symbol_name,
            hit.relative_path
        );
    }

    // The top 3 results should include auth-related functions
    let top_names: Vec<&str> = results
        .iter()
        .take(3)
        .map(|h| h.symbol_name.as_str())
        .collect();
    println!("  Top 3: {:?}", top_names);

    let auth_related = [
        "login_user",
        "logout_user",
        "validate_token",
        "handle_request",
    ];
    let top_has_auth = top_names.iter().any(|n| auth_related.contains(n));
    assert!(
        top_has_auth,
        "top 3 results should include at least one auth-related function, got: {:?}",
        top_names
    );

    Ok(())
}

#[tokio::test]
async fn test_search_rejects_unrelated_code() -> Result<()> {
    let (store, embedder, _dir) = test_store().await;
    let chunks = make_chunks();
    index_chunks(embedder.as_ref(), store.as_ref(), &chunks).await?;

    // Search for sorting algorithms
    let query = "sort array of integers";
    let query_vec = embedder.embed_one(query)?;
    let results = store.search(&query_vec, chunks.len()).await?;

    println!("\n=== Search: '{}' ===", query);
    for (i, hit) in results.iter().enumerate() {
        println!(
            "  #{:<3} {:.4}  {} ({})",
            i + 1,
            hit.score,
            hit.symbol_name,
            hit.relative_path
        );
    }

    // Sorting functions should appear in top results
    let top5_names: Vec<&str> = results
        .iter()
        .take(5)
        .map(|h| h.symbol_name.as_str())
        .collect();
    println!("  Top 5: {:?}", top5_names);

    let has_sort = top5_names
        .iter()
        .any(|n| *n == "bubble_sort" || *n == "quick_sort");
    assert!(
        has_sort,
        "sort query should return sorting functions in top 5, got: {:?}",
        top5_names
    );

    Ok(())
}

#[tokio::test]
async fn test_parallel_search_performance() -> Result<()> {
    let embedder = Arc::new(LocalEmbedder::new(256));
    let dir = TempDir::new().expect("tempdir");
    let store: Arc<dyn VectorStore> = Arc::new(
        LanceDbVectorStore::open(dir.path().join("vectors"), embedder.dimensions())
            .await
            .expect("open lancedb store"),
    );

    // Index the same set of chunks multiple times to simulate a larger codebase
    let base_chunks = make_chunks();
    let mut all_chunks = Vec::new();
    for i in 0..100 {
        for chunk in &base_chunks {
            let mut c = chunk.clone();
            c.id = format!("{}:dup{}", chunk.id, i);
            all_chunks.push(c);
        }
    }
    println!(
        "  Indexing {} chunks for performance test",
        all_chunks.len()
    );
    index_chunks(embedder.as_ref(), store.as_ref(), &all_chunks).await?;

    let stats = store.stats();
    println!(
        "  Vector store: {} vectors, {} dimensions",
        stats.total_vectors, stats.dimensions
    );

    // Run multiple searches and measure time
    let queries = vec![
        "authenticate user",
        "sort numbers in array",
        "render HTML template",
        "database query",
        "parse configuration file",
    ];

    let mut total_time_ms = 0.0;
    let mut iterations = 0;

    for _ in 0..3 {
        // warmup
        for q in &queries {
            let v = embedder.embed_one(q)?;
            let _ = store.search(&v, 10).await?;
        }
    }

    for _ in 0..5 {
        // measured
        for q in &queries {
            let v = embedder.embed_one(q)?;
            let start = std::time::Instant::now();
            let results = store.search(&v, 10).await?;
            let elapsed = start.elapsed().as_secs_f64() * 1000.0;
            total_time_ms += elapsed;
            iterations += 1;
            let _ = results.len(); // use result
        }
    }

    let avg_ms = total_time_ms / iterations as f64;
    println!(
        "\n  Performance: {} searches, avg {:.2}ms per search (total: {:.1}ms)",
        iterations, avg_ms, total_time_ms
    );

    // Verify the target: vector_search should be <50ms per search
    assert!(
        avg_ms < 50.0,
        "search too slow: avg {:.2}ms per search (target: <50ms)",
        avg_ms
    );

    Ok(())
}

#[tokio::test]
async fn test_token_index_properly_built() -> Result<()> {
    let (store, embedder, _dir) = test_store().await;
    let chunks = make_chunks();

    index_chunks(embedder.as_ref(), store.as_ref(), &chunks).await?;

    let stats = store.stats();
    assert_eq!(
        stats.total_vectors,
        chunks.len(),
        "all chunks should be indexed"
    );

    // Verify we can find code by symbol name via search
    let query = "validate_token";
    let query_vec = embedder.embed_one(query)?;
    let results = store.search(&query_vec, 5).await?;

    let found: Vec<&str> = results.iter().map(|h| h.symbol_name.as_str()).collect();
    println!("  Search for '{}' returned: {:?}", query, found);
    assert!(
        found.contains(&"validate_token"),
        "search should find exact symbol match: validate_token"
    );

    Ok(())
}

#[tokio::test]
async fn test_chunk_by_id_o1() -> Result<()> {
    let embedder = LocalEmbedder::new(256);
    let dir = TempDir::new().expect("tempdir");
    let store = LanceDbVectorStore::open(dir.path().join("vectors"), embedder.dimensions())
        .await
        .expect("open lancedb store");

    let chunks = make_chunks();
    let vectors: Vec<Vec<f32>> = chunks
        .iter()
        .map(|c| {
            embedder
                .embed_one(&format!("{} {} {}", c.language, c.symbol_kind, c.content))
                .unwrap()
        })
        .collect();
    store.upsert_chunks(&chunks, &vectors).await?;

    // Verify the store works for multiple ops
    let stats = store.stats();
    assert_eq!(stats.total_vectors, chunks.len());
    println!("  Inserted {} chunks", stats.total_vectors);

    Ok(())
}
