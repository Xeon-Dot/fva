//! Wiki knowledge base for AI coding agents.
//!
//! Markdown files with YAML-like frontmatter in `.fva/wiki/`,
//! backed by a separate vector index for semantic search.

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::embedding::{Embedder, cosine_similarity};
use crate::error::{FvaError, Result};

#[derive(Debug, Clone)]
pub struct WikiEntry {
    pub slug: String,
    pub title: String,
    pub tags: Vec<String>,
    pub created: String,
    pub updated: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WikiVector {
    slug: String,
    title: String,
    tags: Vec<String>,
    content_preview: String,
    vector: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct WikiSnapshot {
    entries: Vec<WikiVector>,
}

#[derive(Debug, Clone, Default)]
pub struct WikiStats {
    pub total_entries: usize,
}

pub struct WikiStore {
    wiki_dir: PathBuf,
    embedder: Arc<dyn Embedder>,
    entries: RwLock<Vec<WikiVector>>,
    persist_path: PathBuf,
}

impl WikiStore {
    pub fn open(wiki_dir: PathBuf, embedder: Arc<dyn Embedder>) -> Result<Self> {
        std::fs::create_dir_all(&wiki_dir)?;

        let persist_path = wiki_dir
            .parent()
            .unwrap_or(&wiki_dir)
            .join("wiki_vectors.bin");

        let store = Self {
            wiki_dir,
            embedder,
            entries: RwLock::new(Vec::new()),
            persist_path,
        };

        if store.persist_path.exists()
            && let Ok(bytes) = std::fs::read(&store.persist_path)
            && let Ok(snapshot) = bincode::deserialize::<WikiSnapshot>(&bytes)
        {
            *store.entries.write() = snapshot.entries;
        }

        store.reconcile()?;

        Ok(store)
    }

    /// Sync vector index with .md files on disk.
    /// ponytail: O(n) scan on open; fine for wiki-scale data (<1k entries).
    fn reconcile(&self) -> Result<()> {
        let mut entries = self.entries.write();
        let indexed: std::collections::HashSet<String> =
            entries.iter().map(|e| e.slug.clone()).collect();

        let mut on_disk: std::collections::HashSet<String> = std::collections::HashSet::new();
        if let Ok(rd) = std::fs::read_dir(&self.wiki_dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md")
                    && let Some(slug) = path.file_stem().and_then(|s| s.to_str())
                {
                    on_disk.insert(slug.to_string());
                }
            }
        }

        entries.retain(|e| on_disk.contains(&e.slug));

        let missing: Vec<String> = on_disk
            .difference(&indexed)
            .cloned()
            .collect();

        if !missing.is_empty() {
            drop(entries);
            for slug in &missing {
                if let Ok(entry) = self.read(slug) {
                    self.index_entry(&entry)?;
                }
            }
        }

        Ok(())
    }

    fn index_entry(&self, entry: &WikiEntry) -> Result<()> {
        let text = format!("{} {}\n{}", entry.title, entry.tags.join(" "), entry.content);
        let vector = self.embedder.embed_one(&text)?;

        let mut entries = self.entries.write();
        entries.retain(|e| e.slug != entry.slug);
        entries.push(WikiVector {
            slug: entry.slug.clone(),
            title: entry.title.clone(),
            tags: entry.tags.clone(),
            content_preview: preview(&entry.content, 200),
            vector,
        });

        Ok(())
    }

    pub fn write(&self, slug: &str, title: &str, tags: &[String], content: &str) -> Result<()> {
        validate_slug(slug)?;

        let now = chrono_now();
        let file_path = self.wiki_dir.join(format!("{slug}.md"));

        let (created, updated) = if file_path.exists() {
            let existing = self.read(slug)?;
            (existing.created, now.clone())
        } else {
            (now.clone(), now.clone())
        };

        let md = format_frontmatter(slug, title, tags, &created, &updated, content);
        std::fs::write(&file_path, md)?;

        let entry = WikiEntry {
            slug: slug.to_string(),
            title: title.to_string(),
            tags: tags.to_vec(),
            created,
            updated,
            content: content.to_string(),
        };
        self.index_entry(&entry)?;
        self.persist()?;

        Ok(())
    }

    pub fn read(&self, slug: &str) -> Result<WikiEntry> {
        let file_path = self.wiki_dir.join(format!("{slug}.md"));
        let raw = std::fs::read_to_string(&file_path)
            .map_err(|_| FvaError::Wiki(format!("wiki entry '{slug}' not found")))?;
        parse_frontmatter(slug, &raw)
    }

    pub fn delete(&self, slug: &str) -> Result<()> {
        let file_path = self.wiki_dir.join(format!("{slug}.md"));
        if !file_path.exists() {
            return Err(FvaError::Wiki(format!("wiki entry '{slug}' not found")));
        }

        std::fs::remove_file(&file_path)?;
        self.entries.write().retain(|e| e.slug != slug);
        self.persist()?;

        Ok(())
    }

    pub fn search(
        &self,
        query: &str,
        tags_filter: Option<&[String]>,
        limit: usize,
    ) -> Result<Vec<(WikiEntry, f32)>> {
        let query_vector = self.embedder.embed_one(query)?;
        let entries = self.entries.read();

        let mut scored: Vec<(f32, usize)> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| {
                tags_filter.is_none_or(|tf| tf.iter().any(|t| e.tags.contains(t)))
            })
            .map(|(i, e)| (cosine_similarity(&query_vector, &e.vector), i))
            .collect();

        scored.sort_unstable_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        let mut results = Vec::with_capacity(scored.len());
        for (score, idx) in scored {
            let wv = &entries[idx];
            match self.read(&wv.slug) {
                Ok(entry) => results.push((entry, score)),
                Err(_) => continue,
            }
        }

        Ok(results)
    }

