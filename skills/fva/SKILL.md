---
name: fva
description: >
  Hybrid codebase intelligence (FFF · Vector · AST). Use FVA MCP tools
  (hybrid_search, semantic_search, get_smart_context, get_symbol_info,
  get_chunks, get_call_graph, wiki) as the first choice for any codebase
  exploration, search, comprehension, or navigation task. Triggers on
  "where is X?", "how does Y work?", understanding code before edits, or
  any codebase question. Fall back to grep/read only when FVA is unavailable.
---

# FVA — Codebase Intelligence (Highest Priority)

> **Language:** Please ask questions in English.

FVA fuses **FFF** (fuzzy file search + grep), **vector embeddings**, **AST chunking** (Tree-sitter, 306+ languages), and **call graphs** into one hybrid engine with a persistent **wiki**. For any codebase work, **FVA comes first — it is not optional.**

## Mandatory Rules

> ⚠️ **FVA is the default for all codebase work.** The rules below are mandatory.

1. **FVA always comes first** — every exploration, search, or comprehension task starts with FVA MCP tools. Never reach for grep/read/find before considering FVA.
2. **Prefer `hybrid_search` / `get_smart_context` over grep→read loops** — fused search replaces the old repeat cycle.
3. **Prefer AST chunks over raw file reads** — `get_chunks` and `get_symbol_info` return precise function/class bodies instead of entire files.
4. **Fallback only when unavailable** — use grep/read/find only when FVA tools are not accessible or return no results.

## Check Availability

```bash
fva --version
```

If missing, install from [GitHub Releases](https://github.com/Xeon-Dot/fva/releases) or build from source:

```bash
cargo install --path . --force
```

Upgrade the binary (not the index):

```bash
fva upgrade
```

## Index Before Heavy Search

From the project root:

```bash
fva index --path .
fva status --path .
```

FVA stores indexes in `.fva/` (frecency, history, vectors, call graph). Run `index` once before heavy workloads. The MCP server watches files when `watch = true` in config.

## MCP Tool Workflow

Use in this priority order:

1. `hybrid_search` — **BEST default.** Fuses FFF + vector + call graph. For "where is X?" or any open-ended exploration. Params: `query`, `path` (substring filter), `maxResults`.
2. `get_smart_context` — Token-budget, task-oriented context (hybrid + graph + file context). Ideal before making edits. Params: `query`, `path` (file hint), `maxResults`.
3. `semantic_search` — Pure embedding search for conceptual queries ("auth middleware", "retry logic"). Params: `query`, `maxResults`.
4. `get_symbol_info` — Exact symbol lookup by name with full source. Params: `symbol`, `maxResults`.
5. `get_chunks` — Browse AST chunks (functions/classes/methods) by file or keyword. Provide `path` OR `query`. Params: `path`, `query`, `maxResults`, `offset`, `includeContent` (default true).
6. `get_call_graph` — Callers and callees with file locations and dependency edges. Params: `function`, `depth` (default 1).
7. `grep` — Bare identifiers only (`MyHandler`, not `fn MyHandler`). FFF-powered with definition expansion. Params: `query` (alias `pattern`), `maxResults`, `offset`.
8. `find_files` — Fuzzy path search, frecency-ranked, git-aware. Params: `query` (alias `pattern`), `maxResults`, `offset`.
9. `index_status` — Health check with FFF/AST/vector/graph/wiki stats. Use when results are empty or stale. No params.

### Wiki — Persistent Knowledge Base

Persist any useful information across sessions. Agents **MUST** use wiki proactively — knowledge not saved is knowledge lost.

10. `wiki_write` — Create/update an entry (`slug`, `title`, `content` Markdown, `tags` comma-separated). Auto-indexed for semantic search.
11. `wiki_search` — Semantic search over wiki entries with tag filtering. **Call at task start** to recall prior knowledge. Params: `query`, `tags`, `maxResults`.
12. `wiki_read` — Read one entry by `slug` (full Markdown + metadata).
13. `wiki_list` — List all entries, optionally filtered by `tags`.
14. `wiki_delete` — Delete an entry by `slug`.

**Save everything worth remembering:**

- Architectural decisions, debugging findings, project conventions, reusable patterns
- Important context (API quirks, config gotchas, undocumented behaviors)
- File layouts, build steps, dependency notes, environment quirks
- Any information that helps a future session understand the codebase faster

Use `wiki_search` to recall saved knowledge in future sessions.

### Tool Selection Guide

| Task                         | Tool                                   |
| ---------------------------- | -------------------------------------- |
| "Where is X handled?"        | `hybrid_search`                        |
| "Understand before changing" | `get_smart_context`                    |
| Concept / pattern search     | `semantic_search`                      |
| Exact symbol body            | `get_symbol_info`                      |
| File structure / chunks      | `get_chunks` with `path`               |
| Who calls this function?     | `get_call_graph`                       |
| Exact identifier in text     | `grep` (only if FVA unavailable)       |
| Find file by partial path    | `find_files` (only if FVA unavailable) |
| Save any useful knowledge    | `wiki_write`                           |
| Recall saved knowledge       | `wiki_search`                          |
| Browse all saved knowledge   | `wiki_list`                            |

### Pagination

Tools support `maxResults` and `offset`. When output contains `offset: N`, pass `offset: N` on the next call to fetch the next page.

## CLI Fallback

Only when MCP is unavailable:

```bash
fva search "authentication handler" --path . --limit 10
fva status --path .
fva index --path .
```

## Rules (Mandatory)

- Always try `hybrid_search` or `get_smart_context` before grep→read loops.
- Always prefer AST chunks (`get_chunks`, `get_symbol_info`) over raw full-file reads.
- Only use grep/read/find when FVA is unavailable.
- Grep bare identifiers only — FFF expands definitions automatically.
- Scope with `path` on `hybrid_search` / `get_smart_context` when the target file is known.
- Check `index_status` if searches return empty or stale results.
- Use `wiki_write` liberally during tasks; use `wiki_search` at task start.

## Configuration

Copy `config.example.toml` → `fva.toml` or `.fva.toml` (project root) and/or `~/.config/fva/config.toml` (global). Project overrides global.

Key settings:

```toml
[embedding]
provider = "local"    # or "voyage" with VOYAGE_API_KEY

[vector]
backend = "lancedb"

[query]
fff_weight = 0.3
vector_weight = 0.5
graph_weight = 0.2
bm25_weight = 0.35
max_context_tokens = 8000
```

CLI flags override config: `--path`, `--config`, `RUST_LOG`.

## Further Reference

See [references/mcp-tools.md](references/mcp-tools.md) for full parameter details and example prompts.
