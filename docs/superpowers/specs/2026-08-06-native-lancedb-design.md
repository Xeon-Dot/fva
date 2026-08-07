# Native LanceDB Backend

**Date:** 2026-08-06
**Status:** Implemented (2026-08-07)

## Goal

Wire up the real LanceDB backend. `backend = "lancedb"` (the config default) is the **only** vector backend — the flat store was deleted. The `lancedb` cargo feature is a default feature.

## Backend Mapping

| `backend` value | Behavior |
|---|---|
| `"lancedb"` (default), `"lancedb-native"` (alias) | Native LanceDB store (`src/vector/lancedb.rs`) |
| anything else | config error (`unknown vector backend`) |

- `Cargo.toml`: `default = ["mimalloc", "lancedb"]`; arrow-array/arrow-schema pinned to `58` (matches lancedb 0.30's arrow).
- Built with `--no-default-features`: `"lancedb"` → config error (existing message pattern).
- `src/vector/flat.rs` deleted; `bincode` removed from vector storage (still used by graph/wiki).

## Async Trait Conversion

`VectorStore` methods that touch the async LanceDB API use explicit `Pin<Box<dyn Future<Output = Result<...>> + Send + '_>>` signatures (async fn in traits is not dyn-compatible on the current toolchain; `stats`/`persist` stayed sync).

Ripple:
- `index_chunks` → async
- `Indexer::index_all` / `index_file` → async; `spawn_background_index` → `tokio::spawn`
- `HybridQueryEngine::hybrid_search` / `semantic_search` → async
- `FvaEngine::new` / `shutdown` → async
- `bench::run` → async; MCP handlers `.await`
- `tests/vector_search.rs`, `tests/indexer_walk.rs` → `#[tokio::test]` (indexer_walk uses `multi_thread` flavor)

`wiki` does not touch `VectorStore` — unaffected.

## LanceDbVectorStore

- `open()`: `connect(uri)`; open existing table or `create_empty_table`. Dimension mismatch → drop table + warn. Re-index required.
- Schema columns: `chunk_id`, `relative_path`, `symbol_name`, `symbol_kind`, `language` (utf8), `start_line`, `end_line` (i64), `content_preview` (utf8), `vector` (FixedSizeList<f32>, nullable).
- `upsert_chunks`: `delete("relative_path = '<escaped>'")` + `add(batch)` — remove-then-insert semantics.
- `remove_file`: SQL delete; no matching rows → Ok.
- `search`: `nearest_to` + limit (via `QueryBase`); Cosine metric; score = `1 - _distance`.
- `stats`: `count_rows` run on a dedicated current-thread runtime (works from any tokio context); `persist`: no-op (auto-persisted).
- No IVF-PQ index initially — exact search. Add as a knob when corpus exceeds ~100k chunks.

## Indexer Restructure

Rayon parallelism + async ops don't mix inside `par_iter`:

1. Rayon phase (sync): read → parse → chunk → embed → collect `(relative_path, hash, chunks, vectors)`
2. Async phase: per-file `remove_file().await` + `upsert_chunks().await`, sequential

`index_file` is a thin wrapper: sync `parse_and_embed` + async `commit_file`.

## Prerequisites & Non-Changes

- Building with the `lancedb` feature requires `protoc` (`sudo apt install protobuf-compiler`).
- No migration of `.fva/vectors/vectors.bin` — re-index replaces it. Old file ignored (deleted on re-index).
- Docs updated: README.md, config.example.toml, CLAUDE.md, AGENTS.md.

## Testing

- `tests/vector_search.rs` — quality/perf tests against the LanceDB store (6 tests).
- `tests/vector_search_lancedb.rs` — round-trip (upsert → search → remove_file → stats), persist across reopen, dimension-change drop (3 tests).
- `tests/indexer_walk.rs` — end-to-end index on the repo itself.
- `tests/common/mod.rs` — shared chunk fixtures.
