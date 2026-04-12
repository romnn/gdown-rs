use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use reqwest::{Client, ClientBuilder, Response};
use tokio::io::AsyncWriteExt;

use crate::error::{Error, Result};

const DEFAULT_FILE_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_10_1) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/39.0.2171.95 Safari/537.36";
const DEFAULT_FOLDER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.102 Safari/537.36";

const MAX_RESPONSE_HTML_BYTES: usize = 512 * 1024;

#[must_use]
pub fn default_file_user_agent() -> &'static str {
    DEFAULT_FILE_USER_AGENT
}

#[must_use]
pub fn default_folder_user_agent() -> &'static str {
    DEFAULT_FOLDER_USER_AGENT
}

/// Build an HTTP client with the given configuration.
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

#[must_use]
pub fn make_progress_bar(total: Option<u64>, start: u64, quiet: bool) -> Option<ProgressBar> {
    if quiet {
        return None;
    }
    let pb = match total {
        Some(t) => ProgressBar::new(t),
        None => ProgressBar::new_spinner(),
    };
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec})")
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("#>-"),
    );
    pb.set_position(start);
    Some(pb)
}

#[must_use]
pub fn content_length(res: &Response) -> Option<u64> {
    res.headers()
        .get("Content-Length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}

/// Stream response body to `writer` with optional speed limiting and progress.
///
/// Returns the number of bytes written.
///
/// # Errors
///
/// Returns an error if reading the response or writing to the output fails.
pub async fn stream_response(
    res: &mut Response,
    writer: &mut (impl AsyncWriteExt + Unpin),
    pbar: Option<&ProgressBar>,
    speed_limit: Option<f64>,
    start_offset: u64,
) -> Result<u64> {
    let t_start = Instant::now();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = res.chunk().await? {
        writer.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        if let Some(pb) = pbar {
            pb.set_position(start_offset + downloaded);
        }
        if let Some(limit) = speed_limit {
            let elapsed = t_start.elapsed().as_secs_f64();
            #[allow(
                clippy::cast_precision_loss,
                reason = "speed limiting uses approximate floating point; minor precision loss is acceptable"
            )]
            let expected = downloaded as f64 / limit;
            if elapsed < expected {
                tokio::time::sleep(Duration::from_secs_f64(expected - elapsed)).await;
            }
        }
    }

    writer.flush().await?;
    Ok(downloaded)
}

/// Read response body with an actual network-level size limit.
///
/// Reads chunks until `MAX_RESPONSE_HTML_BYTES` is reached, then stops.
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
