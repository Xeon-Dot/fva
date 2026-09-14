use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};
use rmcp::{ServiceExt, transport::stdio};
use tracing_subscriber::{EnvFilter, fmt};

use fva::cli_output;
use fva::config::Config;
use fva::engine::FvaEngine;
use fva::mcp::FvaServer;
use fva::util::parse_tags;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// FVA — FFF · Vector · AST: hybrid codebase intelligence for AI coding agents.
#[derive(Parser)]
#[command(name = "fva", version, about, long_about = None)]
struct Cli {
    /// Project root to index.
    #[arg(short, long, global = true, value_name = "PATH")]
    path: Option<String>,

    /// Config file path.
    #[arg(short, long, global = true, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Log level override.
    #[arg(long, global = true, env = "RUST_LOG")]
    log_level: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start MCP server on stdio (default).
    Serve,
    /// Run full index (AST + vectors + call graph) and exit.
    Index,
    /// Print indexing status.
    Status,
    /// Hybrid search from CLI.
    Search {
        /// Search query.
        query: String,
        /// Max results.
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// Wiki knowledge base — write, read, search, list entries.
    Wiki {
        #[command(subcommand)]
        command: WikiCommands,
    },
    /// Run performance benchmarks (Phase 5).
    #[cfg(feature = "bench")]
    Bench {
        /// Benchmark iterations per operation.
        #[arg(short, long, default_value_t = 5)]
        iterations: usize,
        /// Warmup iterations (discarded).
        #[arg(short, long, default_value_t = 2)]
        warmup: usize,
        /// Write JSON report to path (default: .fva/benchmarks/latest.json).
        #[arg(long, value_name = "PATH")]
        output: Option<PathBuf>,
        /// Output JSON to stdout instead of table.
        #[arg(long)]
        json: bool,
    },
    /// Print version info.
    Version,
    /// Upgrade FVA to the latest release.
    #[command(alias = "update")]
    Upgrade {
        /// Install a specific release tag (e.g. v0.2.0) instead of latest.
        #[arg(long, value_name = "TAG")]
        version: Option<String>,
        /// Reinstall even if already on the target version.
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum WikiCommands {
    /// Create or update a wiki entry.
    Write {
        /// Entry slug (filename without extension).
        slug: String,
        /// Entry title.
        #[arg(short, long)]
        title: String,
        /// Markdown content. Reads from stdin if omitted.
        #[arg(long)]
        content: Option<String>,
        /// Comma-separated tags.
        #[arg(long)]
        tags: Option<String>,
        /// Entry type: source|entity|concept|analysis|adr|arch|gotcha (default: concept).
        #[arg(long = "type")]
        entry_type: Option<String>,
        /// Comma-separated source paths or URLs.
        #[arg(long)]
        sources: Option<String>,
    },
    /// Read a wiki entry by slug.
    Read {
        /// Entry slug.
        slug: String,
    },
    /// Delete a wiki entry by slug.
    Delete {
        /// Entry slug.
        slug: String,
    },
    /// Semantic search over wiki entries.
    Search {
        /// Search query.
        query: String,
        /// Filter by comma-separated tags.
        #[arg(short, long)]
        tags: Option<String>,
        /// Filter by entry type: source|entity|concept|analysis|adr|arch|gotcha.
        #[arg(long = "type")]
        entry_type: Option<String>,
        /// Max results.
        #[arg(short, long, default_value_t = 10)]
        limit: usize,
    },
    /// List wiki entries.
    List {
        /// Filter by comma-separated tags.
        #[arg(short, long)]
        tags: Option<String>,
        /// Filter by entry type: source|entity|concept|analysis|adr|arch|gotcha.
        #[arg(long = "type")]
        entry_type: Option<String>,
    },
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
}

fn init_logging(config: &Config, cli_level: Option<&str>) {
    let level = cli_level
        .or(Some(config.mcp.log_level.as_str()))
        .unwrap_or("info");

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    if config.mcp.log_file.is_empty() {
        fmt().with_env_filter(filter).with_target(false).init();
    } else {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config.mcp.log_file);
        match file {
            Ok(f) => {
                fmt()
                    .with_env_filter(filter)
                    .with_target(false)
                    .with_writer(std::sync::Mutex::new(f))
                    .init();
            }
            Err(e) => {
                eprintln!("warning: cannot open log file: {e}");
                fmt().with_env_filter(filter).with_target(false).init();
            }
        }
    }
}

fn ensure_data_dirs(config: &Config, root: &std::path::Path) {
    let data_dir = config.resolve_data_dir(root);
    let _ = std::fs::create_dir_all(&data_dir);
    let _ = std::fs::create_dir_all(data_dir.join("frecency"));
    let _ = std::fs::create_dir_all(data_dir.join("history"));
    let _ = std::fs::create_dir_all(data_dir.join("vectors"));
    // ponytail: auto-gitignore .fva contents so host repos stay clean
    let gitignore = data_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(&gitignore, "*\n");
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if matches!(cli.command, Some(Commands::Version)) {
        cli_output::version(env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if let Some(Commands::Upgrade { version, force }) = &cli.command {
        fva::upgrade::run(version.as_deref(), *force)?;
        return Ok(());
    }

    let config = Config::load(cli.config.as_deref(), cli.path.as_deref())?;
    init_logging(&config, cli.log_level.as_deref());

    let root = config.resolve_root(cli.path.as_deref())?;
    ensure_data_dirs(&config, &root);

    tracing::info!("FVA starting — root: {}", root.display());

    let engine = Arc::new(FvaEngine::new(config, root).await?);

    // Wait for FFF scan in background
    let fff_clone = engine.fff.clone();
    tokio::task::spawn_blocking(move || {
        if fff_clone.wait_for_scan(Duration::from_secs(120)) {
            tracing::info!("FFF scan complete — {} files", fff_clone.total_files());
        }
    });

    match cli.command.unwrap_or(Commands::Serve) {
        Commands::Index => {
            let count = engine.indexer.index_all().await?;
            let ast = engine.indexer.stats();
            let vec = engine.vectors.stats();
            let g = engine.graph.stats();
            cli_output::index_done(count, &ast, &vec, &g);
            engine.shutdown().await;
        }

        Commands::Status => {
            if engine.indexer.stats().indexed_files == 0 {
                let _ = engine.indexer.index_all().await;
            }
            let ast = engine.indexer.stats();
            let vec = engine.vectors.stats();
            let g = engine.graph.stats();
            cli_output::status(
                engine.fff.total_files(),
                &ast,
                &vec,
                &g,
                engine.embedder.name(),
            );
            engine.shutdown().await;
        }

        #[cfg(feature = "bench")]
        Commands::Bench {
            iterations,
            warmup,
            output,
            json,
        } => {
            let _ = engine.fff.wait_for_scan(Duration::from_secs(120));
            if engine.indexer.stats().indexed_files == 0 {
                let _ = engine.indexer.index_all().await;
            }
            let opts = fva::bench::BenchOptions {
                iterations,
                warmup,
                queries: vec![
                    "hybrid_search".into(),
                    "Indexer".into(),
                    "embedding".into(),
                    "config".into(),
                ],
                output: output.or_else(|| {
                    Some(
                        engine
                            .config
                            .resolve_data_dir(&engine.root)
                            .join("benchmarks"),
                    )
                }),
                json,
            };
            let report = fva::bench::run(&engine, &opts).await;
            fva::bench::emit(&report, &opts);
            engine.shutdown().await;
        }

        Commands::Search { query, limit } => {
            if engine.indexer.stats().indexed_files == 0 {
                let _ = engine.indexer.index_all().await;
            }
            let result = engine.query.hybrid_search(&query, limit).await;
            cli_output::search_header(&query, result.hits.len());
            for (i, hit) in result.hits.iter().enumerate() {
                cli_output::search_hit(i + 1, hit);
            }
            engine.shutdown().await;
        }

        Commands::Wiki { command } => {
            match command {
                WikiCommands::Write {
                    slug,
                    title,
                    content,
                    tags,
                    entry_type,
                    sources,
                } => {
                    let content = match content {
                        Some(c) => c,
                        None => {
                            use std::io::Read;
                            let mut buf = String::new();
                            std::io::stdin().read_to_string(&mut buf)?;
                            buf
                        }
                    };
                    let tags = parse_tags(&tags.unwrap_or_default());
                    let entry_type = entry_type.unwrap_or_else(|| "concept".into());
                    let sources = parse_tags(&sources.unwrap_or_default());
                    engine
                        .wiki
                        .write(&slug, &title, &entry_type, &tags, &sources, &content)?;
                    cli_output::wiki_saved(&slug);
                }
                WikiCommands::Read { slug } => {
                    let entry = engine.wiki.read(&slug)?;
                    cli_output::wiki_read(
                        &entry.slug,
                        &entry.entry_type,
                        &entry.tags,
                        &entry.sources,
                        &entry.created,
                        &entry.updated,
                        &entry.content,
                    );
                }
                WikiCommands::Delete { slug } => {
                    engine.wiki.delete(&slug)?;
                    cli_output::wiki_deleted(&slug);
                }
                WikiCommands::Search {
                    query,
                    tags,
                    entry_type,
                    limit,
                } => {
                    let tags = tags.map(|t| parse_tags(&t)).filter(|v| !v.is_empty());
                    let results = engine.wiki.search(&query, tags.as_deref(), limit)?;
                    let results: Vec<_> = match &entry_type {
                        Some(t) => results
                            .into_iter()
                            .filter(|(e, _)| &e.entry_type == t)
                            .collect(),
                        None => results,
                    };
                    let mapped: Vec<(String, Vec<String>, String, f64)> = results
                        .iter()
                        .map(|(e, score)| {
                            (
                                e.title.clone(),
                                e.tags.clone(),
                                e.content.clone(),
                                *score as f64,
                            )
                        })
                        .collect();
                    cli_output::wiki_search_results(&query, &mapped);
                }
                WikiCommands::List { tags, entry_type } => {
                    let tags = tags.map(|t| parse_tags(&t)).filter(|v| !v.is_empty());
                    let entries = engine.wiki.list(tags.as_deref());
                    let entries: Vec<_> = match &entry_type {
                        Some(t) => entries.into_iter().filter(|e| &e.entry_type == t).collect(),
                        None => entries,
                    };
                    cli_output::wiki_list(&entries);
                }
                WikiCommands::Ingest {
                    content,
                    file,
                    title,
                    source_uri,
                    area_hint,
                } => {
                    let text = match (content, file) {
                        (Some(c), _) => c,
                        (_, Some(f)) => std::fs::read_to_string(&f)?,
                        _ => std::io::read_to_string(std::io::stdin())?,
                    };
                    let plan = engine.wiki.ingest(
                        &text,
                        source_uri.as_deref(),
                        title.as_deref(),
                        area_hint.as_deref(),
                    )?;
                    cli_output::wiki_ingest_plan(
                        &plan.source_slug,
                        &plan.related,
                        &plan.neighbors,
                        &plan.suggested_slugs,
                    );
                }
                WikiCommands::Query { query } => {
                    let bundle = engine.wiki.query_bundle(&query, 10)?;
                    println!("{bundle}");
                }
                WikiCommands::Lint { stale_days } => {
                    let report = engine.wiki.lint_report(stale_days)?;
                    println!("{report}");
                }
            }
            engine.shutdown().await;
        }

        Commands::Serve => {
            engine.indexer.spawn_background_index();

            let server = FvaServer::new(engine.clone());

            let engine_shutdown = engine.clone();
            tokio::spawn(async move {
                tokio::signal::ctrl_c().await.ok();
                tracing::info!("shutting down FVA...");
                engine_shutdown.shutdown().await;
                let _ = engine_shutdown
                    .indexer
                    .wait_for_index(Duration::from_secs(5));
                std::process::exit(0);
            });

            tracing::info!("MCP server starting on stdio");
            let service = server
                .serve(stdio())
                .await
                .map_err(|e| format!("MCP server error: {e}"))?;

            service.waiting().await?;
            engine.shutdown().await;
        }

        _ => unreachable!(),
    }

    Ok(())
}
