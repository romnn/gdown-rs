//! Defines shared download options and progress reporting contracts.

use std::sync::Arc;

use crate::error::{Error, Result};

/// Callback for reporting download progress.
///
/// Implement this trait to receive progress updates during downloads.
///
/// The optional `indicatif` feature provides a terminal progress-bar implementation.
pub trait Progress: Send + Sync {
    /// Reports the cumulative byte position after a response chunk is written.
    ///
    /// `downloaded` includes any resume offset.
    /// `total` is the expected final size in bytes when the server provides enough
    /// information to calculate it.
    fn on_progress(&self, downloaded: u64, total: Option<u64>);

    /// Reports that the response body was written and flushed successfully.
    ///
    /// This method is not called when streaming or flushing fails.
    fn on_finish(&self);
}

/// Holds settings shared by file and folder downloads.
#[derive(Clone)]
pub struct Options {
    /// Selects the output path.
    ///
    /// [`None`] derives a filename or uses the current directory.
    /// A value of `-` writes a single-file download to standard output.
    pub output: Option<String>,
    /// Suppresses informational download logging when `true`.
    pub quiet: bool,
    /// Routes HTTP and HTTPS requests through this proxy URL when present.
    pub proxy: Option<String>,
    /// Limits transfer throughput to this many bytes per second when present.
    ///
    /// Values must be positive and finite.
    pub speed: Option<f64>,
    /// Enables the HTTP client's in-memory cookie store.
    pub use_cookies: bool,
    /// Verifies server TLS certificates when `true`.
    pub verify: bool,
    /// Overrides the operation-specific default HTTP user agent when present.
    pub user_agent: Option<String>,
    /// Reuses partial downloads and skips completed files when `true`.
    pub resume: bool,
    /// Receives progress updates for transferred response bodies when present.
    pub progress: Option<Arc<dyn Progress>>,
}

impl std::fmt::Debug for Options {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Options")
            .field("output", &self.output)
            .field("quiet", &self.quiet)
            .field("proxy", &self.proxy)
            .field("speed", &self.speed)
            .field("use_cookies", &self.use_cookies)
            .field("verify", &self.verify)
            .field("user_agent", &self.user_agent)
            .field("resume", &self.resume)
            .field("progress", &self.progress.as_ref().map(|_| "..."))
            .finish()
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            output: None,
            quiet: false,
            proxy: None,
            speed: None,
            use_cookies: true,
            verify: true,
            user_agent: None,
            resume: false,
            progress: None,
        }
    }
}

/// Resolves a URL-or-ID pair into a single URL.
///
/// Exactly one of `url` and `id` must be [`Some`].
/// `base_url_fn` converts a supplied ID into a full URL and is not called when `url` is
/// supplied.
///
/// # Errors
///
/// Returns an error if both or neither of `url` / `id` are provided.
pub fn resolve_url_or_id(
    url: &Option<String>,
    id: &Option<String>,
    base_url_fn: impl FnOnce(&str) -> String,
) -> Result<String> {
    match (id, url) {
        (Some(id), None) => Ok(base_url_fn(id)),
        (None, Some(u)) => Ok(u.clone()),
        _ => Err(Error::InvalidInput(
            "exactly one of url or id must be specified".to_string(),
        )),
    }
}
