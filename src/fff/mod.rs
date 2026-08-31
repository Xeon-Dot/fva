//! FFF (Fast File Finder) integration layer.
//!
//! Wraps `fff-search` for file discovery, content grep, and frecency tracking.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fff_query_parser::AiGrepConfig;
use fff_search::file_picker::FilePicker;
use fff_search::frecency::FrecencyTracker;
use fff_search::grep::{GrepMode, GrepSearchOptions, has_regex_metacharacters};
use fff_search::{
    FFFMode, FilePickerOptions, FuzzySearchOptions, PaginationArgs, QueryParser, QueryTracker,
    SharedFilePicker, SharedFrecency, SharedQueryTracker,
};
use git2::Repository;
use std::sync::RwLock;

use crate::config::FffConfig;
use crate::error::{FvaError, Result};

/// Owned find_files result (no lifetime ties to FilePicker).
#[derive(Debug, Clone)]
pub struct FindFilesOutput {
    pub paths: Vec<String>,
    pub total_matched: usize,
    pub total_files: usize,
}

/// Owned grep match line.
#[derive(Debug, Clone)]
pub struct GrepMatchLine {
    pub file: String,
    pub line_number: usize,
    pub content: String,
}

/// Owned grep result.
#[derive(Debug, Clone)]
pub struct GrepOutput {
    pub matches: Vec<GrepMatchLine>,
    pub next_file_offset: usize,
}

/// Shared FFF engine state.
#[derive(Clone)]
pub struct FffEngine {
    pub picker: SharedFilePicker,
    pub frecency: SharedFrecency,
    pub query_tracker: SharedQueryTracker,
    base_path: Arc<RwLock<String>>,
}

fn with_picker<F, T>(picker: &SharedFilePicker, f: F) -> Result<T>
where
    F: FnOnce(&FilePicker) -> Result<T>,
{
    let guard = picker.read()?;
    let p = guard
        .as_ref()
        .ok_or_else(|| FvaError::Fff(fff_search::Error::FilePickerMissing))?;
    f(p)
}

impl FffEngine {
    pub fn new(base_path: impl AsRef<Path>, config: &FffConfig) -> Result<Self> {
        let base_path = discover_git_root(base_path.as_ref())?;
        tracing::info!("FFF indexing root: {}", base_path);

        let shared_picker = SharedFilePicker::default();
        let shared_frecency = SharedFrecency::default();
        let shared_query_tracker = SharedQueryTracker::default();

        let frecency_db = resolve_db_path(&base_path, &config.frecency_db);
        let history_db = resolve_db_path(&base_path, &config.history_db);

        if let Some(parent) = frecency_db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if let Some(parent) = history_db.parent() {
            std::fs::create_dir_all(parent)?;
        }

        match FrecencyTracker::open(&frecency_db) {
            Ok(tracker) => {
                shared_frecency.init(tracker)?;
            }
            Err(e) => {
                tracing::warn!("frecency db unavailable: {e}");
            }
        }

        match QueryTracker::open(&history_db) {
            Ok(tracker) => {
                shared_query_tracker.init(tracker)?;
            }
            Err(e) => {
                tracing::warn!("query history db unavailable: {e}");
            }
        }

        let enable_content_indexing = config.enable_content_indexing && config.enable_warmup;

        FilePicker::new_with_shared_state(
            shared_picker.clone(),
            shared_frecency.clone(),
            FilePickerOptions {
                base_path: base_path.clone(),
                enable_mmap_cache: config.enable_warmup,
                enable_content_indexing,
                watch: true,
                mode: FFFMode::Ai,
                cache_budget: Some(fff_search::ContentCacheBudget::new_for_repo(
                    config.max_cached_files,
                )),
                follow_symlinks: false,
                ..Default::default()
            },
        )?;

        Ok(Self {
            picker: shared_picker,
            frecency: shared_frecency,
            query_tracker: shared_query_tracker,
            base_path: Arc::new(RwLock::new(base_path)),
        })
    }

    pub fn wait_for_scan(&self, timeout: Duration) -> bool {
        self.picker.wait_for_scan(timeout)
    }

    pub fn is_scanning(&self) -> bool {
        self.picker
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.is_scan_active()))
            .unwrap_or(true)
    }

    pub fn total_files(&self) -> usize {
        self.picker
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|p| p.get_files().len()))
            .unwrap_or(0)
    }

    pub fn base_path(&self) -> String {
        self.base_path.read().unwrap().clone()
    }

    pub fn find_files(&self, query: &str, offset: usize, limit: usize) -> Result<FindFilesOutput> {
        with_picker(&self.picker, |picker| {
            let parser = QueryParser::default();
            let parsed = parser.parse(query);
            let qt_guard = self.query_tracker.read().ok();
            let qt_ref = qt_guard.as_ref().and_then(|g| g.as_ref());

            let result = picker.fuzzy_search(
                &parsed,
                qt_ref,
                FuzzySearchOptions {
                    max_threads: 0,
                    current_file: None,
                    project_path: Some(picker.base_path()),
                    combo_boost_score_multiplier: 100,
                    min_combo_count: 3,
                    pagination: PaginationArgs { offset, limit },
                },
            );

            let paths = result
                .items
                .iter()
                .map(|item| item.relative_path(picker).to_string())
                .collect();

            Ok(FindFilesOutput {
                paths,
                total_matched: result.total_matched,
                total_files: result.total_files,
            })
        })
    }

    pub fn grep(&self, query: &str, offset: usize, limit: usize) -> Result<GrepOutput> {
        with_picker(&self.picker, |picker| {
            let parser = QueryParser::new(AiGrepConfig);
            let parsed = parser.parse(query);
            let grep_text = parsed.grep_text();

            let mode = if has_regex_metacharacters(&grep_text) {
                GrepMode::Regex
            } else {
                GrepMode::PlainText
            };

            let options = GrepSearchOptions {
                max_file_size: 10 * 1024 * 1024,
                max_matches_per_file: 10,
                smart_case: true,
                file_offset: offset,
                page_limit: limit,
                mode,
                time_budget_ms: 0,
                before_context: 0,
                after_context: 8,
                classify_definitions: true,
                trim_whitespace: true,
                abort_signal: None,
            };

            let result = picker.grep(&parsed, &options);
            let matches = result
                .matches
                .iter()
                .map(|m| {
                    let file = result.files[m.file_index].relative_path(picker).to_string();
                    GrepMatchLine {
                        file,
                        line_number: m.line_number as usize,
                        content: m.line_content.trim().to_string(),
                    }
                })
                .collect();

            Ok(GrepOutput {
                matches,
                next_file_offset: result.next_file_offset,
            })
        })
    }

    pub fn shutdown(&self) {
        if let Ok(mut guard) = self.picker.write()
            && let Some(ref mut picker) = *guard
        {
            picker.stop_background_monitor();
        }
    }
}

fn resolve_db_path(base_path: &str, db_path: &str) -> std::path::PathBuf {
    let path = Path::new(db_path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(base_path).join(path)
    }
}

fn discover_git_root(path: &Path) -> Result<String> {
    let path_str = path.to_string_lossy().to_string();
    match Repository::discover(path) {
        Ok(repo) => {
            if let Some(workdir) = repo.workdir() {
                Ok(workdir.to_string_lossy().to_string())
            } else {
                Ok(path_str)
            }
        }
        Err(_) => Ok(path_str),
    }
}
