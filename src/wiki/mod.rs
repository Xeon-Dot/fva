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
    pub entry_type: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
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

        store.migrate_legacy()?;
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
        collect_md_slugs(&self.wiki_dir, &self.wiki_dir, &mut on_disk);

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

    pub fn write(
        &self,
        slug: &str,
        title: &str,
        entry_type: &str,
        tags: &[String],
        sources: &[String],
        content: &str,
    ) -> Result<()> {
        validate_slug(slug)?;

        if slug == "index" || slug == "log" {
            return Err(FvaError::Wiki(format!("slug '{slug}' is reserved")));
        }

        let file_path = self.wiki_dir.join(format!("{slug}.md"));
        if slug.starts_with("sources/") && file_path.exists() {
            return Err(FvaError::Wiki(format!(
                "source entry '{slug}' is immutable"
            )));
        }
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let now = chrono_now();

        let (created, updated) = if file_path.exists() {
            let existing = self.read(slug)?;
            (existing.created, now.clone())
        } else {
            (now.clone(), now.clone())
        };

        let md = format_frontmatter(
            title, entry_type, tags, sources, &created, &updated, content,
        );
        std::fs::write(&file_path, md)?;

        let entry = WikiEntry {
            slug: slug.to_string(),
            title: title.to_string(),
            entry_type: entry_type.to_string(),
            tags: tags.to_vec(),
            sources: sources.to_vec(),
            created,
            updated,
            content: content.to_string(),
        };
        self.index_entry(&entry)?;
        if slug != "index" && slug != "log" {
            if let Err(e) = self.render_index() {
                tracing::warn!("wiki render_index failed: {e}");
            }
            if let Err(e) = self.append_log("write", slug, title) {
                tracing::warn!("wiki append_log failed: {e}");
            }
        }
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

        let title = self
            .read(slug)
            .map(|e| e.title)
            .unwrap_or_else(|_| slug.to_string());
        std::fs::remove_file(&file_path)?;
        self.entries.write().unwrap().retain(|e| e.slug != slug);
        if slug != "index" && slug != "log" {
            if let Err(e) = self.render_index() {
                tracing::warn!("wiki render_index failed: {e}");
            }
            if let Err(e) = self.append_log("delete", slug, &title) {
                tracing::warn!("wiki append_log failed: {e}");
            }
        }
        self.persist()?;

        Ok(())
    }

    fn render_index(&self) -> Result<()> {
        let mut entries = self.list(None);
        entries.retain(|e| e.slug != "index" && e.slug != "log");
        entries.sort_by(|a, b| b.updated.cmp(&a.updated));
        let mut md = String::from(
            "---\ntitle: Index\ntype: index\ntags: \ncreated: \nupdated: \n---\n\n# Index\n",
        );
        let mut area = String::new();
        for e in &entries {
            let a = e.slug.split('/').next().unwrap_or("general").to_string();
            if a != area {
                area = a.clone();
                md.push_str(&format!("\n## {area}\n"));
            }
            let summary: String = e
                .content
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .chars()
                .take(100)
                .collect();
            md.push_str(&format!("- [[{}]] — {} ({summary})\n", e.slug, e.title));
        }
        // index.md 자체는 frontmatter 최소형으로 직접 저장 (재귀가드: render는 write()를 호출하지 않음)
        std::fs::write(self.wiki_dir.join("index.md"), md)?;
        Ok(())
    }

    fn append_log(&self, op: &str, slug: &str, title: &str) -> Result<()> {
        let log_path = self.wiki_dir.join("log.md");
        if !log_path.exists() {
            std::fs::write(
                &log_path,
                "---\ntitle: Log\ntype: log\ntags: \ncreated: \nupdated: \n---\n\n# Log\n",
            )?;
        }
        let now = chrono_now();
        let day = now.get(..10).unwrap_or(&now);
        let line = format!("## [{day}] {op} | {slug} ({title})\n");
        use std::io::Write;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)?
            .write_all(line.as_bytes())?;
        Ok(())
    }

    fn migrate_legacy(&self) -> Result<()> {
        // ponytail: flat root scan on open; wiki-scale only.
        let mut legacy: Vec<String> = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.wiki_dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path.extension().is_some_and(|e| e == "md")
                    && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
                    && stem != "index"
                    && stem != "log"
                {
                    legacy.push(stem.to_string());
                }
            }
        }
        if legacy.is_empty() {
            return Ok(());
        }
        std::fs::create_dir_all(self.wiki_dir.join("concepts"))?;
        for name in &legacy {
            let raw = std::fs::read_to_string(self.wiki_dir.join(format!("{name}.md")))?;
            let raw = ensure_type_field(&raw, name);
            let mut target = format!("concepts/legacy-{name}");
            let mut n = 2;
            while self.wiki_dir.join(format!("{target}.md")).exists() {
                target = format!("concepts/legacy-{name}-{n}");
                n += 1;
            }
            std::fs::write(self.wiki_dir.join(format!("{target}.md")), raw)?;
            std::fs::remove_file(self.wiki_dir.join(format!("{name}.md")))?;
            self.entries.write().unwrap().retain(|e| e.slug != *name);
        }
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

