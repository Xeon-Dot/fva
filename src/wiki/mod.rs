//! Wiki knowledge base for AI coding agents.
//!
//! Markdown files with YAML-like frontmatter in `.fva/wiki/`,
//! backed by a separate vector index for semantic search.

use std::path::PathBuf;
use std::sync::Arc;

use std::sync::RwLock;
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
            *store.entries.write().unwrap() = snapshot.entries;
        }

        store.reconcile()?;

        Ok(store)
    }

    /// Sync vector index with .md files on disk.
    /// ponytail: O(n) scan on open; fine for wiki-scale data (<1k entries).
    fn reconcile(&self) -> Result<()> {
        let mut entries = self.entries.write().unwrap();
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

        let missing: Vec<String> = on_disk.difference(&indexed).cloned().collect();

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
        let text = format!(
            "{} {}\n{}",
            entry.title,
            entry.tags.join(" "),
            entry.content
        );
        let vector = self.embedder.embed_one(&text)?;

        let mut entries = self.entries.write().unwrap();
        entries.retain(|e| e.slug != entry.slug);
        entries.push(WikiVector {
            slug: entry.slug.clone(),
            title: entry.title.clone(),
            tags: entry.tags.clone(),
            content_preview: crate::util::truncate_preview(&entry.content, 200),
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

        let md = format_frontmatter(title, tags, &created, &updated, content);
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
        self.entries.write().unwrap().retain(|e| e.slug != slug);
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
        let entries = self.entries.read().unwrap();

        let mut scored: Vec<(f32, usize)> = entries
            .iter()
            .enumerate()
            .filter(|(_, e)| tags_filter.is_none_or(|tf| tf.iter().any(|t| e.tags.contains(t))))
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
        let entries = self.entries.read().unwrap();
        let mut results: Vec<WikiEntry> = entries
            .iter()
            .filter(|e| tags_filter.is_none_or(|tf| tf.iter().any(|t| e.tags.contains(t))))
            .filter_map(|e| self.read(&e.slug).ok())
            .collect();

        results.sort_by(|a, b| b.updated.cmp(&a.updated));
        results
    }

    pub fn stats(&self) -> WikiStats {
        WikiStats {
            total_entries: self.entries.read().unwrap().len(),
        }
    }

    pub fn persist(&self) -> Result<()> {
        let snapshot = WikiSnapshot {
            entries: self.entries.read().unwrap().clone(),
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
    if slug.contains("..") || slug.contains('/') || slug.contains('\\') || slug.contains('\0') {
        return Err(FvaError::Wiki(format!("invalid slug: {slug}")));
    }
    Ok(())
}

fn format_frontmatter(
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

    #[derive(serde::Deserialize)]
    struct FrontMatter {
        title: Option<String>,
        tags: Option<String>,
        created: Option<String>,
        updated: Option<String>,
    }

    let fm: FrontMatter = serde_yaml::from_str(fm_block)
        .map_err(|e| FvaError::Wiki(format!("invalid frontmatter in '{slug}': {e}")))?;

    let tags = fm
        .tags
        .map(|t| {
            t.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Ok(WikiEntry {
        slug: slug.to_string(),
        title: fm.title.unwrap_or_else(|| slug.to_string()),
        tags,
        created: fm.created.unwrap_or_default(),
        updated: fm.updated.unwrap_or_default(),
        content,
    })
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
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
        // Test that chrono_now produces valid RFC3339
        let now = chrono_now();
        assert!(now.contains('T'));
        assert!(now.ends_with('Z'));
    }
}
