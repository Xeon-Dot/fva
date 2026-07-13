//! Smart context builder — token-efficient context for agents.

use serde::{Deserialize, Serialize};

use super::hybrid::{HybridHit, HybridSearchResult};
use crate::graph::CallGraphStore;
use crate::indexer::store::ChunkStore;
use crate::util;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartContext {
    pub query: String,
    pub path_hint: Option<String>,
    pub sections: Vec<ContextSection>,
    pub estimated_tokens: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSection {
    pub title: String,
    pub content: String,
    pub estimated_tokens: usize,
}

pub struct ContextBuilder {
    store: Arc<ChunkStore>,
    graph: Arc<CallGraphStore>,
    max_tokens: usize,
}

impl ContextBuilder {
    pub fn new(store: Arc<ChunkStore>, graph: Arc<CallGraphStore>, max_tokens: usize) -> Self {
        Self {
            store,
            graph,
            max_tokens,
        }
    }

    pub fn build(
        &self,
        query: &str,
        path_hint: Option<&str>,
        search_result: &HybridSearchResult,
    ) -> SmartContext {
        let mut sections = Vec::new();
        let mut used_tokens = 0usize;

        // Primary hits from hybrid search
        let mut primary = String::new();
        for hit in &search_result.hits {
            let section = format_hit(hit);
            let tokens = util::estimate_tokens(&section);
            if used_tokens + tokens > self.max_tokens {
                break;
            }
            primary.push_str(&section);
            primary.push('\n');
            used_tokens += tokens;
        }

        self.try_add_section(
            &mut sections,
            &mut used_tokens,
            "Primary matches".into(),
            primary,
        );

        // Graph context for top hit
        if let Some(top) = search_result.hits.first() {
            let callers = self.graph.callers(&top.symbol_name, 1);
            let callees = self.graph.callees(&top.symbol_name, 1);

            if !callers.is_empty() || !callees.is_empty() {
                let mut graph_text = format!("## Call graph for `{}`\n", top.symbol_name);
                if !callers.is_empty() {
                    graph_text.push_str("**Callers:** ");
                    graph_text.push_str(
                        &callers
                            .iter()
                            .map(|s| format!("{} ({})", s.name, s.file))
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    graph_text.push('\n');
                }
                if !callees.is_empty() {
                    graph_text.push_str("**Callees:** ");
                    graph_text.push_str(
                        &callees
                            .iter()
                            .map(|s| s.name.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                    graph_text.push('\n');
                }

                self.try_add_section(
                    &mut sections,
                    &mut used_tokens,
                    "Call graph".into(),
                    graph_text,
                );
            }
        }

        // Path-specific chunks
        if let Some(path) = path_hint {
            let chunks = self.store.chunks_for_file(path);
            if !chunks.is_empty() {
                let mut text = format!("## File: {path}\n");
                for chunk in chunks.iter().take(5) {
                    text.push_str(&format!(
                        "### {} [{}] L{}-{}\n```{}\n{}\n```\n",
                        chunk.symbol_name,
                        chunk.symbol_kind,
                        chunk.start_line,
                        chunk.end_line,
                        chunk.language,
                        chunk.content
                    ));
                }
                self.try_add_section(
                    &mut sections,
                    &mut used_tokens,
                    format!("File context: {path}"),
                    text,
                );
            }
        }

        let truncated = used_tokens >= self.max_tokens;

        SmartContext {
            query: query.to_string(),
            path_hint: path_hint.map(String::from),
            sections,
            estimated_tokens: used_tokens,
            truncated,
        }
    }

    fn try_add_section(
        &self,
        sections: &mut Vec<ContextSection>,
        used_tokens: &mut usize,
        title: String,
        content: String,
    ) {
        let tokens = util::estimate_tokens(&content);
        if *used_tokens + tokens <= self.max_tokens {
            *used_tokens += tokens;
            sections.push(ContextSection {
                title,
                content,
                estimated_tokens: tokens,
            });
        }
    }

    pub fn format_context(ctx: &SmartContext) -> String {
        let mut out = format!("# Smart Context: {}\n", ctx.query);
        if let Some(path) = &ctx.path_hint {
            out.push_str(&format!("Path hint: {path}\n"));
        }
        out.push_str(&format!(
            "Estimated tokens: {} {}\n\n",
            ctx.estimated_tokens,
            if ctx.truncated { "(truncated)" } else { "" }
        ));

        for section in &ctx.sections {
            out.push_str(&format!("## {}\n", section.title));
            out.push_str(&section.content);
            out.push('\n');
        }

        out
    }
}

fn format_hit(hit: &HybridHit) -> String {
    format!(
        "### {} [{}] {}:{}-{} (score={:.3}, sources={})\n```{}\n{}\n```\n",
        hit.symbol_name,
        hit.symbol_kind,
        hit.relative_path,
        hit.start_line,
        hit.end_line,
        hit.score,
        hit.sources.join("+"),
        hit.language,
        hit.content
    )
}
