---
name: vector-db-improvements
description: "벡터 DB 검색 성능 및 임베딩 품질 개선 내역"
metadata:
  type: reference
---

## 개선 사항 요약

### 1. 임베딩 품질 개선 (`src/embedding/local.rs`)
- **멀티해시 (2-salts)**: 각 feature를 2개 버킷에 해싱하여 collision noise 감소
- **TF 가중치**: 문서 내 토큰 빈도 sqrt() 스케일링 적용
- **N-gram 계층**: 2-gram(0.25) → 3-gram(0.5) → 4-gram(0.75) 가중치
- **숫자 경계 분할**: "parse2json" → "parse", "json" 분할
- **케이스 부스트**: 대문자/숫자 포함 식별자 원래 케이스로 유지
- **구조 마커**: `__def__`, `__arrow__`, `__comment__` 등 코드 구조 인식

### 2. 검색 속도 개선 (`src/vector/flat.rs`)
- **Rayon 병렬 검색**: CPU 코어 수만큼 병렬로 cosine similarity 계산
- **토큰 inverted index**: upsert 시 청크 메타데이터 기반 해시 인덱스 구축

### 3. 청크 조회 최적화 (`src/indexer/store.rs`)
- **`chunks_by_id` HashMap 추가**: O(n) → O(1) ID 기반 청크 조회
- **`search_chunks` 심볼 인덱스 우선 검색** + limit 파라미터

### 4. 임베딩 텍스트 개선 (`src/vector/mod.rs`)
- "symbol_kind symbol_name in path" 헤딩 + signature line 강조

## 검증 결과
- **1300 vectors 검색**: 0.30ms → 0.07ms (4.3x faster)
- **25개 테스트 전부 통과**

**Why**: 기존 hash 기반 임베딩이 단순 feature hashing에 그쳐 의미적 유사성을 캡처하지 못했고, FlatVectorStore의 sequential scan이 병목이었다. `find_chunk`의 O(n) 순회도 hybrid search 지연을 유발했다.

**How to apply**: 추가 config 변경 불필요 (기존 API 완전 호환).
