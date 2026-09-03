# FVA — FFF · Vector · AST

Hybrid codebase intelligence engine for AI coding agents. FVA combines **FFF** (fuzzy file search), **vector embeddings**, and **AST** chunking via Tree-sitter, plus call graphs, into a single MCP server — so agents search by path, content, meaning, and structure in one pass.

## Why FVA

Most agent workflows chain `grep` → `read_file` → repeat. FVA replaces that loop with fused search:

- **File discovery** — frecency-ranked fuzzy paths (FFF)
- **Semantic recall** — embedding search over AST chunks
- **Structural context** — call graph neighbors and symbol bodies
- **One MCP server** — stdio transport, local-first, no telemetry

## Install

Pre-built binaries are published on [GitHub Releases](https://github.com/Xeon-Dot/fva/releases) for:

| Platform | amd64 | arm64 |
| -------- | ----- | ----- |
| Linux    | ✓     | ✓     |
| Windows  | ✓     | ✓     |
| macOS    | —     | ✓     |

### One-liner

**Linux / macOS (Apple Silicon)**

```bash
curl -fsSL https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.sh | bash
```

**Windows (PowerShell)**

```powershell
irm https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.ps1 | iex
```

Installs to `~/.local/bin` (Unix) or `%LOCALAPPDATA%\Programs\fva\bin` (Windows) and adds the directory to your user `PATH`.

### Pin a version

```bash
FVA_VERSION=v0.2.0 curl -fsSL https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.sh | bash
```

```powershell
$env:FVA_VERSION = "v0.2.0"; irm https://raw.githubusercontent.com/Xeon-Dot/fva/main/scripts/install.ps1 | iex
```

### Manual download

Download the archive for your platform from [Releases](https://github.com/Xeon-Dot/fva/releases), verify against `SHA256SUMS.txt`, and place `fva` (or `fva.exe`) on your `PATH`.

### Build from source

Requires [Rust](https://rustup.rs/) 1.75+.

```bash
git clone https://github.com/Xeon-Dot/fva.git
cd FVA
cargo build --release
```

Binary: `target/release/fva` (`.exe` on Windows).

Intel Mac has no pre-built binary yet — use `cargo install --path .` or build from source.

## Quick Start

```bash
# Verify install
fva --version

# Start MCP server (default mode)
fva --path /path/to/project

# Build full index (AST + vectors + call graph)
fva index --path .

# Check index status
fva status --path .

# CLI hybrid search
fva search "authentication handler" --path . --limit 5
```

On first run, FVA creates a `.fva/` directory in the project root for indexes, frecency data, and vectors.

## MCP Client Setup

Add FVA to your MCP client config. Use the installed binary path on your system.

Ready-to-copy examples for each agent tool live in [`examples/mcp-clients/`](examples/mcp-clients/). See [`manifest.json`](examples/mcp-clients/manifest.json) for install paths.

| Agent              | Example file                   | Install location                      |
| ------------------ | ------------------------------ | ------------------------------------- |
| Cursor (project)   | `cursor.project.mcp.json`      | `<project>/.cursor/mcp.json`          |
| Claude Desktop     | `claude-desktop.*.json`        | OS-specific — see manifest            |
| Claude Code        | `claude-code.project.mcp.json` | `<project>/.mcp.json`                 |
| VS Code / Copilot  | `vscode.workspace.mcp.json`    | `<project>/.vscode/mcp.json`          |
| Windsurf / Cascade | `windsurf.mcp_config.json`     | `~/.codeium/windsurf/mcp_config.json` |
| Zed                | `zed.context_servers.json`     | Merge into Zed `settings.json`        |
| Continue           | `continue.fva.yaml`            | `<project>/.continue/mcpServers/`     |
| Gemini CLI         | `gemini-cli.settings.json`     | Merge into `~/.gemini/settings.json`  |
| Cline / Roo Code   | `cline.mcp_settings.json`      | Extension MCP settings                |

**macOS / Linux (generic `mcpServers` format)**

```json
{
  "mcpServers": {
    "fva": {
      "command": "fva",
      "args": ["--path", "/path/to/your/project"],
      "env": { "RUST_LOG": "info" }
    }
  }
}
```

**Windows**

```json
{
  "mcpServers": {
    "fva": {
      "command": "C:\\Users\\You\\AppData\\Local\\Programs\\fva\\bin\\fva.exe",
      "args": ["--path", "D:\\Dev\\YourProject"],
      "env": { "RUST_LOG": "info" }
    }
  }
}
```

If `fva` is on `PATH`, the short `"command": "fva"` form works on all platforms.

### Recommended agent prompt

```
For codebase exploration, use FVA MCP tools:
- hybrid_search: default — combines file search + semantic + call graph
- semantic_search: natural language concept search
- get_smart_context: token-efficient context for a task
- get_symbol_info / get_chunks: full function/class bodies
- get_call_graph: callers and callees
Prefer hybrid_search over repeated grep+read cycles.
```

## MCP Tools

| Tool                | Description                              |
| ------------------- | ---------------------------------------- |
| `hybrid_search`     | **Default.** FFF + vector + graph fusion |
| `semantic_search`   | Natural language embedding search        |
| `find_files`        | Fuzzy path search, frecency-ranked       |
| `grep`              | Content search with definition expansion |
| `get_chunks`        | AST chunks by file or query              |
| `get_symbol_info`   | Symbol lookup with full source           |
| `get_call_graph`    | Callers/callees of a function            |
| `get_smart_context` | Token-budget smart context builder       |
| `index_status`      | Full indexing statistics                 |

## Configuration

Copy `config.example.toml` to `fva.toml` (or `.fva.toml`) in the project root, and/or to `~/.config/fva/config.toml` for global defaults. Project settings override global; CLI `--config` overrides both.

```toml
[embedding]
provider = "local"    # "local" (default) or "voyage"
model = "voyage-code-3"

[vector]
backend = "lancedb"    # "lancedb" (default, native LanceDB) | "lancedb-native" (alias)
db_path = "vectors"

[query]
fff_weight = 0.3
vector_weight = 0.5
graph_weight = 0.2
bm25_weight = 0.35
max_context_tokens = 8000
```

CLI flags override config: `--path`, `--config`, `--log-level` / `RUST_LOG`.

### Voyage API (optional)

For higher-quality embeddings, switch provider and set your API key:

```toml
[embedding]
provider = "voyage"
```

```bash
export VOYAGE_API_KEY=your-key-here
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    MCP Server (stdio)                        │
│  hybrid_search │ semantic_search │ grep │ find_files │ ... │
└─────────────┬───────────────────────┬───────────────────────┘
              │                       │
     ┌────────▼────────┐     ┌────────▼────────┐
     │   FFF Engine    │     │  AST Indexer    │
     │ frecency+fuzzy  │     │  Tree-sitter    │
     │  git-aware grep │     │  chunk store    │
     └────────┬────────┘     └────────┬────────┘
              │                ┌───────▼────────┐
              │                │  Vector Store  │
              │                │  (flat/LanceDB)│
              │                └───────┬────────┘
              │                ┌───────▼────────┐
              │                │  Call Graph    │
              │                │  (petgraph)    │
              └────────┬───────┴────────┬───────┘
                       │                │
                ┌──────▼────────────────▼──────┐
                │     Hybrid Query Engine      │
                │  FFF → Vector → Graph fusion  │
                └──────────────────────────────┘
```

## Development Status

| Phase | Feature                           | Status  |
| ----- | --------------------------------- | ------- |
| 1     | FFF MCP + Tree-sitter chunking    | Done    |
| 2     | Embedding pipeline + vector store | Done    |
| 3     | Call graph + hybrid query engine  | Done    |
| 4     | Full MCP tool set + CLI           | Done    |
| 5     | Large-scale benchmarks + docs     | Planned |

## Module Structure

```
src/
├── main.rs            # CLI (serve, index, status, search)
├── engine.rs          # Central orchestrator
├── embedding/         # local-hash + voyage providers
├── vector/            # flat vector store (LanceDB optional)
├── graph/             # call graph (petgraph)
├── indexer/           # AST parsing + chunking + pipeline
├── query/             # hybrid search + smart context
├── fff/               # FFF integration
└── mcp/               # MCP tool handlers
```

## Performance Targets

| Operation                  | Target  | Notes                |
| -------------------------- | ------- | -------------------- |
| File search (100k files)   | < 50ms  | FFF frecency + SIMD  |
| Grep (warm index)          | < 100ms | mmap + content index |
| AST chunk (single file)    | < 5ms   | Tree-sitter          |
| Vector search (10k chunks) | < 50ms  | flat brute-force     |
| Hybrid search              | < 200ms | 3-stage fusion       |
| Full index (10k files)     | < 30s   | rayon parallel       |

## Security

- Sandboxed indexing — project root only
- No telemetry — all data stored locally in `.fva/`
- Embeddings are local by default; Voyage only when explicitly configured

## License

MIT
