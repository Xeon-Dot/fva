---
name: audit-fixes-2025
description: Audit fixes applied: removed 6 deps, ~170 lines, GraphDelta, fast_lower, hash_bytes, batch_size
metadata:
  type: reference
---

fva audit findings applied:
- Removed 6 unused Cargo deps: heed, memmap2, ahash, globset, notify, notify-debouncer-full (all transitive through fff-search)
- Removed GraphDelta + delta log system from CallGraphStore (always redundant with full snapshot persist)
- Removed FvaEngine::persist() (double-persist — index_all already calls it)
- Replaced hand-rolled fast_lower() with str::to_ascii_lowercase()
- Replaced hand-rolled FNV-1a hash_bytes() with std::hash::DefaultHasher
- Removed unused EmbeddingConfig::batch_size field
- Fixed VectorConfig field name the_model → backend
- Trimmed keyword blacklist in graph::builder from 30 to 18 entries
- Inlined chrono_like_timestamp() in bench/report.rs
- Kept with_picker() helper in fff/mod.rs (legitimate abstraction)
- Kept embed_one() as both trait method + free function

**Why:** Code bloat reduction, dead code removal, stdlib reuse.
**How to apply:** Already applied to working tree — compile clean, tests pass.
