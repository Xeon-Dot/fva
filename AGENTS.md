# AGENTS.md

See `CLAUDE.md` for full architecture, data flow, and design decisions.

## Commands

```bash
cargo build                  # dev build
cargo test                   # all tests (unit + integration)
cargo test --test vector_search   # single integration test
cargo test -- chunker::tests      # unit tests in a module
cargo clippy --all-targets   # lint (run before finishing work)
```

No CI test gate — only a release workflow. Run `cargo test` and `cargo clippy --all-targets` yourself.

## Gotchas

- **Rust edition 2024** — not 2021. Some older patterns/syntax may not apply.
- **sccache is the rustc-wrapper** (`.cargo/config.toml`). Builds fail if sccache isn't installed. Workaround: `RUSTC_WRAPPER= cargo build`.
- **`mimalloc` is a default feature** — the global allocator. `--no-default-features` drops it.
- **`lancedb` is a default feature** — native LanceDB is the only vector backend. `--no-default-features` disables it and `backend = "lancedb"` then returns a config error. Building lance requires `protoc` (protobuf-compiler) on the path.
- **ChunkStore is in-memory only** — not persisted. VectorStore and CallGraphStore persist to `.fva/`. After restart, hybrid search degrades until background re-index completes.
- **Integration tests** (`tests/`) use `tempfile` for isolated dirs. No external services needed.

## Conventions

- Config precedence: defaults → `~/.config/fva/config.toml` → `fva.toml`/`.fva.toml` → `--config` flag.
- The project dogfoods itself: `opencode.jsonc` runs `fva` as its own MCP server.
- Linux release builds target `*-unknown-linux-gnu` (glibc). musl static binaries cannot `dlopen` the tree-sitter grammar `.so`s that `tree-sitter-language-pack` downloads at runtime.