    pub fn list(&self, tags_filter: Option<&[String]>) -> Vec<WikiEntry> {
        let entries = self.entries.read();
        let mut results: Vec<WikiEntry> = entries
            .iter()
            .filter(|e| {
                tags_filter.is_none_or(|tf| tf.iter().any(|t| e.tags.contains(t)))
            })
            .filter_map(|e| self.read(&e.slug).ok())
            .collect();

        results.sort_by(|a, b| b.updated.cmp(&a.updated));
        results
    }

    pub fn stats(&self) -> WikiStats {
        WikiStats {
            total_entries: self.entries.read().len(),
        }
    }

    pub fn persist(&self) -> Result<()> {
        let snapshot = WikiSnapshot {
            entries: self.entries.read().clone(),
        };
        let bytes = bincode::serialize(&snapshot)
            .map_err(|e| FvaError::Wiki(format!("wiki serialize: {e}")))?;
        std::fs::write(&self.persist_path, bytes)?;
        Ok(())
    }
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(FvaError::Wiki("slug cannot be empty".into()));
    }
    if slug.contains("..")
        || slug.contains('/')
        || slug.contains('\\')
        || slug.contains('\0')
    {
        return Err(FvaError::Wiki(format!("invalid slug: {slug}")));
    }
    Ok(())
}

fn format_frontmatter(
    _slug: &str,
    title: &str,
    tags: &[String],
    created: &str,
    updated: &str,
    content: &str,
) -> String {
    let tags_str = tags.join(", ");
    format!(
        "---\ntitle: {title}\ntags: {tags_str}\ncreated: {created}\nupdated: {updated}\n---\n\n{content}\n"
    )
}

fn parse_frontmatter(slug: &str, raw: &str) -> Result<WikiEntry> {
    let raw = raw.trim_start();
    if !raw.starts_with("---") {
        return Ok(WikiEntry {
            slug: slug.to_string(),
            title: slug.to_string(),
            tags: Vec::new(),
            created: String::new(),
            updated: String::new(),
            content: raw.to_string(),
        });
    }

    let after_first = &raw[3..];
    let end = after_first
        .find("\n---")
        .ok_or_else(|| FvaError::Wiki(format!("unclosed frontmatter in '{slug}'")))?;

    let fm_block = &after_first[..end];
    let content = after_first[end + 4..].trim().to_string();

    let mut title = slug.to_string();
    let mut tags = Vec::new();
    let mut created = String::new();
    let mut updated = String::new();

    for line in fm_block.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "title" => title = value.to_string(),
                "tags" => {
                    tags = value
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .filter(|t| !t.is_empty())
                        .collect();
                }
                "created" => created = value.to_string(),
                "updated" => updated = value.to_string(),
                _ => {}
            }
        }
    }

    Ok(WikiEntry {
        slug: slug.to_string(),
        title,
        tags,
        created,
        updated,
        content,
    })
}

fn preview(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }
    let mut end = max_len.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &content[..end])
}

fn chrono_now() -> String {
    // ponytail: no chrono dep — UTC timestamp from SystemTime is enough for wiki metadata.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (y, m, d, hh, mm, ss) = unix_to_ymdhms(secs);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

fn unix_to_ymdhms(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let ss = secs % 60;
    let mins = secs / 60;
    let mm = mins % 60;
    let hh = (mins / 60) % 24;
    let mut days = secs / 86400;

    let mut y = 1970u64;
    loop {
        let dy = if is_leap(y) { 366 } else { 365 };
        if days < dy {
            break;
        }
        days -= dy;
        y += 1;
    }

    let leap = is_leap(y);
    let mdays: [u64; 12] = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    while m < 12 && days >= mdays[m] {
        days -= mdays[m];
        m += 1;
    }

    (y, m as u64 + 1, days + 1, hh, mm, ss)
}

fn is_leap(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter() {
        let raw = "---\ntitle: Test Page\ntags: rust, testing\ncreated: 2024-01-01T00:00:00Z\nupdated: 2024-01-02T00:00:00Z\n---\n\nHello world";
        let entry = parse_frontmatter("test", raw).unwrap();
        assert_eq!(entry.title, "Test Page");
        assert_eq!(entry.tags, vec!["rust", "testing"]);
        assert_eq!(entry.content, "Hello world");
    }

    #[test]
    fn test_parse_no_frontmatter() {
        let entry = parse_frontmatter("bare", "just some text").unwrap();
        assert_eq!(entry.title, "bare");
        assert!(entry.tags.is_empty());
        assert_eq!(entry.content, "just some text");
    }

    #[test]
    fn test_validate_slug() {
        assert!(validate_slug("my-page").is_ok());
        assert!(validate_slug("page_v2").is_ok());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("../etc/passwd").is_err());
        assert!(validate_slug("a/b").is_err());
    }

    #[test]
    fn test_roundtrip() {
        let md = format_frontmatter(
            "test",
            "My Title",
            &["tag1".into(), "tag2".into()],
            "2024-01-01T00:00:00Z",
            "2024-01-02T00:00:00Z",
            "Some content here",
        );
        let entry = parse_frontmatter("test", &md).unwrap();
        assert_eq!(entry.title, "My Title");
        assert_eq!(entry.tags, vec!["tag1", "tag2"]);
        assert_eq!(entry.content, "Some content here");
    }

    #[test]
    fn test_unix_to_ymdhms() {
        assert_eq!(unix_to_ymdhms(0), (1970, 1, 1, 0, 0, 0));
        assert_eq!(unix_to_ymdhms(1704067200), (2024, 1, 1, 0, 0, 0));
    }
}
