//! Benchmark report types and statistics.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetStatus {
    Pass,
    Fail,
    Warn,
    NoTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub name: String,
    pub iterations: usize,
    pub min_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub target_ms: Option<f64>,
    pub status: TargetStatus,
    pub note: Option<String>,
}

impl BenchResult {
    pub fn from_samples(name: &str, samples: &[f64], target_ms: Option<f64>) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let n = sorted.len().max(1);
        let min_ms = sorted.first().copied().unwrap_or(0.0);
        let max_ms = sorted.last().copied().unwrap_or(0.0);
        let mean_ms = sorted.iter().sum::<f64>() / n as f64;
        let p50_ms = percentile(&sorted, 0.50);
        let p95_ms = percentile(&sorted, 0.95);

        let status = match target_ms {
            None => TargetStatus::NoTarget,
            Some(t) if p95_ms <= t => TargetStatus::Pass,
            Some(t) if p50_ms <= t => TargetStatus::Warn,
            Some(_) => TargetStatus::Fail,
        };

        Self {
            name: name.to_string(),
            iterations: samples.len(),
            min_ms,
            max_ms,
            mean_ms,
            p50_ms,
            p95_ms,
            target_ms,
            status,
            note: None,
        }
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchReport {
    pub version: String,
    pub timestamp: String,
    pub repo: String,
    pub corpus: Option<super::CorpusStats>,
    pub results: Vec<BenchResult>,
    pub duration_total_ms: f64,
}

impl BenchReport {
    pub fn new(repo: String) -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_else(|_| "0".to_string()),
            repo,
            corpus: None,
            results: Vec::new(),
            duration_total_ms: 0.0,
        }
    }
}
