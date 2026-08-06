# Native LanceDB Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire up the real LanceDB vector store so `backend = "lancedb"` (the config default, landed in Task 3) runs native LanceDB instead of aliasing the flat store.

**Architecture:** Convert the `VectorStore` trait to async (LanceDB's API is tokio-based), implement `LanceDbVectorStore` over `lancedb::Connection`/`Table` with a FixedSizeList f32 vector column, restructure the indexer into a sync rayon parse/embed phase + async upsert phase, and flip `lancedb` into default features.

**Tech Stack:** Rust edition 2024, tokio (already full-featured), lancedb 0.30 + arrow 55 (already optional deps), `futures 0.3` (new, for stream collection), bincode (flat store, unchanged).

## Global Constraints

- Working tree: **only `Cargo.lock` is dirty** (fva version 1.0.1→1.0.2, matches Cargo.toml). Keep it; it stays dirty until Task 3 commits. Do NOT commit Cargo.lock in Task 1 or 2 unless `cargo` changes it.
- `protoc` (protobuf-compiler) is REQUIRED to compile with the `lancedb` feature (lance's prost-build). Not installed on the dev machine; user installs before Task 2 (`sudo apt install protobuf-compiler`).
- `sccache` is the rustc-wrapper; if builds fail with "sccache not found" use `RUSTC_WRAPPER= cargo build`.
- No new dependencies except `futures = "0.3"` (Task 2).
- lancedb 0.30 API is async and returns builders: `.execute().await`. Errors map to `FvaError::Other` / `FvaError::Config` (see `src/error.rs`).
- Spec: `docs/superpowers/specs/2026-08-06-native-lancedb-design.md` — this plan implements it exactly.

---

### Task 1: Async `VectorStore` trait conversion

**Files:**
- Modify: `src/vector/mod.rs` (trait → async, `index_chunks` → async, `build_vector_store` → async)
- Modify: `src/vector/flat.rs` (async signatures; bodies unchanged; `preview` becomes `pub(crate)`)
- Modify: `src/indexer/mod.rs` (two-phase `index_all`, async `index_file`, `tokio::spawn` background index)
- Modify: `src/query/hybrid.rs` (`hybrid_search`/`semantic_search` → async)
- Modify: `src/engine.rs` (`FvaEngine::new` → async, `shutdown` → async)
- Modify: `src/main.rs` (`.await` all new async calls)
- Modify: `src/bench/mod.rs` (`run` → async, `bench_op` async closures)
- Modify: `src/mcp/server.rs` (3 tool handlers → `async fn`)
- Modify: `tests/vector_search.rs` (→ `#[tokio::test]`, `.await`)
- Modify: `tests/indexer_walk.rs` (→ `#[tokio::test]`, `.await`, explicit `backend = "flat"`)
- Test: `cargo test`, `cargo clippy --all-targets`

**Interfaces:**
- Consumes: existing `VectorStore` trait, `Indexer`, `HybridQueryEngine`, `FvaEngine`, `bench::run`
- Produces:
  - `pub trait VectorStore: Send + Sync` with `async fn upsert_chunks(&self, chunks: &[CodeChunk], vectors: &[Vec<f32>]) -> Result<()>`, `async fn remove_file(&self, relative_path: &str) -> Result<()>`, `async fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>>`, `async fn stats(&self) -> VectorStats`, `async fn persist(&self) -> Result<()>` — dyn-compatible (Rust 1.97)
  - `pub async fn index_chunks(embedder: &dyn Embedder, store: &dyn VectorStore, chunks: &[CodeChunk]) -> Result<usize>`
  - `pub async fn build_vector_store(config: &VectorConfig, data_dir: &Path, dimensions: usize) -> Result<Arc<dyn VectorStore>>`
  - `Indexer::index_all(&self) -> Result<usize>` (async), `Indexer::index_file(&self, file_path: &Path) -> Result<usize>` (async)
  - `FvaEngine::new(config: Config, root: PathBuf) -> Result<Self>` (async), `FvaEngine::shutdown(&self)` (async)
  - `pub async fn run(engine: &Arc<FvaEngine>, opts: &BenchOptions) -> BenchReport`
  - `pub(crate) fn preview(content: &str, max_len: usize) -> String` in `src/vector/mod.rs`
  - `pub(crate) fn chunk_texts(chunks: &[CodeChunk]) -> Vec<String>` in `src/vector/mod.rs` (extracted from `index_chunks`)

- [ ] **Step 1: Write the failing tests (convert existing test harness)**

`tests/vector_search.rs`: change every `#[test]` to `#[tokio::test]`, every `index_chunks(...)` to `index_chunks(...).await`, every `store.search(...)` to `store.search(...).await`, `store.upsert_chunks(...)` → `.await`, `store.persist()` → `.await` (line 400 is the last one: `store.upsert_chunks(&chunks, &vectors)?;` → `store.upsert_chunks(&chunks, &vectors).await?;`). `test_store()` helper unchanged except both callers gain `.await`.

`tests/indexer_walk.rs`: `#[test]` → `#[tokio::test]`; `indexer.index_all().expect(...)` → `indexer.index_all().await.expect(...)`. In `test_indexer()` (top of file), after building `config`, add `config.vector.backend = "flat".to_string();` BEFORE `build_vector_store(...)` so the test does not depend on the (later-renamed) default backend.

- [ ] **Step 2: Run tests to verify they fail to compile**

Run: `cargo test --test vector_search`
Expected: FAIL — compile errors: `async fn` not allowed in trait / `waiting on error` (trait methods not async).

- [ ] **Step 3: Convert the trait and flat store**

`src/vector/mod.rs`:
```rust
pub trait VectorStore: Send + Sync {
    async fn upsert_chunks(&self, chunks: &[CodeChunk], vectors: &[Vec<f32>]) -> Result<()>;
    async fn remove_file(&self, relative_path: &str) -> Result<()>;
    async fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>>;
    async fn stats(&self) -> VectorStats;
    async fn persist(&self) -> Result<()>;
}

pub async fn build_vector_store(
    config: &VectorConfig,
    data_dir: &Path,
    dimensions: usize,
) -> Result<Arc<dyn VectorStore>> {
    let path = if Path::new(&config.db_path).is_absolute() {
        Path::new(&config.db_path).to_path_buf()
    } else {
        data_dir.join(&config.db_path)
    };
    match config.backend.as_str() {
        "flat" => Ok(Arc::new(FlatVectorStore::open(path, dimensions)?)),
        #[cfg(feature = "lancedb")]
        "lancedb" | "lancedb-native" => Err(FvaError::Other(
            "native LanceDB backend not yet wired — use backend = \"flat\"".into(),
        )),
        #[cfg(not(feature = "lancedb"))]
        "lancedb" | "lancedb-native" => Err(FvaError::Config(
            "lancedb requires building with --features lancedb".into(),
        )),
        other => Err(FvaError::Config(format!("unknown vector backend: {other}"))),
    }
}

pub(crate) fn preview(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }
    let mut end = max_len.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &content[..end])
}

pub(crate) fn chunk_texts(chunks: &[CodeChunk]) -> Vec<String> {
    chunks
        .iter()
        .map(|c| format!("{} {} {}\n{}", c.language, c.symbol_kind, c.symbol_name, c.content))
        .collect()
}

pub async fn index_chunks(
    embedder: &dyn Embedder,
    store: &dyn VectorStore,
    chunks: &[CodeChunk],
) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }
    let texts = chunk_texts(chunks);
    let vectors = embedder.embed(&texts)?;
    store.upsert_chunks(chunks, &vectors).await?;
    Ok(chunks.len())
}
```
Keep the old `index_chunks` body's `chunks.is_empty()` guard. Delete the stale comment "flat store is the default production path".

`src/vector/flat.rs`: add `use super::{preview, VectorStore};` (drop the private `preview` method — `StoredVector::from_chunk` calls `super::preview(&chunk.content, 200)`; remove `fn preview` from the impl block). Change each `impl VectorStore for FlatVectorStore` method signature to `async fn` — **bodies unchanged**.

- [ ] **Step 4: Run tests — expect next wave of compile errors (callers)**

Run: `cargo test --test vector_search`
Expected: FAIL — callers of `search`/`upsert_chunks`/`index_chunks` are still sync (engine, indexer, hybrid, bench, main, mcp).

- [ ] **Step 5: Convert indexer (two-phase)**

`src/indexer/mod.rs`:
```rust
pub async fn index_all(&self) -> Result<usize> {
    *self.scanning.write() = true;
    let result = self.index_all_inner().await;
    *self.scanning.write() = false;
    if result.is_ok() {
        let _ = self.vectors.persist().await;
        let _ = self.graph.persist();
    }
    result
}

async fn index_all_inner(&self) -> Result<usize> {
    let files = self.collect_files()?;
    tracing::info!("indexing {} source files", files.len());

    // Phase 1 (rayon, sync, CPU-bound): read -> parse -> chunk -> embed
    let parsed: Vec<(String, blake3::Hash, Vec<CodeChunk>, Vec<Vec<f32>>)> = files
        .par_iter()
        .filter_map(|f| self.parse_and_embed(f).ok().flatten())
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
        self.vectors.stats().await.total_vectors,
        self.graph.stats().edges
    );

    Ok(parsed.iter().map(|(_, _, c, _)| c.len()).sum())
}

fn parse_and_embed(
    &self,
    file_path: &Path,
) -> Result<Option<(String, blake3::Hash, Vec<CodeChunk>, Vec<Vec<f32>>)>> {
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

pub async fn index_file(&self, file_path: &Path) -> Result<usize> {
    let Some((relative, hash, chunks, vectors)) = self.parse_and_embed(file_path)? else {
        return Ok(0);
    };
    self.commit_file(&relative, &hash, &chunks, &vectors).await;
    Ok(chunks.len())
}

pub fn spawn_background_index(self: &Arc<Self>) {
    let indexer = Arc::clone(self);
    tokio::spawn(async move {
        if let Err(e) = indexer.index_all().await {
            tracing::error!("background index failed: {e}");
        }
    });
}
```
Remove the old `index_all_inner`/`index_file` bodies. Keep `collect_files`, `wait_for_index`, `is_scanning`, `store`, `root`, `stats` unchanged. Add `use crate::vector::chunk_texts` — or call `crate::vector::chunk_texts` fully qualified as above (no import needed if fully qualified — use the fully-qualified call).

- [ ] **Step 6: Convert hybrid query, engine, main, mcp, bench**

`src/query/hybrid.rs`: `pub fn hybrid_search(&self, ...)` → `pub async fn hybrid_search(...)`; inside, `let Ok(vector_hits) = self.vectors.search(&query_vec, limit * 5)` → `let Ok(vector_hits) = self.vectors.search(&query_vec, limit * 5).await`. Same for `semantic_search` with `limit`. Bodies otherwise unchanged.

`src/engine.rs`: `pub fn new(...)` → `pub async fn new(...)`; `let vectors = build_vector_store(&config.vector, &data_dir, embedder.dimensions())?;` → `...await?;`. `pub fn shutdown(&self)` → `pub async fn shutdown(&self)`; `let _ = self.vectors.persist();` → `let _ = self.vectors.persist().await;` (graph/wiki persist stay sync).

`src/main.rs`:
- Line 193: `let engine = Arc::new(FvaEngine::new(config, root)?);` → `FvaEngine::new(config, root).await?`
- Line 205: `engine.indexer.index_all()?` → `engine.indexer.index_all().await?`
- Line 215: `let _ = engine.indexer.index_all();` → `.await`
- Line 232: same in Bench branch
- Line 260: same in Search branch
- Line 262: `let result = engine.query.hybrid_search(&query, limit);` → `...await;`
- Lines 210, 221, 255, 267, 333, 359: `engine.shutdown();` → `engine.shutdown().await;` (including the one in the ctrl_c `tokio::spawn` block at line 345)
- Line 253: `let report = fva::bench::run(&engine, &opts);` → `fva::bench::run(&engine, &opts).await;`

`src/mcp/server.rs`: the three tool handlers that call `engine.query.hybrid_search(...)` / `semantic_search(...)` — lines ~336, ~352, ~418. Change `fn semantic_search(...)` → `async fn semantic_search(...)` and add `.await` to the query calls (`self.engine.query.semantic_search(&params.query, limit).await`, `self.engine.query.hybrid_search(&params.query, limit).await`, and the one at line ~418). rmcp 1.7 `#[tool]` macros support async handlers; if the macro rejects `async fn`, check whether the handler already runs inside a spawned task and use `tokio::task::block_in_place(|| Handle::current().block_on(...))` instead — but try `async fn` first.

`src/bench/mod.rs`:
- `pub fn run(...)` → `pub async fn run(...)`; `let _ = engine.indexer.index_all();` (line 62) → `.await`; `let _ = engine.vectors.search(&vec, 20)` → `let _ = engine.vectors.search(&vec, 20).await`; `let _ = engine.query.semantic_search(&q, 10);` → `.await`; `let _ = engine.query.hybrid_search(&q, 10);` → `.await`; `engine.query.hybrid_search(&query, 5)` (line 170) → `.await`.
- `bench_op` becomes async and takes async closures:
```rust
async fn bench_op<F>(name: &str, opts: &BenchOptions, mut f: F, target_ms: Option<f64>) -> BenchResult
where
    F: FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + '_>>,
{
    for _ in 0..opts.warmup {
        f().await;
    }
    let mut samples = Vec::with_capacity(opts.iterations);
    for _ in 0..opts.iterations {
        let start = Instant::now();
        f().await;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    BenchResult::from_samples(name, &samples, target_ms)
}
```
- Every `suite.add(bench_op(name, opts, || {...}, target))` closure becomes `|| Box::pin(async move { ... })` — the FFF sync closures (`find_files`, `grep`) get `Box::pin(async move { ... })` wrappers too (bodies unchanged). Each closure that captures a loop variable (`q`) does `let q = q.clone();` as its first statement.
- `bench_full_index` → `async fn bench_full_index(engine: &Arc<FvaEngine>, _opts: &BenchOptions) -> BenchResult` with `let _ = engine.indexer.index_all().await;`. `bench_ast_chunk` stays sync.
- Add `use std::future::Future; use std::pin::Pin;` imports.
- `run`'s calls to `bench_op(...)`, `bench_ast_chunk`, `bench_full_index` → `bench_op(...).await`, `bench_full_index(...).await` (use `suite.add(...)` with the awaited result).

- [ ] **Step 7: Run all tests + lint**

Run: `cargo test`
Expected: PASS (all unit + integration tests, flat backend). If `dyn VectorStore` async dispatch fails to compile, fall back to explicit signatures in the trait:
```rust
fn search<'a>(&'a self, query_vector: &'a [f32], limit: usize) -> Pin<Box<dyn Future<Output = Result<Vec<VectorHit>>> + Send + 'a>>;
```
(matching bodies become `Box::pin(async move { ... })`).

Run: `cargo clippy --all-targets` — fix any warnings introduced (there will be some `async_fn_in_trait` / await-related lints; prefer minimal fixes).

- [ ] **Step 8: Commit**

```bash
git add src/vector/mod.rs src/vector/flat.rs src/indexer/mod.rs src/query/hybrid.rs src/engine.rs src/main.rs src/bench/mod.rs src/mcp/server.rs tests/vector_search.rs tests/indexer_walk.rs
git commit -m "refactor: make VectorStore trait async, two-phase async indexer"
```
(Do not stage `Cargo.lock`.)

---

### Task 2: `LanceDbVectorStore` implementation

**Prerequisite:** user has run `sudo apt install protobuf-compiler`.

**Files:**
- Create: `src/vector/lancedb.rs`
- Modify: `src/vector/mod.rs` (module decl, wire `build_vector_store` arms)
- Modify: `Cargo.toml` (add `futures = "0.3"`)
- Create: `tests/vector_search_lancedb.rs`
- Modify: `tests/common/mod.rs` (create — shared chunk helpers)
- Modify: `tests/vector_search.rs` (use `mod common;`)
- Test: `cargo test --features lancedb`

**Interfaces:**
- Consumes: async `VectorStore` trait (Task 1), `preview`/`chunk_texts` from `src/vector/mod.rs`, `tests/common` chunk helpers
- Produces: `pub struct LanceDbVectorStore` with `pub async fn open(path: PathBuf, dimensions: usize) -> Result<Self>` implementing `VectorStore`. Backend strings `"lancedb"` | `"lancedb-native"` resolve to it (feature-gated).

- [ ] **Step 1: Preflight protoc + feature build**

Run: `cargo build --features lancedb`
Expected: compiles (first build downloads/compiles lance+arrow, several minutes; sccache caches it). If protoc is missing, the build fails with "protoc not found" — user must install it before continuing.

- [ ] **Step 2: Add the failing test**

`tests/common/mod.rs` — move `make_chunk` and `make_chunks` verbatim from `tests/vector_search.rs` (they must stay in a `mod common;` file so both test binaries share them). In `tests/vector_search.rs`, replace the two local fns with `mod common; use common::{make_chunk, make_chunks};` (keep `make_chunk` since other helpers use it — check: `make_chunk` is only used by `make_chunks`, so just import both).

`tests/vector_search_lancedb.rs`:
```rust
//! LanceDB vector store round-trip tests (feature-gated; run with --features lancedb).
#![cfg(feature = "lancedb")]

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

    let stats = store.stats().await;
    assert_eq!(stats.total_vectors, chunks.len());

    let query_vec = embedder.embed_one("authenticate user login")?;
    let results = store.search(&query_vec, chunks.len()).await?;
    assert!(!results.is_empty());
    let top: Vec<&str> = results.iter().take(3).map(|h| h.symbol_name.as_str()).collect();
    let auth_related = ["login_user", "logout_user", "validate_token", "handle_request"];
    assert!(top.iter().any(|n| auth_related.contains(n)), "top3: {top:?}");

    store.remove_file("src/auth.rs").await?;
    assert_eq!(store.stats().await.total_vectors, chunks.len() - 3);
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
    assert_eq!(store.stats().await.total_vectors, make_chunks().len());
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
    assert_eq!(store.stats().await.total_vectors, 0);
    Ok(())
}
```
(`fva::error::Result` import — check `make_chunks` returns `Vec<CodeChunk>` so `?` works with `Result<()>`.)

Run: `cargo test --features lancedb --test vector_search_lancedb`
Expected: FAIL — `LanceDbVectorStore` not found.

- [ ] **Step 3: Implement `src/vector/lancedb.rs`**

```rust
//! Native LanceDB vector store.
//!
//! Async (tokio) API wrapped by the async VectorStore trait. Data persists
//! automatically in the Lance format — `persist()` is a no-op.

use std::sync::Arc;

use arrow_array::{
    FixedSizeListArray, Float32Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::ExecutableQuery;
use lancedb::{Connection, DistanceType, Table};

use super::{VectorHit, VectorStats, VectorStore, preview};
use crate::error::{FvaError, Result};
use crate::indexer::chunker::CodeChunk;

const TABLE_NAME: &str = "chunks";

pub struct LanceDbVectorStore {
    table: Table,
    dimensions: usize,
}

fn schema(dimensions: usize) -> Schema {
    Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("relative_path", DataType::Utf8, false),
        Field::new("symbol_name", DataType::Utf8, false),
        Field::new("symbol_kind", DataType::Utf8, false),
        Field::new("language", DataType::Utf8, false),
        Field::new("start_line", DataType::Int64, false),
        Field::new("end_line", DataType::Int64, false),
        Field::new("content_preview", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dimensions,
            ),
            true,
        ),
    ])
}

fn vector_dims(schema: &Schema) -> Option<usize> {
    match schema.field_with_name("vector").ok()?.data_type() {
        DataType::FixedSizeList(_, n) => Some(*n as usize),
        _ => None,
    }
}

fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

impl LanceDbVectorStore {
    pub async fn open(path: std::path::PathBuf, dimensions: usize) -> Result<Self> {
        std::fs::create_dir_all(&path)?;
        let uri = path.to_str().ok_or_else(|| {
            FvaError::Other(format!("non-utf8 vector path: {}", path.display()))
        })?;
        let conn: Connection = lancedb::connect(uri)
            .execute()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb connect: {e}")))?;

        let table = match conn.open_table(TABLE_NAME).execute().await {
            Ok(t) => {
                if vector_dims(t.schema()) != Some(dimensions) {
                    tracing::warn!(
                        "vector dimensions changed, dropping lancedb table — re-index required"
                    );
                    conn.drop_table(TABLE_NAME).execute().await.map_err(|e| {
                        FvaError::Other(format!("lancedb drop_table: {e}"))
                    })?;
                    conn.create_empty_table(TABLE_NAME, Arc::new(schema(dimensions)))
                        .execute()
                        .await
                        .map_err(|e| FvaError::Other(format!("lancedb create: {e}")))?
                } else {
                    t
                }
            }
            Err(_) => conn
                .create_empty_table(TABLE_NAME, Arc::new(schema(dimensions)))
                .execute()
                .await
                .map_err(|e| FvaError::Other(format!("lancedb create: {e}")))?,
        };

        Ok(Self { table, dimensions })
    }
}

impl VectorStore for LanceDbVectorStore {
    async fn upsert_chunks(&self, chunks: &[CodeChunk], vectors: &[Vec<f32>]) -> Result<()> {
        if chunks.len() != vectors.len() {
            return Err(FvaError::Other(format!(
                "chunk/vector count mismatch: {} vs {}",
                chunks.len(),
                vectors.len()
            )));
        }
        if let Some(path) = chunks.first().map(|c| c.relative_path.as_str()) {
            self.remove_file(path).await?;
        }
        let n = chunks.len();
        let batch = RecordBatch::try_new(
            Arc::new(schema(self.dimensions)),
            vec![
                Arc::new(StringArray::from(chunks.iter().map(|c| c.id.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.relative_path.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.symbol_name.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.symbol_kind.clone()).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| c.language.clone()).collect::<Vec<_>>())),
                Arc::new(Int64Array::from(chunks.iter().map(|c| c.start_line as i64).collect::<Vec<_>>())),
                Arc::new(Int64Array::from(chunks.iter().map(|c| c.end_line as i64).collect::<Vec<_>>())),
                Arc::new(StringArray::from(chunks.iter().map(|c| preview(&c.content, 200)).collect::<Vec<_>>())),
                Arc::new(FixedSizeListArray::from_iter_primitive::<arrow_array::types::Float32Type, _, _>(
                    vectors.iter().map(|v| Some(v.iter().map(|x| Some(*x)).collect::<Vec<_>>())).collect::<Vec<_>>(),
                    self.dimensions,
                )),
            ],
        )
        .map_err(|e| FvaError::Other(format!("arrow batch: {e}")))?;

        self.table
            .add(batch)
            .execute()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb add: {e}")))?;
        Ok(())
    }

    async fn remove_file(&self, relative_path: &str) -> Result<()> {
        let sql = format!("relative_path = '{}'", escape_sql_literal(relative_path));
        self.table
            .delete(sql)
            .await
            .map_err(|e| FvaError::Other(format!("lancedb delete: {e}")))?;
        Ok(())
    }

    async fn search(&self, query_vector: &[f32], limit: usize) -> Result<Vec<VectorHit>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let stream = self
            .table
            .query()
            .nearest_to(&query_vector[..])
            .map_err(|e| FvaError::Other(format!("lancedb query: {e}")))?
            .distance_type(DistanceType::Cosine)
            .limit(limit as u32)
            .execute()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb search: {e}")))?;

        let batches: Vec<RecordBatch> = stream
            .try_collect()
            .await
            .map_err(|e| FvaError::Other(format!("lancedb stream: {e}")))?;

        let mut hits = Vec::new();
        for batch in batches {
            let schema = batch.schema();
            let idx = |name: &str| -> Result<usize> {
                schema
                    .index_of(name)
                    .map_err(|e| FvaError::Other(format!("column {name}: {e}")))
            };
            let (i_chunk, i_path, i_sym, i_kind, i_lang, i_start, i_end, i_prev, i_dist) = (
                idx("chunk_id")?, idx("relative_path")?, idx("symbol_name")?,
                idx("symbol_kind")?, idx("language")?, idx("start_line")?,
                idx("end_line")?, idx("content_preview")?, idx("_distance")?,
            );
            let str_at = |name: &str, i: usize| -> Result<String> {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .ok_or_else(|| FvaError::Other(format!("{name} not utf8")))?
                    .value(i)
                    .to_string()
                    .pipe(Ok)
            };
            let int_at = |i: usize, j: usize| -> Result<usize> {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or_else(|| FvaError::Other("int column"))?
                    .value(j) as usize
            };
            let dist_at = |i: usize, j: usize| -> Result<f32> {
                batch
                    .column(i)
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| FvaError::Other("distance column"))?
                    .value(j)
            };
            for row in 0..batch.num_rows() {
                hits.push(VectorHit {
                    chunk_id: str_at("chunk_id", row)?,
                    relative_path: str_at("relative_path", row)?,
                    symbol_name: str_at("symbol_name", row)?,
                    symbol_kind: str_at("symbol_kind", row)?,
                    language: str_at("language", row)?,
                    start_line: int_at(i_start, row)?,
                    end_line: int_at(i_end, row)?,
                    content_preview: str_at("content_preview", row)?,
                    score: 1.0 - dist_at(i_dist, row)?,
                });
            }
        }
        Ok(hits)
    }

    async fn stats(&self) -> VectorStats {
        let total = self
            .table
            .count_rows(None)
            .await
            .map_err(|e| FvaError::Other(format!("lancedb count_rows: {e}")))
            .unwrap_or(0) as usize;
        VectorStats {
            total_vectors: total,
            dimensions: self.dimensions,
        }
    }

    async fn persist(&self) -> Result<()> {
        Ok(()) // ponytail: Lance format auto-persists
    }
}
```
Notes for the implementer (fix at compile time):
- `.pipe(Ok)` is not std — replace `str_at` with an explicit match/`Ok(...)` expression. If the closure-in-closure style fights borrowck, inline the column extraction in the row loop instead.
- `StringArray::from(Vec<String>)` — use `StringArray::from(v.as_slice())` if `From<Vec<String>>` isn't implemented in arrow 55; the stable form is `StringArray::from(Vec<String>)` (arrow-array implements `From<Vec<String>>` — keep as-is unless compile fails).
- `count_rows(None)` — if the signature is `count_rows(&self, filter: Option<&str>)`, pass `None`; if it's `count_rows()` in 0.30, drop the arg.
- `DistanceType` — if not re-exported at crate root in 0.30, use `lancedb::index::DistanceType`.
- `nearest_to(&query_vector[..])` — if `IntoQueryVector` isn't implemented for `&[f32]` but is for `Vec<f32>`, pass `query_vector.to_vec()`.
- `conn.open_table(...).execute()` — if `open_table` returns `OpenTableBuilder` requiring `.execute().await`, the code above is right.

- [ ] **Step 4: Wire into `build_vector_store`**

`src/vector/mod.rs`: add `#[cfg(feature = "lancedb")] mod lancedb;` and `#[cfg(feature = "lancedb")] pub use lancedb::LanceDbVectorStore;` near the top. Replace the two `"lancedb" | "lancedb-native"` arms of `build_vector_store` with:
```rust
#[cfg(feature = "lancedb")]
"lancedb" | "lancedb-native" => Ok(Arc::new(LanceDbVectorStore::open(path, dimensions).await?)),
#[cfg(not(feature = "lancedb"))]
"lancedb" | "lancedb-native" => Err(FvaError::Config(
    "lancedb requires building with --features lancedb".into(),
)),
```
(`path` is computed at the top of the fn — Task 1 moved it there; `flat` arm uses the same `path`.)

- [ ] **Step 5: Run tests**

Run: `cargo test --features lancedb`
Expected: PASS — all tests including the three new lancedb tests. Also run `cargo test` (no features) — flat-only path must still pass (the lancedb test file self-skips via `#![cfg(feature = "lancedb")]`).

Run: `cargo clippy --all-targets --features lancedb` — fix warnings (e.g. the row-loop closures may trigger `redundant_closure` or borrowck complaints; prefer inlining).

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock src/vector/mod.rs src/vector/lancedb.rs tests/vector_search.rs tests/vector_search_lancedb.rs tests/common/mod.rs
git commit -m "feat: native LanceDB vector store backend"
```

---

### Task 3: Default feature, default backend rename, docs

**Files:**
- Modify: `Cargo.toml` (`default = ["mimalloc", "lancedb"]`)
- Modify: `src/config.rs` (`default_vector_backend()` → `"lancedb"`)
- Modify: `README.md` (config example comment)
- Modify: `config.example.toml` (backend comment + default)
- Modify: `CLAUDE.md` (vector/ module, dependencies, key design decisions)
- Modify: `AGENTS.md` (gotchas)
- Test: `cargo test`, `cargo clippy --all-targets`

- [ ] **Step 1: Flip default features + default backend**

`Cargo.toml` line 62: `default = ["mimalloc"]` → `default = ["mimalloc", "lancedb"]`.

`src/config.rs` line 229-231:
```rust
fn default_vector_backend() -> String {
    "lancedb".to_string()
}
```

- [ ] **Step 2: Update docs**

`README.md` (~line 173):
```toml
[vector]
backend = "lancedb"      # "lancedb" (default, native LanceDB) | "flat" | "lancedb-native" (alias)
db_path = "vectors"
```
Also update the data storage section if it mentions `vectors/vectors.bin` (now `vectors/` holds the Lance table).

`config.example.toml` (~lines 47-48):
```toml
# Vector store backend: "lancedb" (default, native LanceDB) | "flat" (in-memory + bincode)
backend = "lancedb"
```

`CLAUDE.md`:
- Architecture list: `vector/` gains `lancedb.rs   LanceDbVectorStore — native LanceDB (Lance format, async API)`
- Dependencies: `**Optional**: `lancedb` (feature-gated)` → note it is now a default feature; keep the feature flag for `--no-default-features` builds
- Key Design Decisions: add one line: `VectorStore is async (LanceDB API); the indexer splits into a sync rayon parse/embed phase and a sequential async upsert phase.`
- Data Storage: `vectors/` — Lance table (flat store's `vectors.bin` no longer written by default)

`AGENTS.md` gotchas: replace the `lancedb` bullet:
- `lancedb is a default feature (native backend is the default). `--no-default-features` drops it and falls back to flat-only (config backend = "flat"). Building with lancedb requires protoc (protobuf-compiler).`

- [ ] **Step 3: Full verification**

Run: `cargo test` — all tests pass (this now also runs `vector_search_lancedb.rs` since the feature is default). This run commits the `Cargo.lock` change too.
Run: `cargo clippy --all-targets`
Run: `cargo build` — verify a plain build works.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/config.rs README.md config.example.toml CLAUDE.md AGENTS.md
git commit -m "feat: make native LanceDB the default vector backend"
```
