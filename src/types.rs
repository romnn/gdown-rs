use crate::error::{Error, Result};

/// Fields shared by both file and folder download options.
#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors CLI flags; these are independent on/off switches, not a state machine"
)]
pub struct CommonOptions {
    pub output: Option<String>,
    pub quiet: bool,
    pub proxy: Option<String>,
    pub speed: Option<f64>,
    pub use_cookies: bool,
    pub verify: bool,
    pub user_agent: Option<String>,
    pub resume: bool,
}

impl Default for CommonOptions {
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
