//! FVA MCP server — full tool set for AI coding agents.

use std::sync::Arc;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData, ServerHandler, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;

use crate::engine::FvaEngine;
use crate::indexer::chunker::{ChunkSearchResult, format_chunks_for_agent};
use crate::query::context::ContextBuilder;
use crate::util::resolve_pagination;

pub const MCP_INSTRUCTIONS: &str = concat!(
    "FVA (FFF · Vector · AST) is a hybrid codebase intelligence engine.\n",
    "\n",
    "## Tools (use in this order)\n",
    "\n",
    "1. **hybrid_search** — BEST default. Combines file search + semantic + call graph.\n",
    "2. **semantic_search** — Natural language / concept search via embeddings.\n",
    "3. **grep** — Exact identifier search in file contents.\n",
    "4. **find_files** — Fuzzy file path search.\n",
    "5. **get_symbol_info** / **get_chunks** — Full function/class bodies (AST-aware).\n",
    "6. **get_call_graph** — Callers and callees of a symbol.\n",
    "7. **get_smart_context** — Token-efficient combined context for a task.\n",
    "8. **index_status** — Check indexing progress.\n",
    "\n",
    "## Rules\n",
    "\n",
    "- Prefer **hybrid_search** or **get_smart_context** over repeated grep+read cycles.\n",
    "- Grep bare identifiers only: 'MyHandler', not 'fn MyHandler'.\n",
    "- AST chunks preserve syntactic integrity — use instead of raw file reads.\n",
);

