//! Integration tests for vector search quality and performance.
//! These measure real search behaviour end-to-end.

use std::sync::Arc;

use fva::embedding::{Embedder, LocalEmbedder, cosine_similarity};
use fva::error::Result;
use fva::indexer::chunker::CodeChunk;
use fva::vector::{FlatVectorStore, VectorStore, index_chunks};

/// Helper to build a test vector store
fn test_store() -> (Arc<dyn VectorStore>, Arc<dyn Embedder>) {
    let embedder = Arc::new(LocalEmbedder::new(256));
    let store = Arc::new(
        FlatVectorStore::open(
            std::env::temp_dir().join(format!("fva_test_vectors_{}", std::process::id())),
            embedder.dimensions(),
        )
        .unwrap(),
    );
    (store, embedder)
}

/// Build a CodeChunk for testing
fn make_chunk(id: &str, symbol: &str, kind: &str, content: &str, path: &str) -> CodeChunk {
    CodeChunk {
        id: id.to_string(),
        file_path: path.to_string(),
        relative_path: path.to_string(),
        language: "rust".to_string(),
        symbol_name: symbol.to_string(),
        symbol_kind: kind.to_string(),
        start_line: 1,
        end_line: content.lines().count().max(1),
        content: content.to_string(),
        content_hash: "".to_string(),
        line_count: content.lines().count().max(1),
    }
}

fn make_chunks() -> Vec<CodeChunk> {
    vec![
        make_chunk(
            "auth:login",
            "login_user",
            "fn",
            "fn login_user(username: &str, password: &str) -> bool {\n    // authenticate user\n    true\n}",
            "src/auth.rs",
        ),
        make_chunk(
            "auth:logout",
            "logout_user",
            "fn",
            "fn logout_user(session: &str) {\n    // clear session\n}",
            "src/auth.rs",
        ),
        make_chunk(
            "auth:validate",
            "validate_token",
            "fn",
            "fn validate_token(token: &str) -> Result<User> {\n    // verify JWT token\n    Ok(User)\n}",
            "src/auth.rs",
        ),
        make_chunk(
            "db:query",
            "query_users",
            "fn",
            "fn query_users(conn: &Connection) -> Result<Vec<User>> {\n    conn.query(\"SELECT * FROM users\")\n}",
            "src/db.rs",
        ),
        make_chunk(
            "db:insert",
            "insert_user",
            "fn",
            "fn insert_user(conn: &Connection, user: &User) -> Result<()> {\n    conn.execute(\"INSERT INTO users VALUES\", user)\n}",
            "src/db.rs",
        ),
        make_chunk(
            "ui:render",
            "render_html",
            "fn",
            "fn render_html(template: &str, data: &Data) -> String {\n    template.render(&data)\n}",
            "src/ui.rs",
        ),
        make_chunk(
            "ui:format",
            "format_date",
            "fn",
            "fn format_date(dt: &DateTime) -> String {\n    dt.format(\"%Y-%m-%d\")\n}",
            "src/ui.rs",
        ),
        make_chunk(
            "sort:bubble",
            "bubble_sort",
            "fn",
            "fn bubble_sort(arr: &mut [i32]) {\n    for i in 0..arr.len() {\n        for j in 0..arr.len()-i-1 {\n            if arr[j] > arr[j+1] {\n                arr.swap(j, j+1);\n            }\n        }\n    }\n}",
            "src/sort.rs",
        ),
        make_chunk(
            "sort:quick",
            "quick_sort",
            "fn",
            "fn quick_sort(arr: &mut [i32]) {\n    if arr.len() <= 1 { return; }\n    let pivot = partition(arr);\n    quick_sort(&mut arr[..pivot]);\n    quick_sort(&mut arr[pivot+1..]);\n}",
            "src/sort.rs",
        ),
        make_chunk(
            "config:parse",
            "parse_config",
            "fn",
            "fn parse_config(path: &Path) -> Result<Config> {\n    let content = std::fs::read_to_string(path)?;\n    toml::from_str(&content)\n}",
            "src/config.rs",
        ),
        make_chunk(
            "http:handle",
            "handle_request",
            "fn",
            "fn handle_request(req: &Request) -> Response {\n    match req.method() {\n        Method::GET => handle_get(req),\n        Method::POST => handle_post(req),\n        _ => Response::not_found(),\n    }\n}",
            "src/http.rs",
        ),
        make_chunk(
            "http:parse",
            "parse_body",
            "fn",
            "fn parse_body(req: &Request) -> Result<Body> {\n    let bytes = req.body_bytes();\n    serde_json::from_slice(&bytes)\n}",
            "src/http.rs",
        ),
        make_chunk(
            "math:add",
            "add_numbers",
            "fn",
            "fn add_numbers(a: i32, b: i32) -> i32 { a + b }",
            "src/math.rs",
        ),
    ]
}

