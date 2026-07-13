//! Tree-sitter based multi-language AST parser.
//!
//! Supports all 306+ tree-sitter grammars via [`tree_sitter_language_pack`].

use tree_sitter_language_pack::{
    ProcessConfig, StructureItem, StructureKind, detect_language_from_content,
    detect_language_from_path, process,
};

/// Multi-language Tree-sitter parser backed by the global language-pack registry.
pub struct AstParser;

impl AstParser {
    pub fn new() -> Self {
        Self
    }

    /// Detect tree-sitter language from file path and optional source (shebang).
    pub fn detect_language(path: &std::path::Path, source: Option<&str>) -> Option<String> {
        let path_str = path.to_string_lossy();
        detect_language_from_path(&path_str)
            .map(str::to_string)
            .or_else(|| {
                source.and_then(|content| detect_language_from_content(content).map(str::to_string))
            })
    }

    /// Extract semantic chunks from source using tree-sitter structure analysis.
    pub fn extract_chunks(&self, language: &str, source: &str) -> Vec<RawChunk> {
        let config = ProcessConfig::new(language);
        match process(source, &config) {
            Ok(result) => {
                let mut chunks = Vec::new();
                collect_structure_chunks(&result.structure, source, &mut chunks);
                if chunks.is_empty() {
                    fallback_chunks(source)
                } else {
                    chunks.sort_by_key(|c| c.start_line);
                    chunks
                }
            }
            Err(_) => fallback_chunks(source),
        }
    }
}

impl Default for AstParser {
    fn default() -> Self {
        Self::new()
    }
}

/// Raw chunk extracted from AST before metadata enrichment.
#[derive(Debug, Clone)]
pub struct RawChunk {
    pub name: String,
    pub kind: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub content: String,
}

fn collect_structure_chunks(items: &[StructureItem], source: &str, out: &mut Vec<RawChunk>) {
    for item in items {
        push_structure_chunk(item, source, out);
        collect_structure_chunks(&item.children, source, out);
    }
}

fn push_structure_chunk(item: &StructureItem, source: &str, out: &mut Vec<RawChunk>) {
    let span = &item.span;
    if span.end_byte <= span.start_byte || span.end_byte > source.len() {
        return;
    }

    let kind = structure_kind_str(&item.kind);
    let start_line = span.start_line + 1;
    let end_line = span.end_line + 1;
    let name = item
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| format!("{kind}@L{start_line}"));

    out.push(RawChunk {
        name,
        kind,
        start_line,
        end_line,
        start_byte: span.start_byte,
        end_byte: span.end_byte,
        content: source[span.start_byte..span.end_byte].to_string(),
    });
}

fn structure_kind_str(kind: &StructureKind) -> String {
    match kind {
        StructureKind::Function => "function".into(),
        StructureKind::Method => "method".into(),
        StructureKind::Class => "class".into(),
        StructureKind::Struct => "struct".into(),
        StructureKind::Interface => "interface".into(),
        StructureKind::Enum => "enum".into(),
        StructureKind::Module => "module".into(),
        StructureKind::Trait => "trait".into(),
        StructureKind::Impl => "impl".into(),
        StructureKind::Namespace => "namespace".into(),
        StructureKind::Other(name) => name.clone(),
    }
}

/// Fallback: split file into line-based chunks when AST extraction fails.
fn fallback_chunks(source: &str) -> Vec<RawChunk> {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return vec![];
    }

    const CHUNK_LINES: usize = 80;
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let content = lines[start..end].join("\n");
        chunks.push(RawChunk {
            name: format!("block@L{}", start + 1),
            kind: "block".to_string(),
            start_line: start + 1,
            end_line: end,
            start_byte: 0,
            end_byte: content.len(),
            content,
        });
        start = end;
    }

    chunks
}

/// Return whether a file path maps to a known tree-sitter language.
pub fn is_indexable(path: &std::path::Path) -> bool {
    AstParser::detect_language(path, None).is_some()
}

/// Number of languages available in the tree-sitter language pack manifest.
pub fn supported_language_count() -> usize {
    tree_sitter_language_pack::language_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_extensions() {
        assert_eq!(
            AstParser::detect_language(std::path::Path::new("main.rs"), None),
            Some("rust".into())
        );
        assert_eq!(
            AstParser::detect_language(std::path::Path::new("app.py"), None),
            Some("python".into())
        );
        assert_eq!(
            AstParser::detect_language(std::path::Path::new("index.tsx"), None),
            Some("tsx".into())
        );
        assert_eq!(
            AstParser::detect_language(std::path::Path::new("README.md"), None),
            Some("markdown".into())
        );
    }

    #[test]
    fn detects_shebang_language() {
        let source = "#!/usr/bin/env python3\nprint('hi')";
        assert_eq!(
            AstParser::detect_language(std::path::Path::new("script"), Some(source)),
            Some("python".into())
        );
    }

    #[test]
    fn extracts_rust_function_chunks() {
        let source = r#"
fn hello_world() {
    println!("hello");
}

struct MyStruct {
    field: i32,
}
"#;
        let parser = AstParser::new();
        let chunks = parser.extract_chunks("rust", source);
        assert!(!chunks.is_empty());
        assert!(chunks.iter().any(|c| c.name == "hello_world"));
    }
}