pub fn extract_wikilinks(content: &str) -> Vec<String> {
    // ponytail: regex 크레이트 없이 수동 스캔. [[...]] 닫힘 없는 조각은 무시.
    let mut out = Vec::new();
    let mut rest = content;
    while let Some(s) = rest.find("[[") {
        let after = &rest[s + 2..];
        if let Some(e) = after.find("]]") {
            let link = after[..e].trim().to_string();
            if !link.is_empty() && !out.contains(&link) {
                out.push(link);
            }
            rest = &after[e + 2..];
        } else {
            break;
        }
    }
    out
}

fn collect_md_slugs(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut std::collections::HashSet<String>,
) {
    // ponytail: 재귀 read_dir; wiki-scale에서 walkdir 의존성 불필요.
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_md_slugs(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "md")
            && let Ok(rel) = path.strip_prefix(root)
            && let Some(s) = rel.to_str()
        {
            out.insert(s.strip_suffix(".md").unwrap_or(s).to_string());
        }
    }
}

fn ensure_type_field(raw: &str, name: &str) -> String {
    if raw.contains("type:") {
        return raw.to_string();
    }
    if let Some(body) = raw.strip_prefix("---") {
        format!("---\ntype: concept{body}")
    } else {
        format!(
            "---\ntitle: {name}\ntype: concept\ntags: \nsources:\ncreated: \nupdated: \n---\n\n{raw}"
        )
    }
}

fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() {
        return Err(FvaError::Wiki("slug cannot be empty".into()));
    }
    if slug.contains("..") || slug.contains('\\') || slug.contains('\0') {
        return Err(FvaError::Wiki(format!("invalid slug: {slug}")));
    }
    if slug.split('/').any(|s| s.is_empty()) {
        return Err(FvaError::Wiki(format!("invalid slug: {slug}")));
    }
    Ok(())
}

fn format_frontmatter(
    title: &str,
    entry_type: &str,
    tags: &[String],
    sources: &[String],
    created: &str,
    updated: &str,
    content: &str,
) -> String {
    let tags_str = tags.join(", ");
    let sources_str = sources.join(", ");
    format!(
        "---\ntitle: {title}\ntype: {entry_type}\ntags: {tags_str}\nsources: {sources_str}\ncreated: {created}\nupdated: {updated}\n---\n\n{content}\n"
    )
}

