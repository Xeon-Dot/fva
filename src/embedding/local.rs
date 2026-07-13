//! Local hash-based embeddings (no API required).
//!
//! Uses feature hashing (similar to HashingVectorizer) for fast,
//! deterministic embeddings suitable for code semantic search.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::{Embedder, normalize};
use crate::error::Result;

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

        // Word-level features (FNV-1a hash instead of blake3)
        for token in lower.split(|c: char| !c.is_alphanumeric() && c != '_') {
            if token.len() < 2 {
                continue;
            }
            add_feature(&mut vec, token.as_bytes(), 1.0, dims);
        }

        // Character trigrams for typo tolerance — hash bytes directly,
        // avoiding String allocation per trigram.
        let bytes = lower.as_bytes();
        if bytes.len() >= 3 {
            for w in bytes.windows(3) {
                let h = {
                    let mut hasher = DefaultHasher::new();
                    w.hash(&mut hasher);
                    hasher.finish()
                };
                let idx = (h as usize) % dims;
                let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
                vec[idx] += sign * 0.5;
            }
        }

        // Identifier tokens (CamelCase / snake_case splits)
        for token in text.split(|c: char| !c.is_alphanumeric()) {
            if token.is_empty() {
                continue;
            }
            for_each_identifier_part(token, |part| {
                add_feature(&mut vec, part.to_ascii_lowercase().as_bytes(), 0.8, dims);
            });
        }

        normalize(&mut vec);
        vec
    }
}

/// Hash a feature and add its weighted contribution to the vector.
fn add_feature(vec: &mut [f32], feature: &[u8], weight: f32, dims: usize) {
    let mut hasher = DefaultHasher::new();
    feature.hash(&mut hasher);
    let h = hasher.finish();
    let idx = (h as usize) % dims;
    let sign = if h & 1 == 0 { 1.0 } else { -1.0 };
    vec[idx] += sign * weight;
}

/// Walk identifier parts without allocating a Vec<String>.
fn for_each_identifier_part<F>(s: &str, mut f: F)
where
    F: FnMut(&str),
{
    if s.is_empty() {
        f(s);
        return;
    }

    let char_indices: Vec<(usize, char)> = s.char_indices().collect();
    let len = char_indices.len();
    let mut part_start = 0usize;
    let mut prev_lower = false;

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

        if is_upper && prev_lower && i > part_start {
            emit(part_start, i);
            part_start = i;
        }
        if *ch == '_' {
            if i > part_start {
                emit(part_start, i);
            }
            part_start = i + 1;
            prev_lower = false;
            continue;
        }
        prev_lower = is_lower;
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
        assert!(sim_ab > sim_ac);
    }
}
