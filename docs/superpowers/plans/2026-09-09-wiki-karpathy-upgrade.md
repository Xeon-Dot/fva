# Wiki Karpathy Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** FVA wiki를 Karpathy식 compounding knowledge base로 승격한다 (新레이아웃 + `wiki_ingest`/`wiki_query`/`wiki_lint` 네이티브 3툴).

**Architecture:** `src/wiki/mod.rs`의 `WikiStore`에 타입·소스·링크·index/log 책임을 넣고, MCP·CLI는 얇은 어댑터로 둔다. 요약 판단은 호출자 LLM에 위임 (Rust에 LLM 없음).

**Tech Stack:** Rust edition 2024, tokio (기존), rmcp `#[tool]` (기존 패턴), bincode+serde (기존), `cargo test`.

**Spec:** `docs/superpowers/specs/2026-09-09-wiki-karpathy-upgrade-design.md`

## Global Constraints

- 구 wiki 호환 전부 드랍 (브레이킹 허용), 루트 구 `*.md`는 `open()`시 1회 `concepts/legacy-*`로 이주.
- slug = 확장자 뺀 상대경로, `/` 허용, `..`·`\`·`\0`·빈 세그먼트 거부.
- `sources/*` write-once 강제, 예약 slug (`index`/`log`) 직접 `wiki_write` 거부.
- 벡터 인덱스에서 `index`/`log` 제외.
- `[[ ]]` 그래프는 별도 persist 없이 주문형 O(n) 스캔 (`ponytail:` 표기).
- index/log 재생성 실패해도 원본 write는 성공 + `tracing::warn`.
- YAGNI: URL fetch, LLM 내장요약, lint 자동수정 없음.

---

## File Structure

- Modify `src/wiki/mod.rs` (377줄 → ~700줄 예상): 스키마·slug·링크·index/log·ingest/query/lint 코어 전부. 단일 책임(위키 지식관리)은 유지, 파일 분할 안 함 (기존 규모에서 분할은 오버엔지니어링).
- Modify `src/mcp/server.rs`: params 구조체 3개 추가 + `#[tool]` 3개 + `WikiWriteParams`에 `entry_type`/`sources` 추가.
- Modify `src/main.rs`: `WikiCommands`에 Ingest/Query/Lint + `Write`에 `--type --sources` + `List/Search`에 `--type`.
- Modify `src/cli_output.rs`: 출력함수 3개 추가.
- Create `tests/wiki_karpathy.rs`: MCP 이전 단계의 `WikiStore` 왕복 통합테스트 1파일.
- Modify `skills/fva/SKILL.md`, `skills/fva/references/mcp-tools.md`: query-first 워크플로 문서.

---

### Task 1: 신스키마 + slug (WikiStore 기초)

**Files:**

- Modify: `src/wiki/mod.rs:15-23` (`WikiEntry` 구조체), `src/wiki/mod.rs:246-323` (`validate_slug`, `format_frontmatter`, `parse_frontmatter`, `chrono_now` 아래에 `slugify` 추가)
- Test: `src/wiki/mod.rs` 내 `#[cfg(test)]` 모듈 (기존 `test_parse_frontmatter` 등 유지·확장)

**Interfaces:**

- Consumes: 기존 `WikiEntry`, `FvaError::Wiki`, `chrono_now()`.
- Produces: `WikiEntry { slug, title, entry_type: String, tags: Vec<String>, sources: Vec<String>, created, updated, content }`, `pub fn slugify(hint: &str) -> String`, 新 `validate_slug` (서브디렉 허용). Task 2~4가 사용.

- [ ] **Step 1: 스키마 변경에 대한 실패 테스트 작성**

```rust
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
```

- [ ] **Step 2: 테스트 실행해 실패 확인**

Run: `cargo test --lib wiki:: 2>&1 | tail -20`
Expected: FAIL (`entry_type`·`sources` 필드 없음, `slugify` 미정의, `a/b` 거부됨)

- [ ] **Step 3: 최소 구현**

```rust
// WikiEntry에 2필드 추가:
pub entry_type: String,   // source|entity|concept|analysis|adr|arch|gotcha|index|log
pub sources: Vec<String>, // 쉼표구분 원천 경로·URL
```

```rust
fn validate_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug == "index" || slug == "log" {
        // NOTE: index/log는 직접 write 금지라 여기서도 거부하지 않음 — write()에서 거부.
    }
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
```

`format_frontmatter`에 `type:`·`sources:` 줄 추가, `parse_frontmatter`의 내부 `FrontMatter`에
`entry_type: Option<String>` (serde rename `type`), `sources: Option<String>` 추가 후
쉼표분리. `entry_type` 기본값 `"concept"`, `sources` 기본값 빈 벡터.
`write()` 시그니처는 Task 2에서 변경하므로 여기서는 구조체·파서·slug만.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cargo test --lib wiki:: 2>&1 | tail -5`
Expected: PASS (기존 5개 테스트 포함 전원 통과)

- [ ] **Step 5: Commit**

```bash
git add src/wiki/mod.rs
git commit -m "feat(wiki): typed frontmatter and subdirectory slugs"
```

### Task 2: index/log 자동유지 + sources 불변 + legacy 이주

**Files:**

- Modify: `src/wiki/mod.rs` (`write`, `delete`, `reconcile`, 신규 `extract_wikilinks`, `render_index`, `append_log`, `migrate_legacy`)
- Test: `src/wiki/mod.rs` 내 `#[cfg(test)]` 모듈

**Interfaces:**

- Consumes: Task 1의 `WikiEntry{entry_type, sources}`, `slugify`, 新 `validate_slug`.
- Produces: `pub fn write(&self, slug, title, entry_type: &str, tags: &[String], sources: &[String], content: &str)`, `extract_wikilinks(&str) -> Vec<String>`, `render_index`, `append_log`, `migrate_legacy`. Task 3~6이 사용.

- [ ] **Step 1: 실패 테스트 작성**

```rust
#[test]
fn test_extract_wikilinks() {
    let links = extract_wikilinks("see [[concepts/a]] and [[adrs/b]] plus [[concepts/a]]");
    assert_eq!(links, vec!["concepts/a", "adrs/b"]);
}

#[test]
fn test_write_blocks_reserved_and_source_rewrite() {
    let dir = tempfile::tempdir().unwrap();
    let store = WikiStore::open(dir.path().join("wiki"), Arc::new(crate::embedding::HashEmbedder::default())).unwrap();
    assert!(store.write("index", "I", "index", &[], &[], "x").is_err());
    store.write("sources/s1", "S", "source", &[], &[], "raw").unwrap();
    assert!(store.write("sources/s1", "S", "source", &[], &[], "raw2").is_err());
}

#[test]
fn test_write_maintains_index_and_log() {
    let dir = tempfile::tempdir().unwrap();
    let store = WikiStore::open(dir.path().join("wiki"), Arc::new(crate::embedding::HashEmbedder::default())).unwrap();
    store.write("concepts/foo", "Foo", "concept", &["rust".into()], &[], "Body").unwrap();
    let index = std::fs::read_to_string(dir.path().join("wiki/index.md")).unwrap();
    assert!(index.contains("[[concepts/foo]]"));
    let log = std::fs::read_to_string(dir.path().join("wiki/log.md")).unwrap();
    assert!(log.contains("write | concepts/foo"));
}
```

NOTE: `HashEmbedder` 이름은 `src/embedding/` 실제 구조체명에 맞춰 조정 (없으면 기존 테스트가 쓰는 embedder 사용). `tempfile`이 dev-dependencies에 없으면 `std::env::temp_dir()` + 프로세스ID 하위디렉으로 대체.

- [ ] **Step 2: 실패 확인**

Run: `cargo test --lib wiki:: 2>&1 | tail -20`
Expected: FAIL (함수·동작 없음)

- [ ] **Step 3: 최소 구현**

```rust
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
```

`write()` 변경점: 시그니처에 `entry_type: &str, sources: &[String]` 추가 →
`validate_slug` → 예약slug 거부 → `sources/*` 존재시 거부 →
상위디렉 `create_dir_all` → frontmatter 포함 저장 → `index_entry` →
자기 자신이 index/log가 아닐 때만 `render_index()` + `append_log("write", slug, title)` →
`persist()`. index/log 재생성 실패는 `tracing::warn!` 후 성공 반환.

```rust
fn render_index(&self) -> Result<()> {
    let mut entries = self.list(None);
    entries.retain(|e| e.slug != "index" && e.slug != "log");
    entries.sort_by(|a, b| b.updated.cmp(&a.updated));
    let mut md = String::from("---\ntitle: Index\ntype: index\ntags: \ncreated: \nupdated: \n---\n\n# Index\n");
    let mut area = String::new();
    for e in &entries {
        let a = e.slug.split('/').next().unwrap_or("general").to_string();
        if a != area {
            area = a.clone();
            md.push_str(&format!("\n## {area}\n"));
        }
        let summary: String = e.content.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(100).collect();
        md.push_str(&format!("- [[{}]] — {} ({summary})\n", e.slug, e.title));
    }
    let now = chrono_now();
    let full = md; // index.md 자체는 frontmatter 최소형으로 직접 저장 (재귀가드: render는 write()를 호출하지 않음)
    std::fs::write(self.wiki_dir.join("index.md"), full)?;
    Ok(())
}
```

`append_log(op, slug, title)`: `log.md` 없으면 헤더 frontmatter+`# Log` 생성 후
`## [<YYYY-MM-DD>] <op> | <title> (<slug>)\n` append (날짜는 `created[..10]`).

`migrate_legacy()`: `open()`에서 `reconcile()` 전에 호출. 루트 `*.md` 중
`index.md`/`log.md` 제외 → `concepts/legacy-<name>.md`로 이동
(충돌시 `-2` 접미사) → 내용에 `type:` 없으면 삽입. 이후 스냅샷 파기·리빌드
(기존 `reconcile`의 missing 경로가 자동 인덱싱하므로 이동만 하면 됨).

- [ ] **Step 4: 통과 확인**

Run: `cargo test --lib wiki:: 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/wiki/mod.rs
git commit -m "feat(wiki): index/log maintenance, immutable sources, legacy migration"
```

### Task 3: `ingest_plan` + `query_bundle` (WikiStore 코어)

**Files:**

- Modify: `src/wiki/mod.rs` (신규 `pub struct IngestPlan`, `pub fn ingest(...)`, `pub fn query_bundle(...)`)
- Test: `src/wiki/mod.rs` 내 `#[cfg(test)]`

**Interfaces:**

- Consumes: Task 2의 `write`, `search`, `extract_wikilinks`.
- Produces: `pub struct IngestPlan { source_slug: String, related: Vec<(String, f32)>, neighbors: Vec<String>, suggested_slugs: Vec<String> }`, `pub fn ingest(&self, content: &str, source_uri: Option<&str>, title_hint: Option<&str>, area_hint: Option<&str>) -> Result<IngestPlan>`, `pub fn query_bundle(&self, query: &str, limit: usize) -> Result<String>`. Task 5(MCP)가 호출.

- [ ] **Step 1: 실패 테스트 작성**

```rust
#[test]
fn test_ingest_returns_plan() {
    let dir = tempfile::tempdir().unwrap();
    let store = WikiStore::open(dir.path().join("wiki"), Arc::new(crate::embedding::HashEmbedder::default())).unwrap();
    store.write("concepts/rust-ownership", "Ownership", "concept", &["rust".into()], &[], "Ownership and borrowing [[concepts/borrowck]]").unwrap();
    let plan = store.ingest("Rust borrowing rules and lifetimes", None, Some("Borrowing"), Some("concept")).unwrap();
    assert!(plan.source_slug.starts_with("sources/"));
    assert!(!plan.related.is_empty());
    assert!(plan.suggested_slugs.contains(&"concepts/borrowing".to_string()));
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test --lib wiki::ingest 2>&1 | tail -10`
Expected: FAIL (메서드 없음)

- [ ] **Step 3: 최소 구현**

```rust
pub struct IngestPlan {
    pub source_slug: String,
    pub related: Vec<(String, f32)>,   // (slug, score) 상위 10
    pub neighbors: Vec<String>,        // 관련페이지의 1-hop [[링크]]
    pub suggested_slugs: Vec<String>,  // 최대 15
}

pub fn ingest(&self, content: &str, source_uri: Option<&str>, title_hint: Option<&str>, area_hint: Option<&str>) -> Result<IngestPlan> {
    let area = area_hint.unwrap_or("concept");
    let base = slugify(title_hint.unwrap_or("untitled"));
    let mut slug = format!("sources/{base}");
    let mut n = 2;
    while self.wiki_dir.join(format!("{slug}.md")).exists() {
        slug = format!("sources/{base}-{n}");
        n += 1;
    }
    let title = title_hint.unwrap_or("Untitled source").to_string();
    let src_tag = source_uri.map(|u| vec![u.to_string()]).unwrap_or_default();
    self.write(&slug, &title, "source", &[], &src_tag, content)?;
    self.append_log("ingest", &slug, &title).ok();

    let hits = self.search(content, None, 10).unwrap_or_default();
    let mut neighbors = Vec::new();
    for (entry, _) in &hits {
        for l in extract_wikilinks(&entry.content) {
            if !neighbors.contains(&l) {
                neighbors.push(l);
            }
        }
    }
    let mut suggested: Vec<String> = hits.iter()
        .map(|(e, _)| e.slug.clone())
        .filter(|s| s != &slug)
        .take(14)
        .collect();
    let fresh = format!("{area}/{base}");
    if !suggested.contains(&fresh) {
        suggested.insert(0, fresh);
    }
    suggested.truncate(15);
    Ok(IngestPlan {
        source_slug: slug,
        related: hits.into_iter().map(|(e, s)| (e.slug, s)).collect(),
        neighbors,
        suggested_slugs: suggested,
    })
}

pub fn query_bundle(&self, query: &str, limit: usize) -> Result<String> {
    let mut out = String::new();
    let index_path = self.wiki_dir.join("index.md");
    if let Ok(idx) = std::fs::read_to_string(&index_path) {
        out.push_str("## Index\n");
        out.push_str(&idx.chars().take(4000).collect::<String>());
        out.push_str("\n");
    }
    let hits = self.search(query, None, limit).unwrap_or_default();
    out.push_str(&format!("## Top {} semantic hits\n", hits.len()));
    for (i, (e, score)) in hits.iter().enumerate() {
        if i < 3 {
            out.push_str(&format!("\n### [[{}]] — {} (score={:.3})\n{}\n", e.slug, e.title, score, e.content));
            let nb = extract_wikilinks(&e.content);
            if !nb.is_empty() {
                out.push_str(&format!("links: {}\n", nb.iter().map(|l| format!("[[{l}]]")).collect::<Vec<_>>().join(" ")));
            }
        } else {
            let preview: String = e.content.chars().take(200).collect();
            out.push_str(&format!("\n- [[{}]] — {} (score={:.3}) {preview}\n", e.slug, e.title, score));
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: 통과 확인**

Run: `cargo test --lib wiki:: 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/wiki/mod.rs
git commit -m "feat(wiki): ingest plan and query bundle"
```

### Task 4: `lint_report` (WikiStore 코어)

**Files:**

- Modify: `src/wiki/mod.rs` (신규 `pub fn lint_report(&self, stale_days: i64) -> Result<String>`)
- Test: `src/wiki/mod.rs` 내 `#[cfg(test)]`

**Interfaces:**

- Consumes: Task 1~2의 `list`, `read`, `extract_wikilinks`, 저장 벡터(`entries`).
- Produces: `pub fn lint_report(&self, stale_days: i64) -> Result<String>`. Task 5가 호출.

- [ ] **Step 1: 실패 테스트 작성**

```rust
#[test]
fn test_lint_finds_orphan_and_dead_link() {
    let dir = tempfile::tempdir().unwrap();
    let store = WikiStore::open(dir.path().join("wiki"), Arc::new(crate::embedding::HashEmbedder::default())).unwrap();
    store.write("concepts/orphan", "Orphan", "concept", &[], &[], "lonely, see [[concepts/nowhere]]").unwrap();
    store.write("concepts/hub", "Hub", "concept", &[], &[], "hub body").unwrap();
    let report = store.lint_report(9999).unwrap();
    assert!(report.contains("orphan"));
    assert!(report.contains("concepts/nowhere"));
}
```

- [ ] **Step 2: 실패 확인**

Run: `cargo test --lib wiki::lint 2>&1 | tail -10`
Expected: FAIL

- [ ] **Step 3: 최소 구현**

```rust
pub fn lint_report(&self, stale_days: i64) -> Result<String> {
    // ponytail: 전수스캔 O(n) + 유사쌍 O(n²). wiki-scale(<1k) 허용.
    let entries = self.list(None);
    let live: Vec<_> = entries.iter().filter(|e| e.slug != "index" && e.slug != "log").collect();
    let slugs: std::collections::HashSet<&str> = live.iter().map(|e| e.slug.as_str()).collect();
    let mut inbound: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut dead = Vec::new();
    for e in &live {
        for l in extract_wikilinks(&e.content) {
            if slugs.contains(l.as_str()) {
                *inbound.entry(l.as_str()).or_default() += 1;
            } else {
                dead.push((e.slug.clone(), l));
            }
        }
    }
    let mut out = String::from("# Wiki lint\n");
    out.push_str("\n## Orphans (no inbound links)\n");
    for e in &live {
        if e.entry_type != "source" && *inbound.get(e.slug.as_str()).unwrap_or(&0) == 0 {
            out.push_str(&format!("- [[{}]] — {}\n", e.slug, e.title));
        }
    }
    out.push_str("\n## Dead links\n");
    for (from, to) in &dead {
        out.push_str(&format!("- [[{from}]] → [[{to}]] (missing)\n"));
    }
    out.push_str("\n## Stale\n");
    let cutoff = chrono::Utc::now() - chrono::Duration::days(stale_days);
    for e in &live {
        if e.entry_type == "source" { continue; }
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&e.updated) {
            if dt.with_timezone(&chrono::Utc) < cutoff {
                out.push_str(&format!("- [[{}]] — {} (updated {})\n", e.slug, e.title, e.updated));
            }
        }
    }
    out.push_str("\n## Contradiction candidates\n");
    let vecs = self.entries.read().unwrap();
    let mut pairs: Vec<(&str, &str, f32)> = Vec::new();
    for i in 0..vecs.len() {
        for j in (i + 1)..vecs.len() {
            let s = crate::embedding::cosine_similarity(&vecs[i].vector, &vecs[j].vector);
            if s > 0.85 {
                pairs.push((vecs[i].slug.as_str(), vecs[j].slug.as_str(), s));
            }
        }
    }
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for (a, b, s) in pairs.iter().take(20) {
        // 반전 키워드 동시포함 쌍만 보고 (저비용 휴리스틱)
        let ca = self.read(a).map(|e| e.content.to_lowercase()).unwrap_or_default();
        let cb = self.read(b).map(|e| e.content.to_lowercase()).unwrap_or_default();
        const NEG: [&str; 6] = ["not", "no", "never", "deprecated", "instead", "avoid"];
        if NEG.iter().any(|w| ca.contains(w)) && NEG.iter().any(|w| cb.contains(w)) {
            out.push_str(&format!("- [[{a}]] ↔ [[{b}]] (sim={s:.3})\n"));
        }
    }
    out.push_str("\n## Thin areas (<3 entries)\n");
    let mut areas: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in &live {
        *areas.entry(e.slug.split('/').next().unwrap_or("general").to_string()).or_default() += 1;
    }
    let mut names: Vec<_> = areas.iter().collect();
    names.sort();
    for (name, count) in names {
        if *count < 3 {
            out.push_str(&format!("- {name}/ ({count} entries)\n"));
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: 통과 확인**

Run: `cargo test --lib wiki:: 2>&1 | tail -5`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/wiki/mod.rs
git commit -m "feat(wiki): lint report (orphans, dead links, stale, contradictions, gaps)"
```

### Task 5: MCP 3툴 + write 확장

**Files:**

- Modify: `src/mcp/server.rs` (`WikiWriteParams`에 `entry_type`·`sources` 추가, params 3종 + `#[tool]` 3개 추가)

**Interfaces:**

- Consumes: Task 3~4의 `ingest`, `query_bundle`, `lint_report`, Task 2의 新 `write`.
- Produces: `wiki_ingest`, `wiki_query`, `wiki_lint` MCP 툴. Task 6·8이 문서화.

- [ ] **Step 1: params 구조체 추가 (컴파일 실패가 테스트)**

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WikiIngestParams {
    /// Raw source text to preserve under sources/.
    pub content: String,
    /// Original path or URL of the source.
    pub source_uri: Option<String>,
    /// Hint for the source title.
    pub title_hint: Option<String>,
    /// One of: entity, concept, analysis, adr, arch, gotcha.
    pub area_hint: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WikiQueryParams {
    /// Natural language query. Index-first drill-down bundle is returned.
    pub query: String,
    #[serde(rename = "maxResults")]
    pub max_results: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WikiLintParams {
    /// Days after which a non-source entry counts as stale.
    pub stale_days: Option<f64>,
}
```

`WikiWriteParams`에 추가:

```rust
    /// Entry type: source|entity|concept|analysis|adr|arch|gotcha. Default: concept.
    pub entry_type: Option<String>,
    /// Comma-separated source paths or URLs.
    pub sources: Option<String>,
```

- [ ] **Step 2: 컴파일 확인**

Run: `cargo build 2>&1 | tail -5`
Expected: 기존 `wiki_write` 호출부가 新 `write()` 시그니처와 불일치해 FAIL (다음 스텝에서 수정)

- [ ] **Step 3: 툴 핸들러 구현**

```rust
// wiki_write 본문을 新 시그니처로 교체:
let tags = parse_tags(&params.tags.unwrap_or_default());
let entry_type = params.entry_type.unwrap_or_else(|| "concept".into());
let sources = parse_tags(&params.sources.unwrap_or_default());
self.engine.wiki.write(&params.slug, &params.title, &entry_type, &tags, &sources, &params.content)
    .map_err(|e| ErrorData::internal_error(format!("wiki_write failed: {e}"), None))?;
```

```rust
#[tool(name = "wiki_ingest", description = "Ingest a source into the wiki (Karpathy-style). Saves raw text under sources/ (immutable), finds related pages via semantic search + wikilink neighbors, and returns an ingest plan with up to 15 suggested pages to touch. Write summaries with wiki_write afterwards. Params: content (required), source_uri, title_hint, area_hint.")]
fn wiki_ingest(&self, Parameters(params): Parameters<WikiIngestParams>) -> Result<CallToolResult, ErrorData> {
    let plan = self.engine.wiki.ingest(&params.content, params.source_uri.as_deref(), params.title_hint.as_deref(), params.area_hint.as_deref())
        .map_err(|e| ErrorData::internal_error(format!("wiki_ingest failed: {e}"), None))?;
    let mut out = format!("Ingested as [[{}]].\n\n## Related pages\n", plan.source_slug);
    for (slug, score) in &plan.related {
        out.push_str(&format!("- [[{slug}]] (score={score:.3})\n"));
    }
    out.push_str("\n## Link neighbors\n");
    for n in &plan.neighbors {
        out.push_str(&format!("- [[{n}]]\n"));
    }
    out.push_str("\n## Suggested pages to touch (max 15)\n");
    for s in &plan.suggested_slugs {
        out.push_str(&format!("- [[{s}]]\n"));
    }
    out.push_str("\nWrite the summary with wiki_write, then cross-link with [[slug]] references.");
    Ok(CallToolResult::success(vec![Content::text(out)]))
}

#[tool(name = "wiki_query", description = "Query the wiki index-first (Karpathy-style). Returns index.md plus top semantic hits plus 1-hop wikilink neighbors. Drill down with wiki_read. File good answers back with wiki_write. Params: query (required), maxResults.")]
fn wiki_query(&self, Parameters(params): Parameters<WikiQueryParams>) -> Result<CallToolResult, ErrorData> {
    let (limit, _offset) = resolve_pagination(params.max_results, None, self.default_max_results);
    let bundle = self.engine.wiki.query_bundle(&params.query, limit)
        .map_err(|e| ErrorData::internal_error(format!("wiki_query failed: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(bundle)]))
}

#[tool(name = "wiki_lint", description = "Lint the wiki (Karpathy-style): orphans, dead [[links]], stale entries, contradiction candidates, thin areas. Report only, no auto-fix. Params: stale_days (default 180).")]
fn wiki_lint(&self, Parameters(params): Parameters<WikiLintParams>) -> Result<CallToolResult, ErrorData> {
    let days = params.stale_days.map(|d| d as i64).unwrap_or(180);
    let report = self.engine.wiki.lint_report(days)
        .map_err(|e| ErrorData::internal_error(format!("wiki_lint failed: {e}"), None))?;
    Ok(CallToolResult::success(vec![Content::text(report)]))
}
```

`wiki_search`·`wiki_list`에 `type` 필터가 필요하면 `WikiSearchParams`·`WikiListParams`에
`entry_type: Option<String>` 추가 후 `search`·`list` 호출 뒤 `.filter(entry_type 일치)` —
스펙의 "type 필터 추가" 최소충족. (WikiStore 시그니처 변경 없이 핸들러에서 후필터.)

- [ ] **Step 4: 빌드+기존 테스트 통과**

Run: `cargo build 2>&1 | tail -3 && cargo test --lib wiki:: 2>&1 | tail -3`
Expected: BUILD OK, PASS

- [ ] **Step 5: Commit**

```bash
git add src/mcp/server.rs
git commit -m "feat(mcp): wiki_ingest, wiki_query, wiki_lint tools"
```

### Task 6: CLI + 출력

**Files:**

- Modify: `src/main.rs` (`WikiCommands`: Ingest/Query/Lint 추가, Write에 `--type --sources`, List/Search에 `--type`), `src/cli_output.rs` (출력 3종)

**Interfaces:**

- Consumes: Task 3~4 (`ingest`, `query_bundle`, `lint_report`).
- Produces: `fva wiki ingest|query|lint` 서브커맨드.

- [ ] **Step 1: enum 확장**

```rust
/// Ingest a source (Karpathy-style): save under sources/ + print ingest plan.
Ingest {
    /// Raw text, or omit to read from stdin.
    #[arg(long)]
    content: Option<String>,
    /// Read raw text from this file instead of --content/stdin.
    #[arg(long)]
    file: Option<String>,
    /// Hint for the source title.
    #[arg(long)]
    title: Option<String>,
    /// Original path or URL.
    #[arg(long)]
    source_uri: Option<String>,
    /// One of: entity, concept, analysis, adr, arch, gotcha.
    #[arg(long)]
    area_hint: Option<String>,
},
/// Query the wiki index-first.
Query {
    /// Search query.
    query: String,
},
/// Lint the wiki.
Lint {
    /// Days after which a non-source entry counts as stale.
    #[arg(long, default_value_t = 180)]
    stale_days: i64,
},
```

`Write`에 `#[arg(long)] entry_type: Option<String>, #[arg(long)] sources: Option<String>` 추가.
`List`·`Search`에 `#[arg(long)] entry_type: Option<String>` 추가.

- [ ] **Step 2: 컴파일 (매치 누락으로 실패)**

Run: `cargo build 2>&1 | tail -5`
Expected: FAIL (`WikiCommands` 비완전 매치)

- [ ] **Step 3: 핸들러 + 출력 구현**

main.rs 매치암에 (기존 wiki 분기 옆):

```rust
WikiCommands::Ingest { content, file, title, source_uri, area_hint } => {
    let text = match (content, file) {
        (Some(c), _) => c,
        (_, Some(f)) => std::fs::read_to_string(&f)?,
        _ => std::io::read_to_string(std::io::stdin())?,
    };
    let plan = engine.wiki.ingest(&text, source_uri.as_deref(), title.as_deref(), area_hint.as_deref())?;
    cli_output::wiki_ingest_plan(&plan.source_slug, &plan.related, &plan.neighbors, &plan.suggested_slugs);
}
WikiCommands::Query { query } => {
    let bundle = engine.wiki.query_bundle(&query, 10)?;
    println!("{bundle}");
}
WikiCommands::Lint { stale_days } => {
    let report = engine.wiki.lint_report(stale_days)?;
    println!("{report}");
}
```

`Write` 호출부를 新 `write(slug, title, entry_type, tags, sources, content)`로 교체
(기본 `entry_type="concept"`). `List`·`Search` 결과에 후필터 적용.

cli_output.rs에:

```rust
pub fn wiki_ingest_plan(source: &str, related: &[(String, f32)], neighbors: &[String], suggested: &[String]) {
    println!("\n  ingested as [[{source}]]\n");
    println!("  related:");
    for (s, score) in related { println!("    [[{s}]] ({score:.3})"); }
    println!("  neighbors:");
    for n in neighbors { println!("    [[{n}]]"); }
    println!("  touch (max 15):");
    for s in suggested { println!("    [[{s}]]"); }
}
```

- [ ] **Step 4: 수동 검증**

Run: `cargo run -- wiki ingest --content "borrowck test" --title "Borrow Test" 2>&1 | tail -8 && cargo run -- wiki lint 2>&1 | tail -8`
Expected: ingest-plan 출력 + lint 리포트 출력, exit 0

- [ ] **Step 5: Commit**

```bash
git add src/main.rs src/cli_output.rs
git commit -m "feat(cli): wiki ingest, query, lint subcommands"
```

### Task 7: 통합테스트 + 전체 게이트

**Files:**

- Create: `tests/wiki_karpathy.rs`
- Test:同文件

**Interfaces:**

- Consumes: Task 2~4의 공개 API.
- Produces: 왕복 회귀보호막. 이후 Task 없음.

- [ ] **Step 1: 실패 테스트 작성**

```rust
use std::sync::Arc;

#[test]
fn karpathy_roundtrip_ingest_query_lint_fileback() {
    let dir = std::env::temp_dir().join(format!("fva-wiki-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let embedder: Arc<dyn fva::embedding::Embedder> = Arc::new(fva::embedding::HashEmbedder::default());
    let store = fva::wiki::WikiStore::open(dir.join("wiki"), embedder).unwrap();

    // ingest → sources/ 보존 + plan
    let plan = store.ingest("Rust ownership: borrowing and lifetimes", Some("book/ch4"), Some("Ownership"), Some("concept")).unwrap();
    assert!(plan.source_slug.starts_with("sources/"));

    // fileback → index/log 반영
    store.write(&plan.suggested_slugs[0], "Ownership", "concept", &["rust".into()], &["book/ch4".into()], "Ownership notes, see [[concepts/borrowing]]").unwrap();
    assert!(std::fs::read_to_string(dir.join("wiki/index.md")).unwrap().contains(&plan.suggested_slugs[0]));

    // query → bundle에 fileback 포함
    let bundle = store.query_bundle("ownership borrowing", 10).unwrap();
    assert!(bundle.contains("Ownership"));

    // lint → 방금 쓴 페이지는 stale 아님, dead link 1건 보고
    let report = store.lint_report(9999).unwrap();
    assert!(report.contains("concepts/borrowing"));

    let _ = std::fs::remove_dir_all(&dir);
}
```

NOTE: `fva::embedding::HashEmbedder`, `fva::wiki::WikiStore`, `fva::embedding::Embedder` 이름은
`src/lib.rs`·`src/embedding/` 실제 export명에 맞춰 조정. `pub` 가시성이 모자라면 Task 3·4에서
`pub(crate)`→`pub`으로 승격 (이 플랜의 전제).

- [ ] **Step 2: 실패 확인**

Run: `cargo test --test wiki_karpathy 2>&1 | tail -10`
Expected: FAIL (파일 없음 → 생성 후에도 API 불일치 가능, 그 경우 Task 1~4로 돌아가 수정)

- [ ] **Step 3: 통과까지 최소수정 (Task 1~4 범위 내에서만)**

- [ ] **Step 4: 전체 게이트**

Run: `cargo test 2>&1 | tail -5 && cargo clippy 2>&1 | tail -3 && cargo fmt --check 2>&1 | tail -3`
Expected: 전부 PASS (네트워크 필요시 첫 실행만 허용)

- [ ] **Step 5: Commit**

```bash
git add tests/wiki_karpathy.rs
git commit -m "test(wiki): karpathy ingest-query-lint roundtrip"
```

### Task 8: 에이전트 문서 개정

**Files:**

- Modify: `skills/fva/SKILL.md`, `skills/fva/references/mcp-tools.md`

**Interfaces:**

- Consumes: Task 5의 3툴 설명.
- Produces: query-first 워크플로 문서. 코드 변경 없음.

- [ ] **Step 1: SKILL.md wiki 섹션 교체**

기존 "10. wiki_write … 14. wiki_delete" 뒤에 추가 (기존 5툴 설명 유지):

```markdown
15. `wiki_query` — Index-first 조회 (Karpathy-style, 기본값). `index.md`+시맨틱 Top-K+1-hop 이웃을 한 번에 반환. 태스크 시작시 `wiki_search` 대신 이것부터. 모르면 `wiki_read`로 드릴다운.
16. `wiki_ingest` — 소스 ingest. `content`+`source_uri`+`title_hint`+`area_hint` → `sources/*` 불변보존 + 최대 15개 손질 플랜 반환. 이후 `wiki_write`로 요약 확정 + `[[slug]]` 크로스링크.
17. `wiki_lint` — 주기적 위생점검 (고아/끊긴링크/stale/모순후보/빈틈). 수정은 직접.
    규칙: `sources/*` 수정금지, `index`/`log` 직접편집 금지, 좋은 답변은 `wiki_write`로 파일백.
```

- [ ] **Step 2: references/mcp-tools.md에 3툴 섹션 추가** (위와 동일 내용, 파라미터 표 포함)

- [ ] **Step 3: 문서 렌더 확인**

Run: `cargo build 2>&1 | tail -2`
Expected: OK (문서만이므로 빌드 무영향 확인용)

- [ ] **Step 4: Commit**

```bash
git add skills/fva/SKILL.md skills/fva/references/mcp-tools.md
git commit -m "docs: query-first wiki workflow (ingest, query, lint)"
```

## Self-Review

1. **Spec coverage:** §3 저장소→Task 1·2 (slug/frontmatter/index/log/migrate). §4 컴포넌트→Task 3·4·5 (ingest/query/lint + write확장). §5 플로우→Task 3·6·7 (plan→fileback→bundle→lint). §6 에러→Task 2·4 (불변/예약/재귀가드/warn/충돌접미사). §7 테스트→Task 7 + 매 Task 유닛. §8 CLI·문서→Task 6·8. 누락 없음.
2. **Placeholder scan:** "적절히", "TBD/TODO", "위와 동일 (반복 생략)" 없음 — Task 8의 mcp-tools 표는 "동일 내용, 파라미터 표 포함"이라 썼으나 실제 파라미터명은 Task 5 구조체에서 그대로 복사하도록 명시됨. Task 2·7의 embedder명·export명은 실측 조정 주석 포함 (허용된 명시적 분기).
3. **Type consistency:** `write(slug, title, entry_type, tags, sources, content)` 시그니처가 Task 2·5·6·7에서 동일. `IngestPlan{source_slug, related, neighbors, suggested_slugs}`가 Task 3·5·6에서 동일. `lint_report(i64)`, `query_bundle(&str, usize)` 동일.
