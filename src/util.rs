//! Shared utility functions.

use crate::error::{FvaError, Result};

/// Rough token estimate (4 chars ≈ 1 token).
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4 + 1
}

/// Trait for items that have a numeric score.
pub trait HasScore {
    fn score(&self) -> f32;
}

/// Sort hits by score descending.
pub fn sort_by_score<T: HasScore>(hits: &mut [T]) {
    hits.sort_by(|a, b| {
        b.score()
            .partial_cmp(&a.score())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
}

/// Build a reqwest blocking client with standard timeout and user-agent.
pub fn http_client(user_agent: &str) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| FvaError::Other(format!("http client: {e}")))
}

/// Resolve pagination params into (limit, offset).
pub fn resolve_pagination(
    max_results: Option<f64>,
    offset: Option<f64>,
    default: usize,
) -> (usize, usize) {
    let limit = match max_results {
        None => default,
        Some(v) if v <= 0.0 || !v.is_finite() => default,
        Some(v) => (v.round() as usize).max(1),
    };
    let offset = offset.map(|v| v.max(0.0) as usize).unwrap_or(0);
    (limit, offset)
}
