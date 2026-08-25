//! Phase 5 benchmark harness — measures FVA operations against performance targets.

mod report;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub use report::{BenchReport, BenchResult, TargetStatus};

use crate::engine::FvaEngine;
use crate::indexer::chunker::chunk_file;
use crate::indexer::parser::AstParser;

/// Performance targets from README (milliseconds).
pub const TARGETS: &[(&str, f64)] = &[
    ("find_files", 50.0),
    ("grep", 100.0),
    ("ast_chunk_single_file", 5.0),
    ("vector_search", 50.0),
    ("semantic_search", 200.0),
    ("hybrid_search", 200.0),
    ("get_call_graph", 50.0),
    ("get_smart_context", 300.0),
    ("full_index", 30_000.0),
];

#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub iterations: usize,
    pub warmup: usize,
    pub queries: Vec<String>,
    pub output: Option<PathBuf>,
    pub json: bool,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            iterations: 5,
            warmup: 2,
            queries: vec![
                "hybrid_search".into(),
                "Indexer".into(),
                "embed".into(),
                "config".into(),
            ],
            output: None,
            json: false,
        }
    }
}

/// Run the full benchmark suite against an initialized engine.
pub async fn run(engine: &Arc<FvaEngine>, opts: &BenchOptions) -> BenchReport {
    let started = Instant::now();
    let mut report = BenchReport::new(engine.root.display().to_string());
    let engine = Arc::clone(engine);

    // Ensure index is warm
    if engine.indexer.stats().indexed_files == 0 {
        tracing::info!("warming index for benchmark...");
        let _ = engine.indexer.index_all().await;
    }

    // Wait for FFF scan
    let _ = engine.fff.wait_for_scan(Duration::from_secs(120));

    let stats = engine.indexer.stats();
    let vectors = engine.vectors.stats();
    let graph = engine.graph.stats();
    let fff_files = engine.fff.total_files();

    report.corpus = Some(CorpusStats {
        fff_files,
        indexed_files: stats.indexed_files,
        total_chunks: stats.total_chunks,
        total_symbols: stats.total_symbols,
        total_vectors: vectors.total_vectors,
        graph_nodes: graph.nodes,
        graph_edges: graph.edges,
        embedder: engine.embedder.name().to_string(),
    });

    // --- FFF benchmarks ---
    report.results.push(
        bench_op(
            "find_files",
            opts,
            || {
                let engine = engine.clone();
                Box::pin(async move {
                    let _ = engine.fff.find_files("indexer", 0, 20).expect("find_files");
                })
            },
            target_ms("find_files"),
        )
        .await,
    );

    report.results.push(
        bench_op(
            "grep",
            opts,
            || {
                let engine = engine.clone();
                Box::pin(async move {
                    let _ = engine.fff.grep("Indexer", 0, 20).expect("grep");
                })
            },
            target_ms("grep"),
        )
        .await,
    );

    // --- AST chunk (single file) ---
    let sample_file = find_largest_rust_file(engine.root.as_path())
        .unwrap_or_else(|| engine.root.join("src/lib.rs"));
    report.results.push(bench_ast_chunk(opts, &sample_file));

    // --- Vector / query benchmarks ---
    for query in &opts.queries {
        let q = query.clone();

        report.results.push(
            bench_op(
                &format!("vector_search:{q}"),
                opts,
                || {
                    let q = q.clone();
                    let engine = engine.clone();
                    Box::pin(async move {
                        if let Ok(vec) = engine.embedder.embed_one(&q) {
                            let _ = engine
                                .vectors
                                .search(&vec, 20)
                                .await
                                .expect("vector search");
                        }
                    })
                },
                target_ms("vector_search"),
            )
            .await,
        );

        report.results.push(
            bench_op(
                &format!("semantic_search:{q}"),
                opts,
                || {
                    let q = q.clone();
                    let engine = engine.clone();
                    Box::pin(async move {
                        let _ = engine.query.semantic_search(&q, 10).await;
                    })
                },
                target_ms("semantic_search"),
            )
            .await,
        );

        report.results.push(
            bench_op(
                &format!("hybrid_search:{q}"),
                opts,
                || {
                    let q = q.clone();
                    let engine = engine.clone();
                    Box::pin(async move {
                        let _ = engine.query.hybrid_search(&q, 10).await;
                    })
                },
                target_ms("hybrid_search"),
            )
            .await,
        );
    }

    // --- Graph / context ---
    let symbol = engine
        .indexer
        .store()
        .all_chunks()
        .first()
        .map(|c| c.symbol_name.clone())
        .unwrap_or_else(|| "main".into());

    report.results.push(
        bench_op(
            "get_call_graph",
            opts,
            || {
                let symbol = symbol.clone();
                let engine = engine.clone();
                Box::pin(async move {
                    let _ = engine.graph.callers(&symbol, 1);
                    let _ = engine.graph.callees(&symbol, 1);
                })
            },
            target_ms("get_call_graph"),
        )
        .await,
    );

    let query = opts
        .queries
        .first()
        .cloned()
        .unwrap_or_else(|| "main".into());
    report.results.push(
        bench_op(
            "get_smart_context",
            opts,
            || {
                let query = query.clone();
                let engine = engine.clone();
                Box::pin(async move {
                    let result = engine.query.hybrid_search(&query, 5).await;
                    let _ = engine.context.build(&query, None, &result);
                })
            },
            target_ms("get_smart_context"),
        )
        .await,
    );

    // --- Full re-index (cold hash bypass via temp re-chunk) ---
    report.results.push(bench_full_index(&engine, opts).await);

    report.duration_total_ms = started.elapsed().as_secs_f64() * 1000.0;
    report
}

