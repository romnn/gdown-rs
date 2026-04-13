use std::sync::Arc;

use crate::error::{Error, Result};

/// Callback for reporting download progress.
///
/// Implement this trait to receive progress updates during downloads.
/// The built-in [`IndicatifProgress`](crate::progress::IndicatifProgress)
/// implementation is available when the `indicatif` feature is enabled.
pub trait Progress: Send + Sync {
    /// Called after each chunk is downloaded.
    ///
    /// `downloaded` is the total bytes downloaded so far (including any
    /// resume offset). `total` is the expected total size, if known.
    fn on_progress(&self, downloaded: u64, total: Option<u64>);

    /// Called when the download is complete.
    fn on_finish(&self);
}

/// Fields shared by both file and folder download options.
#[derive(Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors CLI flags; these are independent on/off switches, not a state machine"
)]
pub struct Options {
    pub output: Option<String>,
    pub quiet: bool,
    pub proxy: Option<String>,
    pub speed: Option<f64>,
    pub use_cookies: bool,
    pub verify: bool,
    pub user_agent: Option<String>,
    pub resume: bool,
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

/// Resolve a URL-or-ID pair into a single URL.
///
/// Exactly one of `url` / `id` must be `Some`. `base_url_fn` turns an
/// ID into a full URL.
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