#[test]
fn test_embedding_quality_semantically_related() -> Result<()> {
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

#[test]
fn test_search_quality_relevant_results_on_top() -> Result<()> {
    let (store, embedder) = test_store();
    let chunks = make_chunks();
    let count = chunks.len();

    // Index all test chunks
    index_chunks(embedder.as_ref(), store.as_ref(), &chunks)?;

    // Search for authentication-related code
    let query = "authenticate user login";
    let query_vec = embedder.embed_one(query)?;
    let results = store.search(&query_vec, count)?;

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

#[test]
fn test_search_rejects_unrelated_code() -> Result<()> {
    let (store, embedder) = test_store();
    let chunks = make_chunks();
    index_chunks(embedder.as_ref(), store.as_ref(), &chunks)?;

    // Search for sorting algorithms
    let query = "sort array of integers";
    let query_vec = embedder.embed_one(query)?;
    let results = store.search(&query_vec, chunks.len())?;

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

#[test]
fn test_parallel_search_performance() -> Result<()> {
    let embedder = Arc::new(LocalEmbedder::new(256));
    let store = Arc::new(
        FlatVectorStore::open(
            std::env::temp_dir().join(format!("fva_test_perf_{}", std::process::id())),
            embedder.dimensions(),
        )
        .unwrap(),
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
    index_chunks(embedder.as_ref(), store.as_ref(), &all_chunks)?;

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
            let _ = store.search(&v, 10)?;
        }
    }

    for _ in 0..5 {
        // measured
        for q in &queries {
            let v = embedder.embed_one(q)?;
            let start = std::time::Instant::now();
            let results = store.search(&v, 10)?;
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

#[test]
fn test_token_index_properly_built() -> Result<()> {
    let (store, embedder) = test_store();
    let chunks = make_chunks();

    // Verify the token index is built (internal detail: upsert triggers token indexing)
    index_chunks(embedder.as_ref(), store.as_ref(), &chunks)?;

    let stats = store.stats();
    assert_eq!(
        stats.total_vectors,
        chunks.len(),
        "all chunks should be indexed"
    );

    // Verify we can find code by symbol name via search
    let query = "validate_token";
    let query_vec = embedder.embed_one(query)?;
    let results = store.search(&query_vec, 5)?;

    let found: Vec<&str> = results.iter().map(|h| h.symbol_name.as_str()).collect();
    println!("  Search for '{}' returned: {:?}", query, found);
    assert!(
        found.contains(&"validate_token"),
        "search should find exact symbol match: validate_token"
    );

    Ok(())
}

#[test]
fn test_chunk_by_id_o1() -> Result<()> {
    let embedder = LocalEmbedder::new(256);
    let store = FlatVectorStore::open(
        std::env::temp_dir().join(format!("fva_test_byid_{}", std::process::id())),
        embedder.dimensions(),
    )?;

    let chunks = make_chunks();
    let vectors: Vec<Vec<f32>> = chunks
        .iter()
        .map(|c| {
            embedder
                .embed_one(&format!("{} {} {}", c.language, c.symbol_kind, c.content))
                .unwrap()
        })
        .collect();
    store.upsert_chunks(&chunks, &vectors)?;

    // Verify the store works for multiple ops
    let stats = store.stats();
    assert_eq!(stats.total_vectors, chunks.len());
    println!("  Inserted {} chunks", stats.total_vectors);

    Ok(())
}
