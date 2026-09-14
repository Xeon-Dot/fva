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

| Parameter    | Type   | Required | Notes                            |
| ------------ | ------ | -------- | -------------------------------- |
| `query`      | string | yes      | Task description or question     |
| `path`       | string | no       | File hint (when target is known) |
| `maxResults` | number | no       | Default 20                       |

## semantic_search — Conceptual search

Pure embedding search over AST chunks. Best when keyword search fails and you need concepts like "auth middleware" or "retry logic".

| Parameter    | Type   | Required | Notes                    |
| ------------ | ------ | -------- | ------------------------ |
| `query`      | string | yes      | Natural language concept |
| `maxResults` | number | no       | Default 20               |

Uses the configured embedder (`local` hash by default, or `voyage` when set).

## get_symbol_info — Exact symbol lookup

Look up a symbol by exact name; returns full AST chunks with source code (functions, structs, classes, methods).

| Parameter    | Type   | Required | Notes       |
| ------------ | ------ | -------- | ----------- |
| `symbol`     | string | yes      | Symbol name |
| `maxResults` | number | no       | Default 20  |

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

| Parameter  | Type   | Required | Notes       |
| ---------- | ------ | -------- | ----------- |
| `function` | string | yes      | Symbol name |
| `depth`    | number | no       | Default 1   |

## grep — Content search

Bare-identifier search in file contents. FFF-powered with definition expansion and fuzzy fallback. **Use bare identifiers only** (e.g. `MyHandler`, not `fn MyHandler`).

| Parameter    | Type   | Required               | Notes             |
| ------------ | ------ | ---------------------- | ----------------- |
| `query`      | string | yes (alias: `pattern`) | Bare identifier   |
| `maxResults` | number | no                     | Default 20        |
| `offset`     | number | no                     | Pagination offset |

## find_files — Fuzzy file discovery

Fuzzy path/name search, frecency-ranked and git-aware (respects `.gitignore`). Use to discover which files exist.

| Parameter    | Type   | Required               | Notes             |
| ------------ | ------ | ---------------------- | ----------------- |
| `query`      | string | yes (alias: `pattern`) | Partial path/name |
| `maxResults` | number | no                     | Default 20        |
| `offset`     | number | no                     | Pagination offset |

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

| Parameter    | Type   | Required | Notes                                                                      |
| ------------ | ------ | -------- | -------------------------------------------------------------------------- |
| `slug`       | string | yes      | Unique id / filename (no extension)                                        |
| `title`      | string | yes      | Human-readable title                                                       |
| `content`    | string | yes      | Markdown body                                                              |
| `tags`       | string | no       | Comma-separated tags                                                       |
| `entry_type` | string | no       | `source\|entity\|concept\|analysis\|adr\|arch\|gotcha` (default `concept`) |
| `sources`    | string | no       | Comma-separated source paths/URLs                                          |

## wiki_read — Read one entry

Read a wiki entry by slug. Returns full Markdown content with metadata (title, tags, created/updated).

| Parameter | Type   | Required | Notes    |
| --------- | ------ | -------- | -------- |
| `slug`    | string | yes      | Entry id |

## wiki_delete — Delete an entry

Irreversible — removes the entry and its index.

| Parameter | Type   | Required | Notes    |
| --------- | ------ | -------- | -------- |
| `slug`    | string | yes      | Entry id |

## wiki_search — Recall knowledge

Semantic search over wiki entries with tag filtering. Use when the `wiki_query` bundle is not enough — `wiki_query` is the default recall path. Returns entries with previews and relevance scores.

| Parameter    | Type   | Required | Notes                                                  |
| ------------ | ------ | -------- | ------------------------------------------------------ |
| `query`      | string | yes      | Natural language query                                 |
| `tags`       | string | no       | Comma-separated tag filter                             |
| `entry_type` | string | no       | `source\|entity\|concept\|analysis\|adr\|arch\|gotcha` |
| `maxResults` | number | no       | Default 20                                             |

## wiki_list — Browse saved knowledge

List all wiki entries with slug, title, tags, and last-updated timestamp. Supports tag and type filtering.

| Parameter    | Type   | Required | Notes                                                  |
| ------------ | ------ | -------- | ------------------------------------------------------ |
| `tags`       | string | no       | Comma-separated tag filter                             |
| `entry_type` | string | no       | `source\|entity\|concept\|analysis\|adr\|arch\|gotcha` |

## wiki_query — Index-first recall (default)

Query the wiki index-first (Karpathy-style). Returns `index.md` plus top semantic hits plus 1-hop `[[slug]]` wikilink neighbors in one bundle. **Start every task here**, not `wiki_search`. Drill down with `wiki_read`; file good answers back with `wiki_write`.

| Parameter    | Type   | Required | Notes                  |
| ------------ | ------ | -------- | ---------------------- |
| `query`      | string | yes      | Natural language query |
| `maxResults` | number | no       | Default 20             |

CLI: `fva wiki query "<question>" --path .` (bundle limit fixed at 10 on CLI).

## wiki_ingest — Ingest a source

Ingest a source into the wiki (Karpathy-style). Saves raw text under `sources/` (immutable), finds related pages via semantic search + wikilink neighbors, and returns an ingest plan with up to 15 suggested pages to touch. Write the summary with `wiki_write` afterwards, then cross-link with `[[slug]]` references.

| Parameter    | Type   | Required | Notes                                                            |
| ------------ | ------ | -------- | ---------------------------------------------------------------- |
| `content`    | string | yes      | Raw source text to preserve under `sources/`                     |
| `source_uri` | string | no       | Original path or URL of the source                               |
| `title_hint` | string | no       | Hint for the source title                                        |
| `area_hint`  | string | no       | One of: `entity`, `concept`, `analysis`, `adr`, `arch`, `gotcha` |

CLI: `fva wiki ingest --content ... | --file <path> | stdin --title "..." --source-uri <path-or-url> --area-hint concept --path .`

Rules: never edit `sources/*`, never hand-edit `index`/`log`.

## wiki_lint — Hygiene check

Lint the wiki (Karpathy-style): orphans, dead `[[links]]`, stale entries, contradiction candidates, thin areas. Report only, no auto-fix — fix things yourself.

| Parameter    | Type   | Required | Notes                                                             |
| ------------ | ------ | -------- | ----------------------------------------------------------------- |
| `stale_days` | number | no       | Days after which a non-source entry counts as stale (default 180) |

CLI: `fva wiki lint --stale-days 180 --path .`

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
- wiki_query: index-first recall (default at task start) — drill down with wiki_read
- wiki_write: persist knowledge and file good answers back (entry_type: source|entity|concept|analysis|adr|arch|gotcha, default concept)
- wiki_ingest: ingest sources (sources/ immutable, then summarize via wiki_write)
- wiki_lint: periodic hygiene check (report only)
Prefer hybrid_search over repeated grep+read cycles.
Use wiki_query at task start; use wiki_write to save decisions, patterns, and learnings.
Check index_status if results are empty or stale.
```
