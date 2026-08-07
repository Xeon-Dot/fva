use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{FvaError, Result};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub project: ProjectConfig,
    #[serde(default)]
    pub indexer: IndexerConfig,
    #[serde(default)]
    pub fff: FffConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub vector: VectorConfig,
    #[serde(default)]
    pub query: QueryConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    #[serde(default = "default_root")]
    pub root: String,
}

impl Default for ProjectConfig {
    fn default() -> Self {
        Self {
            root: default_root(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerConfig {
    #[serde(default = "default_true")]
    pub watch: bool,
    #[serde(default = "default_debounce")]
    pub debounce_ms: u64,
    #[serde(default = "default_max_file_size")]
    pub max_file_size: u64,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default = "default_true")]
    pub respect_gitignore: bool,
    #[serde(default = "default_true")]
    pub git_boost: bool,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            watch: default_true(),
            debounce_ms: default_debounce(),
            max_file_size: default_max_file_size(),
            languages: vec![],
            respect_gitignore: default_true(),
            git_boost: default_true(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FffConfig {
    #[serde(default = "default_frecency_db")]
    pub frecency_db: String,
    #[serde(default = "default_history_db")]
    pub history_db: String,
    #[serde(default = "default_max_cached_files")]
    pub max_cached_files: usize,
    #[serde(default = "default_true")]
    pub enable_warmup: bool,
    #[serde(default = "default_true")]
    pub enable_content_indexing: bool,
}

impl Default for FffConfig {
    fn default() -> Self {
        Self {
            frecency_db: default_frecency_db(),
            history_db: default_history_db(),
            max_cached_files: default_max_cached_files(),
            enable_warmup: default_true(),
            enable_content_indexing: default_true(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_embedding_provider")]
    pub provider: String,
    #[serde(default)]
    pub voyage_api_key: String,
    #[serde(default = "default_embedding_model")]
    pub model: String,
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            provider: default_embedding_provider(),
            voyage_api_key: String::new(),
            model: default_embedding_model(),
            dimensions: default_dimensions(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VectorConfig {
    #[serde(default = "default_vector_backend")]
    pub backend: String,
    #[serde(default = "default_vector_db")]
    pub db_path: String,
}

impl Default for VectorConfig {
    fn default() -> Self {
        Self {
            backend: default_vector_backend(),
            db_path: default_vector_db(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryConfig {
    #[serde(default = "default_max_results")]
    pub default_max_results: usize,
    #[serde(default = "default_fff_weight")]
    pub fff_weight: f32,
    #[serde(default = "default_vector_weight")]
    pub vector_weight: f32,
    #[serde(default = "default_graph_weight")]
    pub graph_weight: f32,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: usize,
}

impl Default for QueryConfig {
    fn default() -> Self {
        Self {
            default_max_results: default_max_results(),
            fff_weight: default_fff_weight(),
            vector_weight: default_vector_weight(),
            graph_weight: default_graph_weight(),
            max_context_tokens: default_max_context_tokens(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default = "default_server_name")]
    pub server_name: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub log_file: String,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            server_name: default_server_name(),
            log_level: default_log_level(),
            log_file: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    #[serde(default = "default_true")]
    pub sandbox_indexing: bool,
    #[serde(default = "default_true")]
    pub no_telemetry: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            sandbox_indexing: default_true(),
            no_telemetry: default_true(),
        }
    }
}

fn default_root() -> String {
    ".".to_string()
}
fn default_true() -> bool {
    true
}
fn default_debounce() -> u64 {
    300
}
fn default_max_file_size() -> u64 {
    10 * 1024 * 1024
}
fn default_frecency_db() -> String {
    ".fva/frecency".to_string()
}
fn default_history_db() -> String {
    ".fva/history".to_string()
}
fn default_max_cached_files() -> usize {
    30_000
}
fn default_embedding_provider() -> String {
    "local".to_string()
}
fn default_embedding_model() -> String {
    "voyage-code-3".to_string()
}
fn default_dimensions() -> usize {
    1024
}
fn default_vector_backend() -> String {
    "lancedb".to_string()
}
fn default_vector_db() -> String {
    "vectors".to_string()
}
fn default_max_results() -> usize {
    20
}
fn default_fff_weight() -> f32 {
    0.3
}
fn default_vector_weight() -> f32 {
    0.5
}
fn default_graph_weight() -> f32 {
    0.2
}
fn default_max_context_tokens() -> usize {
    8000
}
fn default_server_name() -> String {
    "fva".to_string()
}
fn default_log_level() -> String {
    "warn".to_string()
}

impl Config {
    /// Load config with layered precedence (low → high):
    /// defaults → `~/.config/fva/config.toml` → project `fva.toml` / `.fva.toml` → `--config`.
    pub fn load(explicit: Option<&Path>, cli_root: Option<&str>) -> Result<Self> {
        Self::load_layered(
            Some(Self::global_config_path().as_path()),
            cli_root,
            explicit,
        )
    }

    fn load_layered(
        global: Option<&Path>,
        cli_root: Option<&str>,
        explicit: Option<&Path>,
    ) -> Result<Self> {
        let default_toml = toml::to_string(&Self::default())
            .map_err(|e| FvaError::Config(format!("serialize defaults: {e}")))?;
        let mut value: toml::Value = toml::from_str(&default_toml)
            .map_err(|e| FvaError::Config(format!("parse defaults: {e}")))?;

        if let Some(path) = global {
            Self::merge_file(&mut value, path)?;
        }

        if let Some(root) = Self::tentative_project_root(cli_root, &value)
            && let Some(project_path) = Self::project_config_path(&root)
        {
            Self::merge_file(&mut value, &project_path)?;
        }

        if let Some(path) = explicit {
            Self::merge_file(&mut value, path)?;
        }

        let merged_toml = toml::to_string(&value)
            .map_err(|e| FvaError::Config(format!("serialize merged config: {e}")))?;
        toml::from_str(&merged_toml).map_err(|e| FvaError::Config(format!("invalid config: {e}")))
    }

    /// Global defaults: `~/.config/fva/config.toml` (XDG-style under home).
    pub fn global_config_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_default()
            .join(".config")
            .join("fva")
            .join("config.toml")
    }

    /// Project config at root: `fva.toml` preferred, then `.fva.toml`.
    pub fn project_config_path(root: &Path) -> Option<PathBuf> {
        let fva = root.join("fva.toml");
        if fva.is_file() {
            return Some(fva);
        }
        let dot = root.join(".fva.toml");
        if dot.is_file() {
            return Some(dot);
        }
        None
    }

    pub fn project_config_candidates(root: &Path) -> [PathBuf; 2] {
        [root.join("fva.toml"), root.join(".fva.toml")]
    }

    fn tentative_project_root(cli_root: Option<&str>, merged: &toml::Value) -> Option<PathBuf> {
        if let Some(root) = cli_root {
            return Some(PathBuf::from(root));
        }

        let project_root = merged
            .get("project")
            .and_then(|p| p.get("root"))
            .and_then(|r| r.as_str())
            .unwrap_or(".");

        if project_root != "." {
            return Some(PathBuf::from(project_root));
        }

        std::env::current_dir().ok()
    }

    fn merge_file(base: &mut toml::Value, path: &Path) -> Result<()> {
        if !path.is_file() {
            return Ok(());
        }
        let content = std::fs::read_to_string(path)?;
        let overlay: toml::Value = toml::from_str(&content)
            .map_err(|e| FvaError::Config(format!("parse {}: {e}", path.display())))?;
        merge_toml_values(base, &overlay);
        Ok(())
    }

    pub fn resolve_root(&self, cli_override: Option<&str>) -> Result<PathBuf> {
        let root = cli_override.unwrap_or(&self.project.root);
        let path = PathBuf::from(root);
        let canonical = dunce::canonicalize(&path)
            .map_err(|e| FvaError::Config(format!("invalid project root '{}': {e}", root)))?;
        Ok(canonical)
    }

    pub fn resolve_data_dir(&self, root: &Path) -> PathBuf {
        root.join(".fva")
    }
}

fn merge_toml_values(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, overlay_value) in overlay_table {
                match base_table.get_mut(key) {
                    Some(base_value) => merge_toml_values(base_value, overlay_value),
                    None => {
                        base_table.insert(key.clone(), overlay_value.clone());
                    }
                }
            }
        }
        (base_slot, overlay_value) => *base_slot = overlay_value.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn global_config_path_is_xdg_home_config() {
        let expected = dirs::home_dir()
            .expect("home dir")
            .join(".config")
            .join("fva")
            .join("config.toml");
        assert_eq!(Config::global_config_path(), expected);
    }

    #[test]
    fn project_config_prefers_fva_toml_over_dot_fva_toml() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("fva.toml"), "[query]\ndefault_max_results = 7\n").unwrap();
        fs::write(root.join(".fva.toml"), "[query]\ndefault_max_results = 9\n").unwrap();

        let path = Config::project_config_path(root).expect("project config");
        assert_eq!(path, root.join("fva.toml"));
    }

    #[test]
    fn project_config_falls_back_to_dot_fva_toml() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join(".fva.toml"), "[query]\ndefault_max_results = 9\n").unwrap();

        let path = Config::project_config_path(root).expect("project config");
        assert_eq!(path, root.join(".fva.toml"));
    }

    #[test]
    fn layered_config_merges_global_and_project() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        let global = root.join("global.toml");
        fs::write(
            &global,
            "[embedding]\nprovider = \"voyage\"\n[query]\nfff_weight = 0.1\n",
        )
        .unwrap();
        fs::write(root.join("fva.toml"), "[query]\nvector_weight = 0.9\n").unwrap();

        let config =
            Config::load_layered(Some(&global), Some(root.to_str().unwrap()), None).expect("load");

        assert_eq!(config.embedding.provider, "voyage");
        assert!((config.query.fff_weight - 0.1).abs() < f32::EPSILON);
        assert!((config.query.vector_weight - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn explicit_config_overrides_project_layer() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        fs::write(root.join("fva.toml"), "[query]\nvector_weight = 0.9\n").unwrap();
        let explicit = root.join("override.toml");
        fs::write(&explicit, "[query]\nvector_weight = 0.2\n").unwrap();

        let config = Config::load_layered(None, Some(root.to_str().unwrap()), Some(&explicit))
            .expect("load");

        assert!((config.query.vector_weight - 0.2).abs() < f32::EPSILON);
    }

    #[test]
    fn loads_global_xdg_config_when_no_local_config() {
        let xdg = Config::global_config_path();
        if !xdg.is_file() {
            return;
        }

        let config = Config::load(None, None).expect("load config");
        assert_eq!(
            config.embedding.provider, "voyage",
            "global ~/.config/fva/config.toml should set embedding.provider"
        );
    }
}
