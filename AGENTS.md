# FVA — Agent Instructions

Rust binary crate (edition 2024). Hybrid codebase search engine exposing MCP server on stdio.

## Build & Test

```bash
cargo build --release          # binary: target/release/fva
cargo test                     # integration tests in tests/
cargo test --features bench    # bench tests gated behind `bench` feature
cargo test --test vector_search_lancedb  # single integration test
cargo clippy                   # lint
cargo fmt --check              # formatting
```

Requires Rust 1.75+. Uses sccache (configured in `.cargo/config.toml`).

## Features

- `default` = `["mimalloc", "lancedb"]`
- `bench` — enables `fva bench` subcommand and bench module

## Source Layout

| Module | Path | Purpose |
|--------|------|---------|
| CLI + main | `src/main.rs` | clap subcommands: serve, index, status, search, wiki, bench, version, upgrade |
| Engine | `src/engine.rs` | `FvaEngine` — orchestrates all subsystems, holds `Arc` refs |
| FFF | `src/fff/` | Frecency-ranked fuzzy file search + grep (wraps `fff-search`) |
| AST Indexer | `src/indexer/` | Tree-sitter parsing, chunking, parallel pipeline (rayon) |
| Vector Store | `src/vector/` | LanceDB backend (the only vector backend) |
| Embedding | `src/embedding/` | Local hash + Voyage API providers |
| Call Graph | `src/graph/` | petgraph-based callers/callees |
| Query | `src/query/` | Hybrid search fusion + smart context builder |
| MCP | `src/mcp/` | rmcp tool handlers (stdio transport) |
| Wiki | `src/wiki/` | Markdown knowledge base with semantic search |
| Config | `src/config.rs` | TOML config: project `.fva.toml` > global `~/.config/fva/config.toml` |

## Conventions

- `Arc` for shared subsystems (indexer, vectors, graph, embedder, wiki)
- `Embedder` is the **only trait** in the crate; all providers implement `Arc<dyn Embedder>`
- Errors via `thiserror` (`FvaError` / `Result<T>`)
- Async: tokio with `#[tokio::main]`
- Data dir: `.fva/` (auto-created, auto-gitignored with `*`)
- Test data dir: `.fva-test/` (also gitignored)
- Config precedence: CLI flags > project `fva.toml` > global config > defaults
- Logging: `tracing` + `tracing-subscriber`, controlled by `RUST_LOG` or config `mcp.log_level`
- CLI output: `src/cli_output.rs` (table-formatted, uses `console`, `indicatif`, `comfy-table`)
- `ponytail:` comments mark deliberate simplifications with known ceilings — respect them

## Key Gotchas

- `cargo test` needs network on first run (downloads tree-sitter grammar data for test fixtures)
- LanceDB vector store is async — `LanceDbVectorStore::open()` is an `async fn`
- `.fva/` directory is sandboxed to project root — never index outside it
- `bench` module is fully `#[cfg(feature = "bench")]` — won't compile references without it
- The `fva search` and `fva status` subcommands auto-index if no index exists yet
- **Two separate vector stores:** LanceDB for code chunks (`vectors/`), bincode file for wiki (`wiki_vectors.bin`) — they are independent
- **Dimension mismatch wipes the LanceDB table** — changing embedding dimensions drops and recreates the vector store
- **Git root discovery:** FFF uses `git2::Repository::discover()` to walk up to the repo root, so indexing scope may extend beyond `--path`
- **Indexer is 2-phase:** rayon parallel parse+embed (sync), then async commit (vector upsert + graph index) — content-hash (`blake3`) skips unchanged files

## MCP Tools (for agent context)

`hybrid_search` is the default. Prefer over repeated grep+read cycles. Full tool list:
`hybrid_search`, `semantic_search`, `find_files`, `grep`, `get_chunks`, `get_symbol_info`, `get_call_graph`, `get_smart_context`, `index_status`, `wiki_write`, `wiki_read`, `wiki_search`, `wiki_list`, `wiki_delete`
