# Native LanceDB Backend

**Date:** 2026-08-06
**Status:** Approved

## Goal

Wire up the real LanceDB backend. `backend = "lancedb"` (the config default) becomes the native LanceDB store instead of an alias for the flat store. The `lancedb` cargo feature becomes a default feature.

## Backend Mapping

| `backend` value | Behavior |
|---|---|
| `"lancedb"` (default), `"lancedb-native"` | Native LanceDB store (`src/vector/lancedb.rs`) |
| `"flat"` | Existing `FlatVectorStore` |

- `Cargo.toml`: `default = ["mimalloc", "lancedb"]`
- Built with `--no-default-features`: `"lancedb"` → config error (existing message pattern)
- `src/vector/mod.rs:32` stale comment removed; "not yet wired" error deleted

## Async Trait Conversion

All 5 `VectorStore` trait methods become `async fn`. Rust 1.97 supports dyn-compatible async fn in traits; if dyn dispatch fails, fall back to explicit `BoxFuture` signatures.

Ripple:
- `index_chunks` → async
- `FlatVectorStore` — async signatures, unchanged sync bodies
- `Indexer::index_all` / `index_file` → async; `spawn_background_index` → `tokio::spawn`
- `HybridQueryEngine::hybrid_search` / `semantic_search` → async
- `FvaEngine::shutdown` → async (persist awaits)
- `bench::run` → async; MCP handlers `.await`
- `tests/vector_search.rs`, `tests/indexer_walk.rs` → `#[tokio::test]`

`wiki` does not touch `VectorStore` — unaffected.

## LanceDbVectorStore

- `open()`: `connect(uri)`; open existing table or `create_empty_table`. Dimension mismatch → drop table + warn (mirrors flat's dimension-change warning). Re-index required.
- Schema columns: `chunk_id`, `relative_path`, `symbol_name`, `symbol_kind`, `language` (utf8), `start_line`, `end_line` (i64), `content_preview` (utf8), `vector` (FixedSizeList<f32>).
- `upsert_chunks`: `delete("relative_path = '<escaped>'")` + `add(batch)` — same remove-then-insert semantics as flat.
- `remove_file`: SQL delete; no matching rows → Ok.
- `search`: `nearest_to` + limit; Cosine metric; score = `1 - _distance` (consistent with flat's `cosine_similarity`).
- `stats`: `count_rows`; `persist`: no-op (auto-persisted).
- No IVF-PQ index initially — exact search parity with flat. Add as a knob when corpus exceeds ~100k chunks.

## Indexer Restructure

Rayon parallelism + async ops don't mix inside `par_iter`:

1. Rayon phase (sync): read → parse → chunk → embed → graph → collect `(relative_path, chunks, vectors)`
2. Async phase: per-file `remove_file().await` + `upsert_chunks().await`, sequential

`index_file` becomes a thin wrapper: sync `parse_file` + async `commit_file`.

## Prerequisites & Non-Changes

- Local dev builds need `protoc` (`sudo apt install protobuf-compiler`). CI already installs it; release already builds `--all-features`.
- No migration of `.fva/vectors/vectors.bin` — re-index replaces it. Old file ignored.
- Docs updated: README.md, config.example.toml, CLAUDE.md, AGENTS.md.

## Testing

- Existing flat-store tests keep passing (async test harness).
- New integration test: LanceDB store round-trip (upsert → search → remove_file → stats) on tempdir.
