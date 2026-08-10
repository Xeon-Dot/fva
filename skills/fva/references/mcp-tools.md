# FVA MCP Tools Reference

> **Language:** Please ask questions in English.

All tools are served by the FVA MCP server (`fva --path <project-root>`). Prefer `hybrid_search` for open-ended questions and `get_smart_context` before making edits. See the `fva` skill for the full workflow and priority order.

## hybrid_search — Best default

Fuses FFF file search, vector semantic search, and call graph traversal (3-stage fusion). Use for "where is X?", "how does Y work?", or any open-ended exploration — stronger than any single signal.

| Parameter    | Type   | Required | Notes                                          |
| ------------ | ------ | -------- | ---------------------------------------------- |
| `query`      | string | yes      | Natural language or identifier                 |
| `maxResults` | number | no       | Default 20                                     |
| `path`       | string | no       | Filter hits to paths containing this substring |

## get_smart_context — Task context before edits

Token-budget context builder that combines hybrid search + call graph + file context into a compact, ranked answer. Call before editing code to understand what to change.

| Parameter    | Type   | Required       | Notes                          |
| ------------ | ------ | -------------- | ------------------------------ |
| `query`      | string | yes            | Task description or question   |
| `path`       | string | no             | File hint (when target is known) |
| `maxResults` | number | no             | Default 20                     |

## semantic_search — Conceptual search

Pure embedding search over AST chunks. Best when keyword search fails and you need concepts like "auth middleware" or "retry logic".

| Parameter    | Type   | Required | Notes                                     |
| ------------ | ------ | -------- | ----------------------------------------- |
| `query`      | string | yes      | Natural language concept                  |
| `maxResults` | number | no       | Default 20                                |

Uses the configured embedder (`local` hash by default, or `voyage` when set).

## get_symbol_info — Exact symbol lookup

Look up a symbol by exact name; returns full AST chunks with source code (functions, structs, classes, methods).

| Parameter    | Type   | Required | Notes        |
| ------------ | ------ | -------- | ------------ |
| `symbol`     | string | yes      | Symbol name  |
| `maxResults` | number | no       | Default 20   |

## get_chunks — Browse AST chunks

AST-aware code chunks (functions, classes, methods) with full source. Provide **`path` OR `query`** — at least one is required.

| Parameter        | Type    | Required          | Notes                    |
| ---------------- | ------- | ----------------- | ------------------------ |
| `path`           | string  | one of path/query | File path to browse      |
| `query`          | string  | one of path/query | Keyword to search chunks |
| `maxResults`     | number  | no                | Default 20               |
| `offset`         | number  | no                | Pagination offset        |
| `includeContent` | boolean | no                | Default `true`           |

## get_call_graph — Callers and callees

Show who calls a symbol and what it calls, with file locations and dependency edges. Supports multi-hop traversal.

| Parameter  | Type   | Required       | Notes          |
| ---------- | ------ | -------------- | -------------- |
| `function` | string | yes            | Symbol name    |
| `depth`    | number | no             | Default 1      |

## grep — Content search

Bare-identifier search in file contents. FFF-powered with definition expansion and fuzzy fallback. **Use bare identifiers only** (e.g. `MyHandler`, not `fn MyHandler`).

| Parameter    | Type   | Required               | Notes              |
| ------------ | ------ | ---------------------- | ------------------ |
| `query`      | string | yes (alias: `pattern`) | Bare identifier    |
| `maxResults` | number | no                     | Default 20         |
| `offset`     | number | no                     | Pagination offset  |

## find_files — Fuzzy file discovery

Fuzzy path/name search, frecency-ranked and git-aware (respects `.gitignore`). Use to discover which files exist.

| Parameter    | Type   | Required               | Notes              |
| ------------ | ------ | ---------------------- | ------------------ |
| `query`      | string | yes (alias: `pattern`) | Partial path/name  |
| `maxResults` | number | no                     | Default 20         |
| `offset`     | number | no                     | Pagination offset  |

## index_status — Health check

No parameters. Returns JSON with:

- **FFF**: `total_files`, `scanning`, `base_path`
- **AST**: `indexed_files`, `total_chunks`, `total_symbols`
- **Vectors**: `total`, `dimensions`, `embedder`
- **Call graph**: `nodes`, `edges`
- **Wiki**: `total_entries`

Call when searches are empty/stale or to confirm indexing progress.

## wiki_write — Save knowledge

Create or update a wiki knowledge entry. Persistent Markdown, auto-indexed for semantic search. Save anything useful — decisions, gotchas, conventions, patterns. Knowledge not saved is knowledge lost.

| Parameter | Type   | Required | Notes                              |
| --------- | ------ | -------- | ---------------------------------- |
| `slug`    | string | yes      | Unique id / filename (no extension)|
| `title`   | string | yes      | Human-readable title               |
| `content` | string | yes      | Markdown body                      |
| `tags`    | string | no       | Comma-separated tags               |

## wiki_read — Read one entry

Read a wiki entry by slug. Returns full Markdown content with metadata (title, tags, created/updated).

| Parameter | Type   | Required | Notes |
| --------- | ------ | -------- | ----- |
| `slug`    | string | yes      | Entry id |

## wiki_delete — Delete an entry

Irreversible — removes the entry and its index.

| Parameter | Type   | Required | Notes |
| --------- | ------ | -------- | ----- |
| `slug`    | string | yes      | Entry id |

## wiki_search — Recall knowledge

Semantic search over wiki entries with tag filtering. **Call at task start** to recall prior decisions, patterns, and gotchas. Returns entries with previews and relevance scores.

| Parameter    | Type   | Required | Notes                        |
| ------------ | ------ | -------- | ---------------------------- |
| `query`      | string | yes      | Natural language query       |
| `tags`       | string | no       | Comma-separated tag filter   |
| `maxResults` | number | no       | Default 20                   |

## wiki_list — Browse saved knowledge

List all wiki entries with slug, title, tags, and last-updated timestamp. Supports tag filtering.

| Parameter | Type   | Required | Notes                      |
| --------- | ------ | -------- | -------------------------- |
| `tags`    | string | no       | Comma-separated tag filter |

## Pagination

`find_files`, `grep`, `get_chunks`, `wiki_search`, and `wiki_list` support `maxResults` / `offset`. When output contains `offset: N`, pass `offset: N` on the next call.

## Recommended Agent Prompt

Copy into your agent's system prompt or project instructions:

```
For codebase exploration, use FVA MCP tools:
- hybrid_search: default — combines file search + semantic + call graph
- semantic_search: natural language concept search
- get_smart_context: token-efficient context before edits
- get_symbol_info / get_chunks: full function/class bodies (AST-aware)
- get_call_graph: callers and callees
- wiki_write / wiki_search: persist and recall knowledge across sessions
Prefer hybrid_search over repeated grep+read cycles.
Use wiki_write to save decisions, patterns, and learnings.
Check index_status if results are empty or stale.
```
