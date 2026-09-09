use std::sync::Arc;

#[test]
fn karpathy_roundtrip_ingest_query_lint_fileback() {
    let dir = std::env::temp_dir().join(format!("fva-wiki-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let embedder: Arc<dyn fva::embedding::Embedder> =
        Arc::new(fva::embedding::LocalEmbedder::new(128));
    let store = fva::wiki::WikiStore::open(dir.join("wiki"), embedder).unwrap();

    // ingest → sources/ 보존 + plan
    let plan = store
        .ingest(
            "Rust ownership: borrowing and lifetimes",
            Some("book/ch4"),
            Some("Ownership"),
            Some("concept"),
        )
        .unwrap();
    assert!(plan.source_slug.starts_with("sources/"));

    // fileback → index/log 반영
    store
        .write(
            &plan.suggested_slugs[0],
            "Ownership",
            "concept",
            &["rust".into()],
            &["book/ch4".into()],
            "Ownership notes, see [[concepts/borrowing]]",
        )
        .unwrap();
    assert!(std::fs::read_to_string(dir.join("wiki/index.md"))
        .unwrap()
        .contains(&plan.suggested_slugs[0]));

    // query → bundle에 fileback 포함
    let bundle = store.query_bundle("ownership borrowing", 10).unwrap();
    assert!(bundle.contains("Ownership"));

    // lint → 방금 쓴 페이지는 stale 아님, dead link 1건 보고
    let report = store.lint_report(9999).unwrap();
    assert!(report.contains("concepts/borrowing"));
    assert!(
        !report.contains(&format!(
            "[[{}]] — Ownership (updated",
            plan.suggested_slugs[0]
        )),
        "freshly written page must not be reported stale"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