fn parse_frontmatter(slug: &str, raw: &str) -> Result<WikiEntry> {
    let raw = raw.trim_start();
    if !raw.starts_with("---") {
        return Ok(WikiEntry {
            slug: slug.to_string(),
            title: slug.to_string(),
            entry_type: "concept".to_string(),
            tags: Vec::new(),
            sources: Vec::new(),
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
        #[serde(rename = "type")]
        entry_type: Option<String>,
        tags: Option<String>,
        sources: Option<String>,
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

    let sources = fm
        .sources
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
        entry_type: fm.entry_type.unwrap_or_else(|| "concept".to_string()),
        tags,
        sources,
        created: fm.created.unwrap_or_default(),
        updated: fm.updated.unwrap_or_default(),
        content,
    })
}

fn chrono_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn slugify(hint: &str) -> String {
    let mut s: String = hint
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "untitled".into() } else { s }
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
        assert!(validate_slug("a/b").is_ok());
    }

    #[test]
    fn test_roundtrip() {
        let md = format_frontmatter(
            "My Title",
            "concept",
            &["tag1".into(), "tag2".into()],
            &["src/main.rs".into()],
            "2024-01-01T00:00:00Z",
            "2024-01-02T00:00:00Z",
            "Some content here",
        );
        let entry = parse_frontmatter("test", &md).unwrap();
        assert_eq!(entry.title, "My Title");
        assert_eq!(entry.tags, vec!["tag1", "tag2"]);
        assert_eq!(entry.sources, vec!["src/main.rs"]);
        assert_eq!(entry.content, "Some content here");
    }

    #[test]
    fn test_unix_to_ymdhms() {
        // Test that chrono_now produces valid RFC3339
        let now = chrono_now();
        assert!(now.contains('T'));
        assert!(now.ends_with('Z'));
    }

    #[test]
    fn test_parse_new_frontmatter() {
        let raw = "---\ntitle: Foo\ntype: adr\ntags: rust\nsources: src/main.rs\ncreated: 2026-01-01T00:00:00Z\nupdated: 2026-01-02T00:00:00Z\n---\n\nBody [[concepts/bar]]";
        let entry = parse_frontmatter("adrs/foo", raw).unwrap();
        assert_eq!(entry.entry_type, "adr");
        assert_eq!(entry.sources, vec!["src/main.rs"]);
        assert_eq!(entry.content, "Body [[concepts/bar]]");
    }

    #[test]
    fn test_slugify_and_validate() {
        assert_eq!(slugify("Hello World_2!"), "hello-world-2");
        assert_eq!(slugify("!!!"), "untitled");
        assert!(validate_slug("concepts/foo").is_ok());
        assert!(validate_slug("a//b").is_err());
        assert!(validate_slug("../x").is_err());
        assert!(validate_slug("a/b").is_ok());
    }

    #[test]
    fn test_extract_wikilinks() {
        let links = extract_wikilinks("see [[concepts/a]] and [[adrs/b]] plus [[concepts/a]]");
        assert_eq!(links, vec!["concepts/a", "adrs/b"]);
    }

    #[test]
    fn test_write_blocks_reserved_and_source_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let store = WikiStore::open(
            dir.path().join("wiki"),
            Arc::new(crate::embedding::LocalEmbedder::new(128)),
        )
        .unwrap();
        assert!(store.write("index", "I", "index", &[], &[], "x").is_err());
        assert!(store.write("log", "L", "log", &[], &[], "x").is_err());
        store
            .write("sources/s1", "S", "source", &[], &[], "raw")
            .unwrap();
        assert!(
            store
                .write("sources/s1", "S", "source", &[], &[], "raw2")
                .is_err()
        );
    }

    #[test]
    fn test_write_maintains_index_and_log() {
        let dir = tempfile::tempdir().unwrap();
        let store = WikiStore::open(
            dir.path().join("wiki"),
            Arc::new(crate::embedding::LocalEmbedder::new(128)),
        )
        .unwrap();
        store
            .write(
                "concepts/foo",
                "Foo",
                "concept",
                &["rust".into()],
                &[],
                "Body",
            )
            .unwrap();
        let index = std::fs::read_to_string(dir.path().join("wiki/index.md")).unwrap();
        assert!(index.contains("[[concepts/foo]]"));
        let log = std::fs::read_to_string(dir.path().join("wiki/log.md")).unwrap();
        assert!(log.contains("write | concepts/foo"));
    }

    #[test]
    fn test_migrate_legacy_moves_root_files() {
        let dir = tempfile::tempdir().unwrap();
        let wiki_dir = dir.path().join("wiki");
        std::fs::create_dir_all(&wiki_dir).unwrap();
        std::fs::write(
            wiki_dir.join("old-note.md"),
            "---\ntitle: Old\ntags: \ncreated: \nupdated: \n---\n\nLegacy body",
        )
        .unwrap();
        let store = WikiStore::open(
            wiki_dir.clone(),
            Arc::new(crate::embedding::LocalEmbedder::new(128)),
        )
        .unwrap();
        assert!(!wiki_dir.join("old-note.md").exists());
        let moved = std::fs::read_to_string(wiki_dir.join("concepts/legacy-old-note.md")).unwrap();
        assert!(moved.contains("type:"));
        assert!(moved.contains("Legacy body"));
        assert!(store.read("concepts/legacy-old-note").is_ok());
    }
}
