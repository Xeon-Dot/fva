//! Extract call graph edges from code chunks.

use std::collections::HashSet;
use std::sync::OnceLock;

use regex::Regex;

use super::{GraphEdge, SymbolId};
use crate::indexer::chunker::CodeChunk;

fn call_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap())
}

/// Extract call edges from a set of chunks.
pub fn extract_edges(chunks: &[CodeChunk]) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    let defined_symbols: HashSet<String> = chunks
        .iter()
        .map(|c| c.symbol_name.to_lowercase())
        .collect();

    let keywords: HashSet<&str> = [
        "if", "for", "while", "match", "return", "let", "mut", "pub", "fn", "struct", "enum",
        "impl", "trait", "mod", "use", "self", "Self", "super", "crate", "loop", "async", "await",
        "move", "ref", "unsafe", "true", "false",
    ]
    .into_iter()
    .collect();

    for chunk in chunks {
        let caller = SymbolId {
            name: chunk.symbol_name.clone(),
            file: chunk.relative_path.clone(),
            line: chunk.start_line,
        };

        for cap in call_re().captures_iter(&chunk.content) {
            if let Some(name) = cap.get(1) {
                let callee = name.as_str();
                if callee.len() < 2 || keywords.contains(callee) {
                    continue;
                }
                if callee == chunk.symbol_name {
                    continue;
                }
                edges.push(GraphEdge {
                    caller: caller.clone(),
                    callee: callee.to_string(),
                    file: chunk.relative_path.clone(),
                    line: chunk.start_line,
                });
            }
        }

        // Boost edges to symbols defined in the same file
        for other in &defined_symbols {
            if other != &chunk.symbol_name.to_lowercase()
                && chunk.content.to_lowercase().contains(other)
            {
                edges.push(GraphEdge {
                    caller: caller.clone(),
                    callee: other.clone(),
                    file: chunk.relative_path.clone(),
                    line: chunk.start_line,
                });
            }
        }
    }

    edges
}
