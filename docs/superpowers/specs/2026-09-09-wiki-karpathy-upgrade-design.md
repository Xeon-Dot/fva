# FVA Wiki Karpathy 업그레이드 — 설계

- 일자: 2026-09-09
- 원본: Karpathy "How I use LLMs, Wiki" gist (2026-04-04)
- 상태: 승인됨 (섹션 1~5 섹션별 승인)

## 1. 배경과 목표

현행 FVA wiki는 `.fva/wiki/*.md` 평면 CRUD + `wiki_vectors.bin` 시맨틱 검색(`wiki_write/read/search/list/delete`)이다.
이를 Karpathy식 compounding knowledge base로 승격한다: ingest 때 지식을 컴파일해 누적하고,
query는 `index.md`→개별 페이지로 드릴다운하며, 좋은 답변은 다시 파일링하고, lint로 드리프트를 잡는다.

핵심 테제(원본): "RAG는 쿼리시점에 재발견하지만, Wiki는 ingest시점에 컴파일한다."

## 2. 확정 결정

- 범위: 둘 다 — 코드 메모리(ADR/아키텍처/gotcha) + 범용 지식(리서치/독서/웹클립).
- 호환: **구 wiki 호환 전부 드랍 (브레이킹 허용).**
- 깊이: **A. 오케스트레이티드 네이티브** — Rust는 기계적 부분만 결정적으로 수행하고,
  요약·크로스레프 판단은 호출자(LLM 에이전트)에 위임. Rust에 LLM이 없으므로
  "1콜 자동요약" (B 추출식=오염 위험, C LLM내장=스코프 폭발)은 채택하지 않음.

## 3. 저장소·아키텍처

- 레이아웃: `.fva/wiki/{index.md, log.md, sources/, entities/, concepts/, analyses/, adrs/, arch/, gotchas-/}`.
- slug = 확장자를 뺀 상대경로 (예 `concepts/foo`).
- `validate_slug` 개정: `/` 허용, `..`·`\`·`\0`·빈 세그먼트는 계속 거부.
- frontmatter 신스키마: `title/type/tags/sources/created/updated`.
  `type` ∈ {source, entity, concept, analysis, adr, arch, gotcha, index, log}.
  `sources` = 쉼표구분 원천 경로·URL.
- `open()`시 1회 `migrate_legacy`: 루트 구 `*.md`(`index`/`log` 제외)를
  `concepts/legacy-<slug>.md`로 이동하며 `type: concept` 부여 후 벡터 스냅샷 파기·리빌드.
  이후 루트 평면파일은 lint 경고 대상.
- `index.md` (영역별 카탈로그: `- [[slug]] — 한줄요약`)와 `log.md`
  (`## [YYYY-MM-DD] <op> | <제목> (<slug>)` append-only)는 `wiki_write`/`delete` 성공 시
  Rust가 자동 재생성/append한다. 쓸 대상이 index/log 자신일 때는 재귀가드.
- 벡터 인덱스(`wiki_vectors.bin`)에서 `index`/`log` 제외 (검색오염 방지).
  `wiki_query`가 둘을 별도 채널로 반환한다.
- `[[slug]]` 링크그래프는 별도 persist 없이 주문형 스캔
  (`\[\[([^\]]+)\]\]` regex, wiki-scale <1k라 O(n) 허용, `ponytail:` 표기).
- `sources/*`는 Rust가 write-once 강제 (존재하면 `wiki_write` 거부, 새 slug만 허용).
  그 외 불변 규칙(`index`/`log` 직접편집 금지)은 SKILL.md 컨벤션으로.

## 4. 컴포넌트 (신규 MCP 3툴 + 기존 변경)

- `wiki_ingest{content*, source_uri?, title_hint?, area_hint?}`
  ① `sources/<slug>.md` (type=source) 저장 ② 임베드+벡터 Top-K(기본 10) 관련페이지
  ③ 1-hop `[[ ]]` 이웃확장 ④ ingest-plan 반환 (추천 신규/갱신 slug 최대 15,
  크로스링크 제안, 관련 발췌). 요약본은 만들지 않음 — 에이전트가 `wiki_write`로 확정.
  slugify 규칙: 소문자화→공백/`_`→`-`→`[^a-z0-9-]` 제거→연속 `-` 압축→앞뒤 `-` 제거→
  빈 결과면 `untitled`, 충돌이면 `-2`, `-3` 접미사. `area_hint` ∈
  {entity, concept, analysis, adr, arch, gotcha} 중 1, 생략시 `concept`
  (호출자가 코드결정·구조·실패기록임을 명시하면 각각 adr/arch/gotcha 권장).
- `wiki_query{query, maxResults?}` (읽기전용)
  `index.md` 전문 + 시맨틱 Top-K (preview) + Top히트 1-hop 이웃을 한 번에 반환.
  본문 풀텍스트는 상위 3개까지, 나머지는 slug+preview (토큰예산).