async fn bench_op<F>(
    name: &str,
    opts: &BenchOptions,
    mut f: F,
    target_ms: Option<f64>,
) -> BenchResult
where
    F: FnMut() -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>,
{
    for _ in 0..opts.warmup {
        f().await;
    }

    let mut samples = Vec::with_capacity(opts.iterations);
    for _ in 0..opts.iterations {
        let start = Instant::now();
        f().await;
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    BenchResult::from_samples(name, &samples, target_ms)
}

fn bench_ast_chunk(opts: &BenchOptions, file: &Path) -> BenchResult {
    let source = std::fs::read_to_string(file).unwrap_or_default();
    let relative = file
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "sample.rs".into());

    let parser = AstParser::new();
    for _ in 0..opts.warmup {
        let _ = chunk_file(&parser, file, &relative, &source, 10 * 1024 * 1024);
    }

    let mut samples = Vec::with_capacity(opts.iterations);
    for _ in 0..opts.iterations {
        let start = Instant::now();
        let _ = chunk_file(&parser, file, &relative, &source, 10 * 1024 * 1024);
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }

    let mut result = BenchResult::from_samples(
        &format!("ast_chunk_single_file:{}", file.display()),
        &samples,
        target_ms("ast_chunk_single_file"),
    );
    result.note = Some(format!("{} lines", source.lines().count()));
    result
}

async fn bench_full_index(engine: &Arc<FvaEngine>, _opts: &BenchOptions) -> BenchResult {
    let files = engine.indexer.collect_files().unwrap_or_default();
    let file_count = files.len();

    // Force cold re-index for accurate timing
    engine.indexer.store().invalidate_hashes();

    let start = Instant::now();
    let _ = engine.indexer.index_all().await;
    let elapsed = start.elapsed().as_secs_f64() * 1000.0;

    let mut result = BenchResult::from_samples("full_index", &[elapsed], target_ms("full_index"));
    result.note = Some(format!("{file_count} source files"));
    result.iterations = 1;
    result
}

fn target_ms(name: &str) -> Option<f64> {
    TARGETS.iter().find(|(k, _)| *k == name).map(|(_, v)| *v)
}

fn find_largest_rust_file(root: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    let src = root.join("src");
    if !src.exists() {
        return None;
    }
    for entry in walkdir_simple(&src) {
        if entry.extension().is_some_and(|e| e == "rs")
            && let Ok(meta) = entry.metadata()
        {
            let size = meta.len();
            if best.as_ref().is_none_or(|(s, _)| size > *s) {
                best = Some((size, entry));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn walkdir_simple(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CorpusStats {
    pub fff_files: usize,
    pub indexed_files: usize,
    pub total_chunks: usize,
    pub total_symbols: usize,
    pub total_vectors: usize,
    pub graph_nodes: usize,
    pub graph_edges: usize,
    pub embedder: String,
}

/// Write report to path and print summary table.
pub fn emit(report: &BenchReport, opts: &BenchOptions) {
    if opts.json {
        println!(
            "{}",
            serde_json::to_string_pretty(report).unwrap_or_default()
        );
    } else {
        print_table(report);
    }

    if let Some(path) = &opts.output {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json_path = if path.extension().is_some_and(|e| e == "json") {
            path.clone()
        } else {
            path.join("latest.json")
        };
        if let Ok(json) = serde_json::to_string_pretty(report) {
            let _ = std::fs::write(&json_path, json);
            if !opts.json {
                eprintln!("\nReport saved: {}", json_path.display());
            }
        }
    }
}

fn print_table(report: &BenchReport) {
    use console::style;

    const W: usize = 68;
    println!("\n+{:-<W$}+", "", W = W);
    println!(
        "|  {} {}",
        style("FVA Benchmark -").bold().cyan(),
        style(truncate_str(&report.repo, 50)).white()
    );
    println!("+{:-<W$}+", "", W = W);

    if let Some(c) = &report.corpus {
        println!(
            "|  Corpus: {} FFF | {} chunks | {} vectors | {} edges",
            style(c.fff_files).yellow(),
            style(c.total_chunks).yellow(),
            style(c.total_vectors).yellow(),
            style(c.graph_edges).yellow()
        );
        println!(
            "|  Embedder: {} | Indexed: {} files",
            style(&c.embedder).green(),
            style(c.indexed_files).yellow()
        );
        println!("+{:-<W$}+", "", W = W);
    }

    println!(
        "|  {:<32} {:>8} {:>8} {:>6} |",
        "Operation", "p50 ms", "p95 ms", "Status"
    );
    println!("+{:-<W$}+", "", W = W);

    for r in &report.results {
        let status = match r.status {
            TargetStatus::Pass => style("PASS").green().bold().to_string(),
            TargetStatus::Fail => style("FAIL").red().bold().to_string(),
            TargetStatus::NoTarget => style("  - ").dim().to_string(),
            TargetStatus::Warn => style("WARN").yellow().bold().to_string(),
        };
        println!(
            "|  {:<32} {:>8.2} {:>8.2} {:>6} |",
            truncate_str(&r.name, 32),
            r.p50_ms,
            r.p95_ms,
            status
        );
    }

    let pass = report
        .results
        .iter()
        .filter(|r| r.status == TargetStatus::Pass)
        .count();
    let fail = report
        .results
        .iter()
        .filter(|r| r.status == TargetStatus::Fail)
        .count();
    let total = report.results.len();

    println!("+{:-<W$}+", "", W = W);
    println!(
        "|  Total: {:.0}ms | {}/{} passed | {} failed",
        report.duration_total_ms,
        style(pass).green(),
        total,
        if fail > 0 {
            style(fail).red().to_string()
        } else {
            style(fail).to_string()
        }
    );
    println!("+{:-<W$}+", "", W = W);
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!(
            "{}...",
            s.chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    }
}
