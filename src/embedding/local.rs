//! Local hash-based embeddings (no API required).
//!
//! Uses multi-hash feature hashing with n-gram subword features, TF weighting,
//! digit-boundary token splitting, and code structure-awareness for improved
//! semantic code similarity.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::{Embedder, normalize};
use crate::error::Result;

/// Salt prefixes for multi-hash. Each feature is hashed with every salt,
/// producing multiple independent bucket positions. This reduces the impact of
/// accidental collisions compared to single-hash feature hashing.
const HASH_SALTS: &[&[u8]] = &[b"h0:", b"h1:"];

pub struct LocalEmbedder {
    dimensions: usize,
}

impl LocalEmbedder {
    pub fn new(dimensions: usize) -> Self {
        Self {
            dimensions: dimensions.max(64),
        }
    }

    fn hash_embed(&self, text: &str) -> Vec<f32> {
        let dims = self.dimensions;
        let mut vec = vec![0.0f32; dims];
        let lower = text.to_lowercase();
        let bytes = lower.as_bytes();

        // ── 1. Word-level features with TF weighting ──────────────────────
        // Collect token frequencies, splitting each raw token at digit
        // boundaries so "parse2json" contributes ["parse", "2", "json"].
        let mut token_counts: std::collections::HashMap<Vec<u8>, usize> =
            std::collections::HashMap::new();

        for token in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if token.is_empty() {
                continue;
            }
            let mut any_emitted = false;
            split_at_digit_boundaries(token, |part| {
                // Skip single non-digit characters (rarely meaningful).
                if part.len() < 2 && !part.bytes().all(|b| b.is_ascii_digit()) {
                    return;
                }
                *token_counts.entry(part.as_bytes().to_vec()).or_insert(0) += 1;
                any_emitted = true;
            });
            // If digit-boundary splitting didn't produce sub-tokens and the
            // token is long enough, emit the whole token.
            if !any_emitted && token.len() >= 2 {
                *token_counts.entry(token.as_bytes().to_vec()).or_insert(0) += 1;
            }
        }

        // Add word tokens with TF weighting — sqrt(count) so more frequent
        // tokens get more weight, but sub-linearly.
        for (token_bytes, count) in &token_counts {
            let weight = (*count as f32).sqrt();
            add_feature_multi(&mut vec, token_bytes, weight, dims);
        }

        // ── 2. Character n-grams for subword / typo resilience ────────────
        // Longer n-grams get proportionally higher weight because they carry
        // more specific information.
        let len = bytes.len();

        if len >= 2 {
            for w in bytes.windows(2) {
                add_feature_multi(&mut vec, w, 0.25, dims);
            }
        }
        if len >= 3 {
            for w in bytes.windows(3) {
                add_feature_multi(&mut vec, w, 0.5, dims);
            }
        }
        if len >= 4 {
            for w in bytes.windows(4) {
                add_feature_multi(&mut vec, w, 0.75, dims);
            }
        }

        // ── 3. Identifier decomposition (CamelCase / snake_case) ──────────
        // Process the original-case text so casing information is preserved
        // for the decomposition boundaries.
        for token in text.split(|c: char| !c.is_alphanumeric()) {
            if token.is_empty() {
                continue;
            }
            for_each_identifier_part(token, |part| {
                if part.len() < 2 {
                    return;
                }
                add_feature_multi(&mut vec, part.to_ascii_lowercase().as_bytes(), 0.5, dims);
            });
        }

        // ── 4. Exact-match boost for cased identifiers ────────────────────
        // Identifiers containing uppercase letters or digits are added with
        // their original casing.  This helps distinguish e.g.  `FooBar` from
        // `foobar` and gives a similarity boost to exact literal matches.
        for token in text.split(|c: char| !c.is_alphanumeric()) {
            if token.len() < 3 {
                continue;
            }
            let has_upper = token.bytes().any(|b| b.is_ascii_uppercase());
            let has_digit = token.bytes().any(|b| b.is_ascii_digit());
            if has_upper || has_digit {
                add_feature_multi(&mut vec, token.as_bytes(), 0.3, dims);
            }
        }

