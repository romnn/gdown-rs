//! Defines errors returned by download, parsing, and extraction operations.

use thiserror::Error;

/// An error produced by the `gdown` library.
#[derive(Error, Debug)]
pub enum Error {
    /// A Google Drive response did not expose a usable file download URL.
    #[error("{0}")]
    FileUrlRetrieval(String),

    /// A folder contains at least the configured maximum number of files.
    #[error("{0}")]
    FolderLimit(String),

    /// Caller-provided options or identifiers are inconsistent or unsupported.
    #[error("{0}")]
    InvalidInput(String),

    /// Google Drive metadata or an HTTP header could not be interpreted.
    #[error("{0}")]
    Parse(String),

    /// An archive format is unsupported or its contents could not be extracted.
    #[error("{0}")]
    Extract(String),

    /// An HTTP client, request, or response-body operation failed.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// A local filesystem or asynchronous I/O operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// A URL required by a download operation was malformed.
    #[error("URL parse error: {0}")]
    UrlParse(#[from] url::ParseError),

    /// Embedded Google Drive metadata was not valid JSON.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

/// A result whose error type is [`enum@Error`].
pub type Result<T> = std::result::Result<T, Error>;
