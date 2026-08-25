//! Shared utility functions.

use crate::error::{FvaError, Result};

/// Rough token estimate (4 chars ≈ 1 token).
pub fn estimate_tokens(text: &str) -> usize {
    text.len() / 4 + 1
}

/// Parse a comma-separated tag string into trimmed, non-empty tags.
pub fn parse_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Truncate content to a UTF-8-safe preview with an ellipsis.
pub fn truncate_preview(content: &str, max_len: usize) -> String {
    if content.len() <= max_len {
        return content.to_string();
    }
    let mut end = max_len.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &content[..end])
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
