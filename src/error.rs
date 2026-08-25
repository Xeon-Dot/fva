use thiserror::Error;

pub type Result<T> = std::result::Result<T, FvaError>;

#[derive(Debug, Error)]
pub enum FvaError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("indexer error: {0}")]
    Indexer(String),

    #[error("fff error: {0}")]
    Fff(#[from] fff_search::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("git error: {0}")]
    Git(#[from] git2::Error),

    #[error("serde error: {0}")]
    Serde(#[from] toml::de::Error),

    #[error("wiki error: {0}")]
    Wiki(String),

    #[error("upgrade error: {0}")]
    Upgrade(String),

    #[error("{0}")]
    Other(String),
}