fn empty_result(msg: String) -> CallToolResult {
    CallToolResult::success(vec![Content::text(msg)])
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindFilesParams {
    #[serde(alias = "pattern")]
    pub query: String,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
    pub offset: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GrepParams {
    #[serde(alias = "pattern")]
    pub query: String,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
    pub offset: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetChunksParams {
    pub path: Option<String>,
    pub query: Option<String>,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
    pub offset: Option<f64>,
    #[serde(rename = "includeContent")]
    pub include_content: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSymbolInfoParams {
    pub symbol: String,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SemanticSearchParams {
    /// Natural language query describing what you're looking for.
    pub query: String,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HybridSearchParams {
    /// Search query (natural language or identifier).
    pub query: String,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
    /// Optional file path constraint.
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetCallGraphParams {
    /// Function/symbol name.
    pub function: String,
    /// Traversal depth (default 1).
    pub depth: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSmartContextParams {
    /// Task description or search query.
    pub query: String,
    /// Optional file path hint.
    pub path: Option<String>,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
}

#[derive(Clone)]
pub struct FvaServer {
    engine: Arc<FvaEngine>,
    default_max_results: usize,
}

impl FvaServer {
    pub fn new(engine: Arc<FvaEngine>) -> Self {
        let default_max_results = engine.config.query.default_max_results;
        Self {
            engine,
            default_max_results,
        }
    }
}

#[tool_router]
impl FvaServer {
    #[tool(
        name = "find_files",
        description = "Fuzzy file search by path/name. Frecency-ranked, git-aware. Use when exploring which files exist."
    )]
    fn find_files(
        &self,
        Parameters(params): Parameters<FindFilesParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, offset) =
            resolve_pagination(params.max_results, params.offset, self.default_max_results);

        let result = self
            .engine
            .fff
            .find_files(&params.query, offset, limit)
            .map_err(|e| ErrorData::internal_error(format!("find_files failed: {e}"), None))?;

        if result.paths.is_empty() {
            return Ok(empty_result(format!(
                "0 results ({} files indexed by FFF)",
                self.engine.fff.total_files()
            )));
        }

        let mut lines = vec![format!(
            "{}/{} matches",
            result.paths.len().min(limit),
            result.total_matched
        )];
        for path in &result.paths {
            lines.push(path.clone());
        }
        let next_offset = offset + result.paths.len();
        if next_offset < result.total_matched {
            lines.push(format!("offset: {next_offset}"));
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        name = "grep",
        description = "Search file contents for bare identifiers. FFF-powered with definition expansion and fuzzy fallback."
    )]
    fn grep(
        &self,
        Parameters(params): Parameters<GrepParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, offset) =
            resolve_pagination(params.max_results, params.offset, self.default_max_results);

        let result = self
            .engine
            .fff
            .grep(&params.query, offset, limit)
            .map_err(|e| ErrorData::internal_error(format!("grep failed: {e}"), None))?;

        if result.matches.is_empty() {
            return Ok(empty_result("0 matches.".to_string()));
        }

        let mut lines = vec![format!("{} matches", result.matches.len())];
        let mut current_file = String::new();
        for m in result.matches.iter().take(limit) {
            if m.file != current_file {
                current_file = m.file.clone();
                lines.push(format!("\n{}:", m.file));
            }
            lines.push(format!("  {}: {}", m.line_number, m.content));
        }
        if result.next_file_offset > offset {
            lines.push(format!("\noffset: {}", result.next_file_offset));
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        name = "get_chunks",
        description = "Get AST-aware code chunks (functions, classes, methods). Provide 'path' or 'query'."
    )]
    fn get_chunks(
        &self,
        Parameters(params): Parameters<GetChunksParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, offset) =
            resolve_pagination(params.max_results, params.offset, self.default_max_results);
        let include_content = params.include_content.unwrap_or(true);
        let store = self.engine.indexer.store();

        let chunks = if let Some(path) = &params.path {
            store.chunks_for_file(path)
        } else if let Some(query) = &params.query {
            store.search_chunks(query)
        } else {
            return Ok(CallToolResult::success(vec![Content::text(
                "Provide 'path' or 'query' parameter.".to_string(),
            )]));
        };

        let result = ChunkSearchResult::paginate(chunks, offset, limit);
        if result.chunks.is_empty() {
            return Ok(empty_result(format!(
                "0 chunks found. Stats: {:?}",
                store.stats()
            )));
        }

        let mut text = format_chunks_for_agent(&result.chunks, include_content);
        if result.has_more {
            text.push_str(&format!(
                "\n---\noffset: {}",
                result.offset + result.chunks.len()
            ));
        }
        Ok(CallToolResult::success(vec![Content::text(text)]))
    }

    #[tool(
        name = "get_symbol_info",
        description = "Look up a symbol by name. Returns full AST chunks with source code."
    )]
    fn get_symbol_info(
        &self,
        Parameters(params): Parameters<GetSymbolInfoParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, _offset) =
            resolve_pagination(params.max_results, None, self.default_max_results);
        let chunks = self.engine.indexer.store().find_symbol(&params.symbol);

        if chunks.is_empty() {
            return Ok(empty_result(format!(
                "Symbol '{}' not found.",
                params.symbol
            )));
        }

        let result = ChunkSearchResult::paginate(chunks, 0, limit);
        Ok(CallToolResult::success(vec![Content::text(
            format_chunks_for_agent(&result.chunks, true),
        )]))
    }

    #[tool(
        name = "semantic_search",
        description = "Natural language semantic search over code chunks using embeddings. Best for conceptual queries like 'authentication logic' or 'error handling patterns'."
    )]
    fn semantic_search(
        &self,
        Parameters(params): Parameters<SemanticSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, _offset) =
            resolve_pagination(params.max_results, None, self.default_max_results);
        let result = self.engine.query.semantic_search(&params.query, limit);
        Ok(CallToolResult::success(vec![Content::text(
            format_hybrid_result(&result),
        )]))
    }

    #[tool(
        name = "hybrid_search",
        description = "BEST default search. Combines FFF file search + vector semantic search + call graph traversal. Use for any codebase exploration task."
    )]
    fn hybrid_search(
        &self,
        Parameters(params): Parameters<HybridSearchParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, _offset) =
            resolve_pagination(params.max_results, None, self.default_max_results);
        let mut result = self.engine.query.hybrid_search(&params.query, limit);

        if let Some(path) = &params.path {
            result.hits.retain(|h| h.relative_path.contains(path));
        }

        Ok(CallToolResult::success(vec![Content::text(
            format_hybrid_result(&result),
        )]))
    }

    #[tool(
        name = "get_call_graph",
        description = "Get callers and callees of a function/symbol. Shows dependency relationships."
    )]
    fn get_call_graph(
        &self,
        Parameters(params): Parameters<GetCallGraphParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let depth = params.depth.map(|d| d.max(1.0) as usize).unwrap_or(1);
        let callers = self.engine.graph.callers(&params.function, depth);
        let callees = self.engine.graph.callees(&params.function, depth);

        let mut lines = vec![format!(
            "Call graph for `{}` (depth={depth}):",
            params.function
        )];

        lines.push("\n## Callers".into());
        if callers.is_empty() {
            lines.push("  (none found)".into());
        } else {
            for c in &callers {
                lines.push(format!("  {} in {}:{}", c.name, c.file, c.line));
            }
        }

        lines.push("\n## Callees".into());
        if callees.is_empty() {
            lines.push("  (none found)".into());
        } else {
            for c in &callees {
                let loc = if c.file.is_empty() {
                    String::new()
                } else {
                    format!(" in {}:{}", c.file, c.line)
                };
                lines.push(format!("  {}{}", c.name, loc));
            }
        }

        Ok(CallToolResult::success(vec![Content::text(
            lines.join("\n"),
        )]))
    }

    #[tool(
        name = "get_smart_context",
        description = "Build token-efficient smart context for a task. Combines hybrid search results + call graph + file context. Best for understanding code before making changes."
    )]
    fn get_smart_context(
        &self,
        Parameters(params): Parameters<GetSmartContextParams>,
    ) -> Result<CallToolResult, ErrorData> {
        let (limit, _offset) =
            resolve_pagination(params.max_results, None, self.default_max_results);
        let search = self.engine.query.hybrid_search(&params.query, limit);
        let ctx = self
            .engine
            .context
            .build(&params.query, params.path.as_deref(), &search);
        Ok(CallToolResult::success(vec![Content::text(
            ContextBuilder::format_context(&ctx),
        )]))
    }

    #[tool(
        name = "index_status",
        description = "Check FVA indexing status: FFF, AST chunks, vectors, call graph."
    )]
    fn index_status(&self) -> Result<CallToolResult, ErrorData> {
        let stats = self.engine.indexer.stats();
        let vectors = self.engine.vectors.stats();
        let graph = self.engine.graph.stats();

        let status = serde_json::json!({
            "fff": {
                "total_files": self.engine.fff.total_files(),
                "scanning": self.engine.fff.is_scanning(),
                "base_path": self.engine.fff.base_path(),
            },
            "ast_index": {
                "scanning": self.engine.indexer.is_scanning(),
                "indexed_files": stats.indexed_files,
                "total_chunks": stats.total_chunks,
                "total_symbols": stats.total_symbols,
            },
            "vectors": {
                "total": vectors.total_vectors,
                "dimensions": vectors.dimensions,
                "embedder": self.engine.embedder.name(),
            },
            "call_graph": {
                "nodes": graph.nodes,
                "edges": graph.edges,
            },
            "phase": "4 — Full hybrid intelligence",
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&status).unwrap_or_default(),
        )]))
    }
}

fn format_hybrid_result(result: &crate::query::HybridSearchResult) -> String {
    if result.hits.is_empty() {
        return format!("0 results for '{}'", result.query);
    }

    let mut lines = vec![format!(
        "{} results for '{}' ({} candidates)",
        result.hits.len(),
        result.query,
        result.total_candidates
    )];

    for hit in &result.hits {
        lines.push(format!(
            "\n### {} [{}] {}:{}-{} (score={:.3}, src={})",
            hit.symbol_name,
            hit.symbol_kind,
            hit.relative_path,
            hit.start_line,
            hit.end_line,
            hit.score,
            hit.sources.join("+")
        ));
        let preview: String = hit.content.lines().take(8).collect::<Vec<_>>().join("\n");
        lines.push(format!("```{}  \n{}\n```", hit.language, preview));
    }

    lines.join("\n")
}

#[tool_handler]
impl ServerHandler for FvaServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(MCP_INSTRUCTIONS)
    }
}
