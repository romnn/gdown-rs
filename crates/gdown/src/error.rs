use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("{0}")]
    FileUrlRetrieval(String),

    #[error("{0}")]
    FolderLimit(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("{0}")]
    Parse(String),

    #[error("{0}")]
    Extract(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
