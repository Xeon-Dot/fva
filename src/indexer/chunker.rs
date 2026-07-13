//! AST-aware chunking with content-hash based incremental indexing.

use std::path::Path;

use blake3::Hash;
use serde::{Deserialize, Serialize};

use super::parser::{AstParser, RawChunk};
use crate::error::Result;
use crate::util;

/// A semantic code chunk with full metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    pub id: String,
    pub file_path: String,
    pub relative_path: String,
    pub language: String,
    pub symbol_name: String,
    pub symbol_kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub content_hash: String,
    pub line_count: usize,
}

impl CodeChunk {
    pub fn from_raw(
        raw: RawChunk,
        file_path: &Path,
        relative_path: &str,
        language: &str,
        content_hash: &Hash,
    ) -> Self {
        let line_count = raw.content.lines().count();
        let id = format!(
            "{}:{}:{}-{}",
            relative_path, raw.name, raw.start_line, raw.end_line
        );

        Self {
            id,
            file_path: file_path.to_string_lossy().to_string(),
            relative_path: relative_path.to_string(),
            language: language.to_string(),
            symbol_name: raw.name,
            symbol_kind: raw.kind,
            start_line: raw.start_line,
            end_line: raw.end_line,
            content: raw.content,
            content_hash: content_hash.to_hex().to_string(),
            line_count,
        }
    }

    /// Rough token estimate (4 chars ≈ 1 token).
    pub fn estimated_tokens(&self) -> usize {
        util::estimate_tokens(&self.content)
    }
}

/// Chunk a single source file using Tree-sitter.
pub fn chunk_file(
    parser: &AstParser,
    file_path: &Path,
    relative_path: &str,
    source: &str,
    max_file_size: u64,
) -> Result<Vec<CodeChunk>> {
    if source.len() as u64 > max_file_size {
        return Ok(vec![]);
    }

    let content_hash = blake3::hash(source.as_bytes());
    let Some(language) = AstParser::detect_language(file_path, Some(source)) else {
        return Ok(vec![]);
    };

    let raw_chunks = parser.extract_chunks(&language, source);

    Ok(raw_chunks
        .into_iter()
        .map(|raw| CodeChunk::from_raw(raw, file_path, relative_path, &language, &content_hash))
        .collect())
}

/// Chunk search result with pagination metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSearchResult {
    pub chunks: Vec<CodeChunk>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    pub has_more: bool,
}

impl ChunkSearchResult {
    pub fn paginate(chunks: Vec<CodeChunk>, offset: usize, limit: usize) -> Self {
        let total = chunks.len();
        let page: Vec<CodeChunk> = chunks.into_iter().skip(offset).take(limit).collect();
        let has_more = offset + page.len() < total;

        Self {
            chunks: page,
            total,
            offset,
            limit,
            has_more,
        }
    }
}

/// Format chunks for MCP/agent consumption (token-efficient).
pub fn format_chunks_for_agent(chunks: &[CodeChunk], include_content: bool) -> String {
    let mut lines = Vec::new();

    for chunk in chunks {
        let header = format!(
            "## {} [{}] {}:{}-{} ({} lines, ~{} tokens)",
            chunk.symbol_name,
            chunk.symbol_kind,
            chunk.relative_path,
            chunk.start_line,
            chunk.end_line,
            chunk.line_count,
            chunk.estimated_tokens()
        );
        lines.push(header);

        if include_content {
            lines.push(format!("```{}\n{}\n```", chunk.language, chunk.content));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn chunks_rust_function() {
        let source = r#"
fn hello_world() {
    println!("hello");
}

struct MyStruct {
    field: i32,
}
"#;
        let parser = AstParser::new();
        let path = PathBuf::from("test.rs");
        let chunks = chunk_file(&parser, &path, "test.rs", source, 1_000_000).unwrap();
        assert!(!chunks.is_empty());
        assert!(chunks.iter().any(|c| c.symbol_name == "hello_world"));
    }
}
