//! Downloads files and folder trees from public Google Drive links.
//!
//! Use [`download()`] for one file and [`download_folder()`] for a complete folder tree.
//! The lower-level [`DriveFolder`] API supports cached directory listing, path resolution, and
//! selective downloads.
//!
//! # Optional features
//!
//! - `rustls` is enabled by default.
//! - `default-tls`, `native-tls`, `native-tls-no-alpn`, `native-tls-vendored`,
//!   `native-tls-vendored-no-alpn`, and `rustls-no-provider` forward the corresponding Reqwest
//!   TLS features.
//! - `indicatif` adds a terminal progress-bar implementation.
//!
//! Disable default features when selecting a non-default TLS backend.
//!
//! # Examples
//!
//! ```no_run
//! # // Uses `no_run` because downloading requires network access to Google Drive.
//! # async fn example() -> gdown::Result<()> {
//! use gdown::{DownloadOptions, download};
//!
//! let options = DownloadOptions {
//!     id: Some("FILE_ID".to_string()),
//!     ..DownloadOptions::default()
//! };
//! let output = download(&options).await?;
//! assert!(output.is_some());
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod download;
pub mod download_folder;
pub mod error;
pub mod extract;
pub mod parse;
#[cfg(feature = "indicatif")]
pub mod progress;
pub mod types;

pub use download::{DownloadOptions, download};
pub use download_folder::{
    DirEntry, DownloadFolderOptions, DriveFolder, FlatEntry, download_folder,
};
pub use error::{Error, Result};
pub use extract::extractall;
pub use parse::{ParsedUrl, is_google_drive_url, parse_url};
pub use types::{Options, Progress};
