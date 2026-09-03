//! Tantivy-backed BM25 lexical signal (hybrid search Stage 1c).

use std::sync::Mutex;

use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::{Field, STORED, STRING, Schema, TEXT, Value};
use tantivy::{Index, IndexReader, IndexWriter, ReloadPolicy, Term};

use crate::error::{FvaError, Result};
use crate::indexer::chunker::CodeChunk;

/// In-memory BM25 index over code chunks.
///
/// Schema: `chunk_id` (STRING|STORED, primary id for deletes),
/// `path` (STRING|STORED, exact path for per-file deletes),
/// `content` + `symbol` (TEXT, BM25-scored).
pub struct Bm25Index {
    index: Index,
    reader: IndexReader,
    writer: Mutex<IndexWriter>,
    chunk_id: Field,
    path: Field,
    content: Field,
    symbol: Field,
}

// ponytail: per-file commit (O(segments) growth on big repos); switch to
// batched commits or merge policy tuning if indexing throughput matters.
impl Bm25Index {
    pub fn new() -> Result<Self> {
        let mut builder = Schema::builder();
        let chunk_id = builder.add_text_field("chunk_id", STRING | STORED);
        let path = builder.add_text_field("path", STRING | STORED);
        let content = builder.add_text_field("content", TEXT);
        let symbol = builder.add_text_field("symbol", TEXT);
        let schema = builder.build();

        let index = Index::create_in_ram(schema);
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(|e| FvaError::Indexer(format!("bm25 reader: {e}")))?;
        let writer = index
            .writer(15_000_000)
            .map_err(|e| FvaError::Indexer(format!("bm25 writer: {e}")))?;

        Ok(Self {
            index,
            reader,
            writer: Mutex::new(writer),
            chunk_id,
            path,
            content,
            symbol,
        })
    }

    /// Replace all docs for `relative_path` with `chunks`.
    pub fn upsert_file(&self, relative_path: &str, chunks: &[CodeChunk]) -> Result<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| FvaError::Indexer(format!("bm25 lock: {e}")))?;
        writer.delete_term(Term::from_field_text(self.path, relative_path));
        for chunk in chunks {
            let mut doc = tantivy::TantivyDocument::default();
            doc.add_text(self.chunk_id, &chunk.id);
            doc.add_text(self.path, relative_path);
            doc.add_text(self.content, &chunk.content);
            doc.add_text(self.symbol, &chunk.symbol_name);
            writer
                .add_document(doc)
                .map_err(|e| FvaError::Indexer(format!("bm25 add: {e}")))?;
        }
        writer
            .commit()
            .map_err(|e| FvaError::Indexer(format!("bm25 commit: {e}")))?;
        self.reader
            .reload()
            .map_err(|e| FvaError::Indexer(format!("bm25 reload: {e}")))?;
        Ok(())
    }

    /// BM25-scored `(chunk_id, score)` pairs. Returns empty on unparsable queries.
    pub fn search(&self, query: &str, limit: usize) -> Vec<(String, f32)> {
        if query.trim().is_empty() || limit == 0 {
            return Vec::new();
        }
        let parser = QueryParser::for_index(&self.index, vec![self.content, self.symbol]);
        let parsed = match parser.parse_query(query) {
            Ok(q) => q,
            Err(_) => return Vec::new(),
        };
        let searcher = self.reader.searcher();
        let top = match searcher.search(&parsed, &TopDocs::with_limit(limit).order_by_score()) {
            Ok(t) => t,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let Ok(doc): std::result::Result<tantivy::TantivyDocument, _> = searcher.doc(addr)
            else {
                continue;
            };
            let Some(id) = doc.get_first(self.chunk_id).and_then(|v| v.as_str()) else {
                continue;
            };
            out.push((id.to_string(), score));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::chunker::CodeChunk;

    fn chunk(id: &str, symbol: &str, content: &str) -> CodeChunk {
        CodeChunk {
            id: id.to_string(),
            relative_path: "src/a.rs".to_string(),
            symbol_name: symbol.to_string(),
            symbol_kind: "function".to_string(),
            language: "rust".to_string(),
            start_line: 1,
            end_line: 3,
            content: content.to_string(),
            line_count: 3,
        }
    }

    #[test]
    fn exact_match_outranks_partial() {
        let index = Bm25Index::new().expect("bm25");
        index
            .upsert_file(
                "src/a.rs",
                &[
                    chunk(
                        "1",
                        "authenticate_user",
                        "fn authenticate_user validates jwt token",
                    ),
                    chunk("2", "helper", "fn helper does miscellaneous work here"),
                ],
            )
            .expect("upsert");
        let hits = index.search("authenticate_user", 10);
        assert!(!hits.is_empty(), "expected bm25 hits");
        assert_eq!(hits[0].0, "1", "exact match should rank first");
    }

    #[test]
    fn upsert_replaces_old_file_docs() {
        let index = Bm25Index::new().expect("bm25");
        index
            .upsert_file(
                "src/a.rs",
                &[chunk("1", "old_symbol", "old unique content xyz")],
            )
            .expect("upsert");
        assert!(!index.search("old_symbol", 10).is_empty());
        index
            .upsert_file(
                "src/a.rs",
                &[chunk("2", "new_symbol", "new unique content abc")],
            )
            .expect("re-upsert");
        assert!(
            index.search("old_symbol", 10).is_empty(),
            "old docs must be gone after re-upsert"
        );
        assert!(!index.search("new_symbol", 10).is_empty());
    }

    #[test]
    fn unparsable_query_returns_empty() {
        let index = Bm25Index::new().expect("bm25");
        index
            .upsert_file("src/a.rs", &[chunk("1", "foo", "bar")])
            .expect("upsert");
        assert!(index.search("(((", 10).is_empty());
        assert!(index.search("", 10).is_empty());
        assert!(index.search("foo", 0).is_empty());
    }
}
