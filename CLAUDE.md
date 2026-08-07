# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test Commands

```bash
# Build (dev)
cargo build

# Release build (LTO, stripped, abort-on-panic)
cargo build --release

# Run all tests
cargo test

# Run specific integration test
cargo test --test vector_search
cargo test --test indexer_walk

# Run specific unit test
cargo test -- chunker::tests
cargo test -- tests::chunks_rust_function

# Lint
cargo clippy --all-targets

# Build with LanceDB support
cargo build --features lancedb
```

## Project Overview

FVA (FFF·Vector·AST) is a hybrid codebase intelligence engine for AI coding agents. It combines fuzzy file search, vector embeddings (semantic search), AST chunking via Tree-sitter (306+ languages), and call graphs into a single MCP server.

### Binary

`src/main.rs` — CLI entry point with subcommands:
- `serve` (default) — start MCP server on stdio
- `index` — full index (AST + vectors + call graph) then exit
- `status` — print indexing statistics as JSON
- `search <query>` — hybrid search from CLI
- `bench` — performance benchmark suite
- `upgrade` — self-upgrade via GitHub releases
- `version` — print version

### Architecture (src/)

```
main.rs            CLI entry point (clap)
engine.rs          FvaEngine — central orchestrator holding all subsystems
config.rs          Layered config (TOML): global → project → explicit
error.rs           FvaError enum + Result<T>

mcp/               MCP server (rmcp + schemars)
  server.rs        9 tools: hybrid_search, semantic_search, find_files,
                   grep, get_chunks, get_symbol_info, get_call_graph,
                   get_smart_context, index_status

fff/               FFF integration (fff-search crate)
  mod.rs           Frecency-ranked fuzzy file search, git-aware grep,
                   content indexing, LMDB persistence

indexer/           Background indexing pipeline (rayon parallel)
  parser.rs        Tree-sitter AST parser — 306+ languages via language-pack
  chunker.rs       AST-aware chunking → CodeChunk with full metadata
  store.rs         In-memory ChunkStore (RwLock<HashMap>), content-hash
                   incremental indexing, symbol search

embedding/         Embedding providers
  mod.rs           Embedder trait, build_embedder(), cosine_similarity
  local.rs         Hash-based embedder (n-grams, TF weighting, structure markers)
  voyage.rs        Voyage API embedder (optional, configurable)

vector/            Vector storage
  mod.rs           VectorStore trait (async), index_chunks()/chunk_texts()/preview()
  lancedb.rs       LanceDbVectorStore — native LanceDB (Lance format, async API)

graph/             Call graph
  mod.rs           CallGraphStore — petgraph DiGraph, bincode persistence
  builder.rs       Regex-based call edge extraction from CodeChunks

query/             Hybrid search engine
  hybrid.rs        HybridQueryEngine — 3-stage fusion (FFF → Vector → Graph)
  context.rs       ContextBuilder — token-budget smart context for agents

bench/             Performance benchmark harness
  mod.rs           Measures ops against targets from README
  report.rs        JSON/table report format

upgrade.rs         Self-upgrade via scripts/install.sh / install.ps1
util.rs            estimate_tokens, sort_by_score, http_client, resolve_pagination
```

### Data Flow

1. **Indexing**: FFF scans files → Tree-sitter parses AST → chunker extracts functions/classes → embedder creates vectors → vector store indexes + graph builder extracts call edges
2. **Search CLI**: `fva index --path .` → `fva search "query" --path .`
3. **MCP Server**: `fva serve --path .` — background index, then serve tools on stdio

### Data Storage (`.fva/`)

- `frecency/` — LMDB database for frecency tracking
- `history/` — LMDB query history
- `vectors/` — Lance table (native LanceDB vector store)
- `call_graph.bin` — bincode-serialized call graph

### Key Design Decisions

- **ChunkStore is in-memory (not persisted)** — populated on each `index_all()`. VectorStore and CallGraphStore persist to disk. After restart, hybrid search falls back to VectorHit metadata while the background index rebuilds.
- **VectorStore is async** — the LanceDB API is tokio-based; the indexer splits into a sync rayon parse/embed phase and a sequential async upsert phase.
- **HybridQueryEngine.merge_hit()** — uses `max()` per signal (FFF/vector/graph), sums scores. This means the same chunk can be boosted by all three signals simultaneously.
- **LanceDbVectorStore** — native LanceDB with a FixedSizeList f32 vector column, cosine distance, `score = 1 - distance`; auto-persists (Lance format), `persist()` is a no-op.
- **LocalEmbedder** — zero-dependency hash-based embedding with: multi-hash (2 salts) feature hashing, digit-boundary token splitting, TF weighting (sqrt), char n-grams (2/3/4), CamelCase/snake_case decomposition, exact-match boost for cased identifiers, structure markers.
- **Incremental indexing** — BLAKE3 content hash. `needs_reindex()` returns false if hash unchanged; old vectors/graph entries are removed before re-index.

### Configuration

Layered precedence: defaults → `~/.config/fva/config.toml` → `fva.toml`/`.fva.toml` (project root) → `--config` flag.

Key settings in `config.example.toml`: embedding provider (local/voyage), vector backend (lancedb), hybrid search weights (fff=0.3, vector=0.5, graph=0.2), max context tokens (8000).

### Dependencies

- **Required**: `fff-search`, `rmcp` (MCP), `tree-sitter-language-pack` (AST), `petgraph` (graph), `blake3`, `rayon`, `clap`, `serde`, `parking_lot`, `tracing`, `git2`, `lancedb` + `arrow-*` (default features — the only vector backend), `futures`
- **Optional**: `mimalloc` (default on, global allocator)
- **Dev**: `tempfile`