        // ── 5. Structure-aware markers ────────────────────────────────────
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Function / class / struct / trait / enum / impl definitions
            if trimmed.starts_with("fn ")
                || trimmed.starts_with("def ")
                || trimmed.starts_with("func ")
                || trimmed.starts_with("class ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("impl ")
                || trimmed.starts_with("trait ")
                || trimmed.starts_with("enum ")
            {
                add_feature_multi(&mut vec, b"__def__", 0.5, dims);
            }
            // Return-type / lambda arrows
            if trimmed.contains("=>") || trimmed.contains("->") {
                add_feature_multi(&mut vec, b"__arrow__", 0.35, dims);
            }
            // Comment lines
            if trimmed.starts_with("//")
                || trimmed.starts_with("#")
                || trimmed.starts_with("/*")
            {
                add_feature_multi(&mut vec, b"__comment__", 0.25, dims);
            }
        }

        normalize(&mut vec);
        vec
    }
}

/// Split a string at digit─non-digit boundaries.
///
/// Example: `"parse2json"` yields `["parse", "2", "json"]`.
///
/// The split positions are always on UTF-8 character boundaries because ASCII
/// digits are single-byte and every adjacent non-digit byte starts a valid
/// character.
fn split_at_digit_boundaries<'a>(s: &'a str, mut f: impl FnMut(&'a str)) {
    if s.is_empty() {
        return;
    }
    let bytes = s.as_bytes();
    let mut start = 0;
    let mut prev_is_digit = bytes[0].is_ascii_digit();

    for i in 1..bytes.len() {
        let cur_is_digit = bytes[i].is_ascii_digit();
        if cur_is_digit != prev_is_digit {
            debug_assert!(i > start);
            f(&s[start..i]);
            start = i;
            prev_is_digit = cur_is_digit;
        }
    }
    if start < s.len() {
        f(&s[start..]);
    }
}

/// Hash a feature into multiple buckets (one per `HASH_SALTS` entry) and add
/// its signed contribution to the vector.  The sign (positive or negative) is
/// derived from the hash, which keeps the distribution centred around zero.
fn add_feature_multi(vec: &mut [f32], feature: &[u8], weight: f32, dims: usize) {
    for salt in HASH_SALTS {
        let mut hasher = DefaultHasher::new();
        hasher.write(salt);
        hasher.write(feature);
        let h = hasher.finish();
        let idx = (h as usize) % dims;
        let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
        vec[idx] += sign * weight;
    }
}

/// Walk identifier parts without allocating a `Vec<String>`.
///
/// Splits on CamelCase boundaries, underscore separators, and digit
/// transitions, so any combination of these naming conventions is decomposed
/// into its atomic pieces.
fn for_each_identifier_part<F>(s: &str, mut f: F)
where
    F: FnMut(&str),
{
    if s.is_empty() {
        return;
    }

    let char_indices: Vec<(usize, char)> = s.char_indices().collect();
    let len = char_indices.len();
    let mut part_start = 0usize;
    let mut prev_lower = false;
    let mut prev_digit = char_indices[0].1.is_ascii_digit();

    let mut emit = |part_start: usize, part_end: usize| {
        if part_end <= part_start {
            return;
        }
        let byte_start = char_indices[part_start].0;
        let byte_end = if part_end < len {
            char_indices[part_end].0
        } else {
            s.len()
        };
        f(&s[byte_start..byte_end]);
    };

    for (i, (_, ch)) in char_indices.iter().enumerate() {
        let is_upper = ch.is_uppercase();
        let is_lower = ch.is_lowercase();
        let is_digit = ch.is_ascii_digit();

        // CamelCase boundary: "fooBar" -> split before "B"
        if is_upper && prev_lower && i > part_start {
            emit(part_start, i);
            part_start = i;
        }
        // Digit boundary: "foo2bar" -> split before "2" and before "bar"
        if is_digit != prev_digit && i > part_start {
            emit(part_start, i);
            part_start = i;
        }
        if *ch == '_' {
            if i > part_start {
                emit(part_start, i);
            }
            part_start = i + 1;
            prev_lower = false;
            prev_digit = false;
            continue;
        }
        prev_lower = is_lower;
        prev_digit = is_digit;
    }
    if part_start < len {
        emit(part_start, len);
    }
}

