//! Builds HTTP clients and streams bounded response bodies.

use std::time::{Duration, Instant};

use reqwest::{Client, ClientBuilder, Response};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};
use crate::types::Progress;

const DEFAULT_FILE_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_10_1) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/39.0.2171.95 Safari/537.36";
const DEFAULT_FOLDER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.102 Safari/537.36";

const MAX_RESPONSE_HTML_BYTES: usize = 512 * 1024;

/// Returns the browser-style user agent used for file requests by default.
#[must_use]
pub fn default_file_user_agent() -> &'static str {
    DEFAULT_FILE_USER_AGENT
}

/// Returns the browser-style user agent used for folder-listing requests by default.
#[must_use]
pub fn default_folder_user_agent() -> &'static str {
    DEFAULT_FOLDER_USER_AGENT
}

/// Builds an HTTP client with the given transport configuration.
///
/// When `proxy` is present, it is applied to every request.
/// Setting `use_cookies` enables an in-memory cookie store, while setting `verify` to
/// `false` accepts invalid TLS certificates.
///
/// # Errors
///
/// Returns an error if the proxy URL is invalid or the client cannot be built.
pub fn build_client(
    proxy: Option<&str>,
    use_cookies: bool,
    user_agent: &str,
    verify: bool,
) -> Result<Client> {
    let mut builder = ClientBuilder::new()
        .user_agent(user_agent)
        .danger_accept_invalid_certs(!verify)
        .cookie_store(use_cookies);

    if let Some(proxy_url) = proxy {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(Error::Http)?;
        builder = builder.proxy(proxy);
        tracing::info!(proxy_url = %proxy_url, "using proxy");
    }

    Ok(builder.build()?)
}

/// Reads a valid `Content-Length` response header as a byte count.
///
/// Returns [`None`] when the header is absent, non-UTF-8, or not an unsigned integer.
#[must_use]
pub fn content_length(res: &Response) -> Option<u64> {
    res.headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Streams a response body to `writer` with optional throttling and progress updates.
///
/// The speed limit is measured in bytes per second.
/// When present, `speed_limit` must be positive and finite.
/// Progress positions include `start_offset`, but the returned byte count covers only bytes read
/// from this response.
/// The writer is flushed and [`Progress::on_finish`] is called after a successful transfer.
///
/// # Errors
///
/// Returns an error if reading the response or writing to the output fails.
pub async fn stream_response(
    res: &mut Response,
    writer: &mut (impl AsyncWriteExt + Unpin),
    progress: Option<&dyn Progress>,
    total: Option<u64>,
    speed_limit: Option<f64>,
    start_offset: u64,
) -> Result<u64> {
    let t_start = Instant::now();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = res.chunk().await? {
        writer.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(cb) = progress {
            cb.on_progress(start_offset + downloaded, total);
        }
        if let Some(limit) = speed_limit {
            let elapsed = t_start.elapsed().as_secs_f64();
            #[expect(
                clippy::cast_precision_loss,
                reason = "throughput throttling tolerates minor precision loss"
            )]
            let expected = downloaded as f64 / limit;
            if elapsed < expected {
                tokio::time::sleep(Duration::from_secs_f64(expected - elapsed)).await;
            }
        }
    }

    writer.flush().await?;

    if let Some(cb) = progress {
        cb.on_finish();
    }

    Ok(downloaded)
}

/// Reads at most 512 KiB from a response body.
///
/// The returned string replaces malformed UTF-8 sequences and may end in the middle of the
/// response.
///
/// # Errors
///
/// Returns an error if reading the response body fails.
pub async fn read_response_limited(res: Response) -> Result<String> {
    let mut buf = Vec::with_capacity(MAX_RESPONSE_HTML_BYTES);
    let mut stream = res;
    while let Some(chunk) = stream.chunk().await? {
        let remaining = MAX_RESPONSE_HTML_BYTES.saturating_sub(buf.len());
        if remaining == 0 {
            break;
        }
        let take = chunk.len().min(remaining);
        buf.extend_from_slice(chunk.get(..take).unwrap_or(&chunk));
        if take < chunk.len() {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&buf).to_string())
}

/// Converts an untrusted filename into a single safe path component.
///
/// Leading and trailing whitespace or quotes are removed, path separators become underscores,
/// and non-breaking spaces become ordinary spaces.
/// Empty names and the special components `.` and `..` become `_`.
#[must_use]
pub fn sanitize_filename(name: &str) -> String {
    let cleaned = name
        .trim()
        .trim_matches(['\'', '"'])
        .replace(['/', '\\'], "_")
        .replace('\u{00A0}', " ");
    let cleaned = cleaned.trim();
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        "_".to_string()
    } else {
        cleaned.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_filename_normal() {
        assert_eq!(sanitize_filename("hello.txt"), "hello.txt");
    }

    #[test]
    fn test_sanitize_filename_slashes() {
        assert_eq!(sanitize_filename("path/to\\file"), "path_to_file");
    }

    #[test]
    fn test_sanitize_filename_traversal() {
        assert_eq!(
            sanitize_filename("../../../etc/passwd"),
            ".._.._.._etc_passwd"
        );
    }

    #[test]
    fn test_sanitize_filename_dot() {
        assert_eq!(sanitize_filename("."), "_");
        assert_eq!(sanitize_filename(".."), "_");
    }

    #[test]
    fn test_sanitize_filename_empty() {
        assert_eq!(sanitize_filename(""), "_");
        assert_eq!(sanitize_filename("   "), "_");
    }

    #[test]
    fn test_sanitize_filename_quotes() {
        assert_eq!(sanitize_filename("'quoted'"), "quoted");
        assert_eq!(sanitize_filename("\"double\""), "double");
    }

    #[test]
    fn test_sanitize_filename_nbsp() {
        assert_eq!(sanitize_filename("hello\u{00A0}world"), "hello world");
    }

    #[test]
    fn test_sanitize_filename_whitespace_padding() {
        assert_eq!(sanitize_filename("  file.txt  "), "file.txt");
    }
}
