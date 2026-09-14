use comfy_table::{Cell, Color, ContentArrangement, Table, presets::UTF8_FULL};
use console::style;

use crate::graph::GraphStats;
use crate::indexer::store::IndexStats;
use crate::query::HybridHit;
use crate::vector::VectorStats;
use crate::wiki::WikiEntry;

pub fn version(v: &str) {
    println!(
        "\n  {} {}\n  {}\n",
        style("fva").bold().cyan(),
        style(v).bold().white(),
        style("FFF · Vector · AST").dim()
    );
}

pub fn index_done(chunks: usize, ast: &IndexStats, vec: &VectorStats, g: &GraphStats) {
    println!(
        "\n  {} {} chunks from {} files",
        style("✓").green().bold(),
        style(chunks).bold().white(),
        style(ast.indexed_files).white()
    );
    println!(
        "  {} vectors · {} call-graph edges\n",
        style(vec.total_vectors).white(),
        style(g.edges).white()
    );
}

pub fn status(
    fff_files: usize,
    ast: &IndexStats,
    vec: &VectorStats,
    g: &GraphStats,
    embedder: &str,
) {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Component").fg(Color::Cyan),
            Cell::new("Metric").fg(Color::Cyan),
            Cell::new("Value").fg(Color::Cyan),
        ]);

    table.add_row(vec!["FFF", "files scanned", &fff_files.to_string()]);
    table.add_row(vec!["AST", "files indexed", &ast.indexed_files.to_string()]);
    table.add_row(vec!["AST", "chunks", &ast.total_chunks.to_string()]);
    table.add_row(vec!["AST", "symbols", &ast.total_symbols.to_string()]);
    table.add_row(vec![
        "Vectors",
        "embeddings",
        &vec.total_vectors.to_string(),
    ]);
    table.add_row(vec!["Vectors", "embedder", embedder]);
    table.add_row(vec!["Graph", "nodes", &g.nodes.to_string()]);
    table.add_row(vec!["Graph", "edges", &g.edges.to_string()]);

    println!("\n{}", table);
    println!("  {}\n", style("index ready").green().dim());
}

pub fn search_header(query: &str, count: usize) {
    println!(
        "\n  {} {}  {} results\n",
        style("⌕").cyan().bold(),
        style(query).bold().white(),
        style(count).yellow()
    );
}

pub fn search_hit(idx: usize, hit: &HybridHit) {
    println!(
        "  {} {} {}  {}:{}-{}",
        style(format!("[{idx}]")).dim(),
        style(&hit.symbol_name).bold().green(),
        style(format!("[{}]", hit.symbol_kind)).dim(),
        style(&hit.relative_path).cyan(),
        style(hit.start_line).yellow(),
        style(hit.end_line).yellow()
    );
    println!(
        "      score={} sources={} lang={}",
        style(format!("{:.3}", hit.score)).white(),
        style(hit.sources.join("+")).dim(),
        style(&hit.language).dim()
    );
    for line in hit.content.lines().take(12) {
        println!("      {}", style(line).white());
    }
    if hit.content.lines().count() > 12 {
        println!("      {}", style("...").dim());
    }
    println!();
}

pub fn wiki_saved(slug: &str) {
    println!(
        "  {} saved '{}'",
        style("✓").green().bold(),
        style(slug).white()
    );
}

pub fn wiki_deleted(slug: &str) {
    println!(
        "  {} deleted '{}'",
        style("✗").red().bold(),
        style(slug).white()
    );
}

pub fn wiki_read(
    slug: &str,
    entry_type: &str,
    tags: &[String],
    sources: &[String],
    created: &str,
    updated: &str,
    content: &str,
) {
    println!(
        "\n  {} {}",
        style("◆").cyan().bold(),
        style(slug).bold().white()
    );
    println!("  {}", style(format!("type: {entry_type}")).dim());
    if !tags.is_empty() {
        println!("  {}", style(format!("tags: {}", tags.join(", "))).dim());
    }
    println!(
        "  {}",
        style(format!("sources: {}", sources.join(", "))).dim()
    );
    println!(
        "  {} · {}",
        style(format!("created: {created}")).dim(),
        style(format!("updated: {updated}")).dim()
    );
    println!();
    for line in content.lines() {
        println!("  {line}");
    }
    println!();
}

pub fn wiki_list(entries: &[WikiEntry]) {
    if entries.is_empty() {
        println!("\n  {}\n", style("no wiki entries").dim());
        return;
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            Cell::new("Slug").fg(Color::Cyan),
            Cell::new("Title").fg(Color::Cyan),
            Cell::new("Type").fg(Color::Cyan),
            Cell::new("Tags").fg(Color::Cyan),
            Cell::new("Sources").fg(Color::Cyan),
            Cell::new("Updated").fg(Color::Cyan),
        ]);

    for e in entries {
        table.add_row(vec![
            e.slug.as_str(),
            e.title.as_str(),
            e.entry_type.as_str(),
            &e.tags.join(", "),
            &e.sources.join(", "),
            e.updated.as_str(),
        ]);
    }

    println!("\n  {} entries\n", style(entries.len()).bold().white());
    println!("{table}\n");
}

pub fn wiki_ingest_plan(
    source: &str,
    related: &[(String, f32)],
    neighbors: &[String],
    suggested: &[String],
) {
    println!("\n  ingested as [[{source}]]\n");
    println!("  related:");
    for (s, score) in related {
        println!("    [[{s}]] ({score:.3})");
    }
    println!("  neighbors:");
    for n in neighbors {
        println!("    [[{n}]]");
    }
    println!("  touch (max 15):");
    for s in suggested {
        println!("    [[{s}]]");
    }
}

pub fn wiki_search_results(query: &str, results: &[(String, Vec<String>, String, f64)]) {
    if results.is_empty() {
        println!(
            "\n  {} results for '{}'\n",
            style(0).yellow(),
            style(query).white()
        );
        return;
    }

    println!(
        "\n  {} results for '{}'\n",
        style(results.len()).bold().yellow(),
        style(query).bold().white()
    );

    for (title, tags, content, score) in results {
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!(
            "  {} {} {}",
            style("◆").cyan(),
            style(title).bold().white(),
            style(format!("{tag_str} score={score:.3}")).dim()
        );
        for line in content.lines().take(8) {
            println!("    {line}");
        }
        if content.lines().count() > 8 {
            println!("    {}", style("...").dim());
        }
        println!();
    }
}