impl Embedder for LocalEmbedder {
    fn name(&self) -> &str {
        "local-hash"
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.hash_embed(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_produces_normalized_vector() {
        let e = LocalEmbedder::new(128);
        let v = e
            .embed_one("fn hello_world() { println!(\"hi\"); }")
            .unwrap();
        assert_eq!(v.len(), 128);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn embed_does_not_panic_on_utf8_identifier_tokens() {
        let e = LocalEmbedder::new(128);
        let v = e.embed_one("注释：对函数 fooBar_baz 做说明").unwrap();
        assert_eq!(v.len(), 128);
    }

    #[test]
    fn similar_code_has_higher_similarity() {
        let e = LocalEmbedder::new(256);
        let a = e
            .embed_one("fn authenticate_user(token: &str) -> Result<User>")
            .unwrap();
        let b = e
            .embed_one("fn authenticate_user(session: &str) -> Result<User>")
            .unwrap();
        let c = e
            .embed_one("fn render_html_template(page: &str) -> String")
            .unwrap();
        let sim_ab = super::super::cosine_similarity(&a, &b);
        let sim_ac = super::super::cosine_similarity(&a, &c);
        assert!(
            sim_ab > sim_ac,
            "expected sim_ab ({}) > sim_ac ({})",
            sim_ab,
            sim_ac,
        );
    }

    #[test]
    fn zero_length_input_produces_zero_vector() {
        let e = LocalEmbedder::new(128);
        let v = e.embed_one("").unwrap();
        assert_eq!(v.len(), 128);
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(norm < 1e-6);
    }

    #[test]
    fn digit_boundary_splitting_works() {
        let e = LocalEmbedder::new(128);
        let a = e.embed_one("parse2json").unwrap();
        let b = e.embed_one("parse_to_json").unwrap();
        let c = e.embed_one("render_html").unwrap();
        let sim_ab = super::super::cosine_similarity(&a, &b);
        let sim_ac = super::super::cosine_similarity(&a, &c);
        assert!(
            sim_ab > sim_ac,
            "expected sim_ab ({}) > sim_ac ({})",
            sim_ab,
            sim_ac,
        );
    }

    #[test]
    fn structure_markers_boost_similarity() {
        let e = LocalEmbedder::new(128);
        let fn_a = e.embed_one("fn compute(input: i32) -> i32").unwrap();
        let fn_b = e.embed_one("fn compute(x: f64) -> f64").unwrap();
        let call = e.embed_one("let result = compute(42);").unwrap();
        let sim_fn = super::super::cosine_similarity(&fn_a, &fn_b);
        let sim_mixed = super::super::cosine_similarity(&fn_a, &call);
        assert!(
            sim_fn > sim_mixed,
            "expected sim_fn ({}) > sim_mixed ({})",
            sim_fn,
            sim_mixed,
        );
    }

    #[test]
    fn repeated_tokens_get_higher_tf_weight() {
        let e = LocalEmbedder::new(256);
        let repeated = e
            .embed_one("data data data process process")
            .unwrap();
        let single = e
            .embed_one("data process query result")
            .unwrap();
        let different = e
            .embed_one("render parse execute compute")
            .unwrap();
        let sim_rs = super::super::cosine_similarity(&repeated, &single);
        let sim_rd = super::super::cosine_similarity(&repeated, &different);
        assert!(
            sim_rs > sim_rd,
            "expected sim_rs ({}) > sim_rd ({})",
            sim_rs,
            sim_rd,
        );
    }
}
