---
name: fva
description: >
  Always prioritize FVA (FFF · Vector · AST) for hybrid codebase intelligence.
  For any codebase exploration, search, or comprehension task, FVA MCP tools
  must be the first choice. Only fall back to traditional methods (grep, read,
  etc.) when FVA is unavailable.
---

# FVA — Codebase Intelligence (Highest Priority)

FVA combines **FFF** (fuzzy file search + grep), **vector embeddings**, **AST chunking** (Tree-sitter), **call graphs**, and a **wiki knowledge base** into one hybrid search engine. **Any codebase-related task must use FVA first — it is not optional.**

## Mandatory Rules

> ⚠️ **FVA is the default and highest-priority tool for all codebase work.**
> The rules below are **mandatory**, not suggestions.

1. **FVA always comes first** — every codebase exploration, search, or comprehension task starts with FVA MCP tools. Never reach for grep/read/find before considering FVA.
2. **FVA over grep/read loops** — always try `hybrid_search` or `get_smart_context` before falling into the old grep → read → grep cycle.
3. **Alt methods only when FVA is unavailable** — only use traditional approaches (grep, read, find_files, etc.) when FVA MCP tools are not accessible or return no results.
4. **Prefer AST chunks over raw file reads** — use `get_chunks` and `get_symbol_info` to retrieve function/class bodies via AST instead of reading entire files.

## Check Availability

```bash
fva --version
```

If missing, install from [GitHub Releases](https://github.com/Xeon-Dot/fva/releases) or build from source:

```bash
cargo install --path . --force
```

Upgrade the binary (not the project index):

```bash
fva upgrade
```

## Index Before Heavy Search

From the target project root:

```bash
fva index --path .
fva status --path .
```

FVA stores indexes in `.fva/` (frecency, history, vectors, call graph). Run `index` once before heavy workloads; the MCP server watches files when `watch = true` in config.

## MCP Tool Workflow

**Default order** — always try in this sequence first:

1. `hybrid_search` — **always try first** (FFF + vector + call graph)
2. `get_smart_context` — token-budget context before edits
3. `semantic_search` — conceptual queries ("auth middleware", "retry logic")
4. `get_symbol_info` / `get_chunks` — full function/class bodies (AST-aware)
5. `get_call_graph` — callers and callees
6. `grep` — bare identifiers only (`MyHandler`, not `fn MyHandler`)
7. `find_files` — fuzzy path discovery
8. `index_status` — check indexing progress

### Wiki (Knowledge Base)

Persist any useful information across sessions. Agents **MUST** use wiki proactively — knowledge not saved is knowledge lost.

9. `wiki_write` — create/update a knowledge entry (slug, title, content, tags)
10. `wiki_read` — read a wiki entry by slug
11. `wiki_delete` — delete a wiki entry
12. `wiki_search` — semantic search over wiki entries with tag filtering
13. `wiki_list` — list all wiki entries, optionally filtered by tags

**Save everything worth remembering:**
- Architectural decisions, debugging findings, project conventions, reusable patterns
- Important context discovered during a task (API quirks, config gotchas, undocumented behaviors)
- Miscellaneous but useful details: file layouts, build steps, dependency notes, environment quirks
- Any information that would help a future session understand the codebase faster

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

Tools support `maxResults` and `offset`. When output includes `offset: N`, pass `offset: N` on the next call.

## CLI Fallback

Only fall back to CLI when MCP is unavailable:

```bash
fva search "authentication handler" --path . --limit 10
fva status --path .
fva index --path .
```

## Rules (Mandatory)

- **Always** use `hybrid_search` or `get_smart_context` before resorting to grep → read loops.
- **Always** use AST chunks (`get_chunks`, `get_symbol_info`) instead of raw full-file reads.
- Only use grep/read/find_files when FVA tools are unavailable.
- Grep bare identifiers only — FFF expands definitions automatically.
- Scope searches with `path` on `hybrid_search` / `get_smart_context` when the target file is known.
- Check `index_status` if searches return empty or stale results.

## Configuration

Copy `config.example.toml` → `fva.toml` or `.fva.toml` (project root) and/or `~/.config/fva/config.toml` (global). Project overrides global.

Key settings:

```toml
[embedding]
provider = "local"    # or "voyage" with VOYAGE_API_KEY

[query]
fff_weight = 0.3
vector_weight = 0.5
graph_weight = 0.2
max_context_tokens = 8000
```

CLI flags override config: `--path`, `--config`, `RUST_LOG`.

## Further Reference

See [references/mcp-tools.md](references/mcp-tools.md) for parameter details and example prompts.