- `wiki_lint{stale_days=180}` (읽기전용, 수정 안 함) 5종 리포트:
  ①고아 (inbound 0, index/log/sources 제외) ②끊긴링크 (타깃 없음)
  ③stale (updated 경과, sources/index/log 제외) ④모순후보 (코사인 상위쌍 +
  반전 키워드 `not/no/never/deprecated/instead` 동시포함) ⑤빈틈 (엔트리 3개 미만 영역).
- `WikiStore` 변경: `write()`에 `type`/`sources` 추가 (브레이킹 OK),
  `sources/*` write-once 가드, index 재생성+log append (재귀가드), 벡터에서 index/log 제외,
  신규 헬퍼 `extract_wikilinks`/`render_index`/`append_log`/`migrate_legacy`.
  기존 `read/delete/search/list`는 slug 경로 허용 + `type` 필터 추가만.
- `server.rs`에 3툴, CLI `fva wiki ingest|query|lint` 3 서브커맨드.
  기존 `search`는 저수준 툴로 유지하고 SKILL.md는 query-first로 유도.

## 5. 데이터플로우

- ingest: `wiki_ingest(content, source_uri?)` → slugify → `sources/*` 저장 →
  embed+Top10+1hop → plan 반환 → 에이전트 `wiki_write` 1~15회 →
  각 write마다 벡터 재인덱스+index 재생성+log append.
  로그: ingest시 2줄 — 내부 `write()`가 `## [날짜] write | 제목 (sources/slug)`를,
  이어서 `ingest()`가 `## [날짜] ingest | 제목 (sources/slug)`를 append
  (구현 `src/wiki/mod.rs:368-369`, Task 7 고정 동작 — 본 §의 기존 "1줄" 기술 정정),
  후속 write마다 `## [날짜] write | slug`.
- query: 읽기전용. 좋은 답변은 에이전트가 `wiki_write`로 파일백 →
  `## [날짜] fileback | slug`.
- lint: 읽기전용 전체스캔 (전 `.md` 파싱→링크추출→나이계산→저장벡터 쌍별 코사인) →
  5종 리포트. 수정은 `wiki_write`/`delete`로, 각 log에 `lint-fix` 기록.
- index 재생성: index/log 제외 전 엔트리 수집 → 영역별 그룹 →
  `- [[slug]] — 제목 (첫 비어있지 않은 줄 100자 한줄요약)`, 그룹 내 updated 내림차순.

## 6. 에러핸들링

- `sources/*` 덮어쓰기 → `FvaError::Wiki("source immutable")` 거부.
- 예약 slug (`index`/`log`) 직접 `wiki_write` → 거부 (내부함수만 갱신).
- `[[ ]]` 타깃 없음 → write시 거부하지 않고 `lint`로 보고 (쓰기흐름 보호).
- index/log 재생성 실패 → 원본 write는 성공 처리 + `tracing::warn` (카탈로그 때문에 지식손실 금지).
- `migrate_legacy` 충돌 (`legacy-*` 이미 있음) → 숫자 접미사로 건너뜀, 실패해도 `open()` 계속.

## 7. 테스팅

- `src/wiki/mod.rs` 유닛: 신스키마 왕복, slug 검증 (서브디렉 허용/`..` 거부),
  링크추출, index 렌더, log append, sources write-once, legacy 이주.
- 통합 `tests/wiki_karpathy.rs` (신규 1): MCP 3툴 왕복
  (ingest→query→lint→fileback) 1개 + `cargo test wiki`.
- wiki 벡터는 별도 파일이라 코드청크 LanceDB 차원불일치 재생성 규칙과 무관.

## 8. CLI·문서·범위

- CLI: `fva wiki ingest (--content|--file) [--title] [--source-uri] [--area-hint]` /
  `fva wiki query <query>` / `fva wiki lint [--stale-days 180]` 신규.
  `write`에 `--type --sources`, `list`/`search`에 `--type` 필터 추가.
  `cli_output.rs`에 ingest-plan / query-bundle / lint-report 출력 3종 추가.
- 문서: `skills/fva/SKILL.md` + `references/mcp-tools.md`를 query-first로 개정
  (태스크 시작 `wiki_query`, 발견시 `wiki_ingest`→`wiki_write` 파일백, 주기적 `wiki_lint`).
- 손댈 파일 (6+2): `src/wiki/mod.rs`, `src/mcp/server.rs`, `src/main.rs`,
  `src/cli_output.rs`, `skills/fva/SKILL.md`, `skills/fva/references/mcp-tools.md` +
  `tests/wiki_karpathy.rs` (신규) + 본 문서. `engine.rs`·벡터코어는 불변.
- 논외: URL 직접 fetch, LLM 내장요약, lint 자동수정, Obsidian 플러그인/Marp/Dataview,
  qmd 패리티, 구 slug 별칭 지원.
