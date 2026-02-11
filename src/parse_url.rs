use regex::Regex;
use url::Url;

/// Check if a URL is a Google Drive URL.
pub fn is_google_drive_url(url_str: &str) -> bool {
    if let Ok(parsed) = Url::parse(url_str) {
        if let Some(host) = parsed.host_str() {
            return host == "drive.google.com" || host == "docs.google.com";
        }
    }
    false
}

/// Parse URLs especially for Google Drive links.
///
/// Returns `(file_id, is_download_link)` where:
/// - `file_id`: ID of file on Google Drive (None if not a Google Drive URL or ID not found).
/// - `is_download_link`: Flag if it is a download link of Google Drive.
pub fn parse_url(url_str: &str, warning: bool) -> (Option<String>, bool) {
    let parsed = match Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return (None, false),
    };

    let is_gdrive = is_google_drive_url(url_str);
    let is_download_link = parsed.path().ends_with("/uc");

    if !is_gdrive {
        return (None, is_download_link);
    }

    let mut file_id: Option<String> = None;

    // Check query parameter "id"
    let query_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    let id_values: Vec<&str> = query_pairs
        .iter()
        .filter(|(k, _)| k == "id")
        .map(|(_, v)| v.as_str())
        .collect();

    if id_values.len() == 1 {
        file_id = Some(id_values[0].to_string());
    } else if id_values.is_empty() {
        let patterns = [
            r"^/file/d/(.*?)/(edit|view)$",
            r"^/file/u/[0-9]+/d/(.*?)/(edit|view)$",
            r"^/document/d/(.*?)/(edit|htmlview|view)$",
            r"^/document/u/[0-9]+/d/(.*?)/(edit|htmlview|view)$",
            r"^/presentation/d/(.*?)/(edit|htmlview|view)$",
            r"^/presentation/u/[0-9]+/d/(.*?)/(edit|htmlview|view)$",
            r"^/spreadsheets/d/(.*?)/(edit|htmlview|view)$",
            r"^/spreadsheets/u/[0-9]+/d/(.*?)/(edit|htmlview|view)$",
        ];

        let path = parsed.path();
        for pattern in &patterns {
            let re = Regex::new(pattern).unwrap();
            if let Some(caps) = re.captures(path) {
                file_id = Some(caps[1].to_string());
                break;
            }
        }
    }

    if warning && !is_download_link {
        if let Some(ref fid) = file_id {
            eprintln!(
                "Warning: You specified a Google Drive link that is not the correct link \
                 to download a file. You might want to try `--fuzzy` option \
                 or the following url: https://drive.google.com/uc?id={}",
                fid
            );
        }
    }

    (file_id, is_download_link)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url() {
        let file_id = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";

        // (url, expected_file_id, expected_is_download_link, should_warn)
        let urls = vec![
            (
                format!("https://drive.google.com/open?id={}", file_id),
                Some(file_id.to_string()),
                false,
                true,
            ),
            (
                format!("https://drive.google.com/uc?id={}", file_id),
                Some(file_id.to_string()),
                true,
                false,
            ),
            (
                format!(
                    "https://drive.google.com/file/d/{}/view?usp=sharing",
                    file_id
                ),
                Some(file_id.to_string()),
                false,
                true,
            ),
            (
                format!(
                    "https://drive.google.com/a/jsk.imi.i.u-tokyo.ac.jp/uc?id={}&export=download",
                    file_id
                ),
                Some(file_id.to_string()),
                true,
                false,
            ),
        ];

        for (url, expected_id, expected_is_download, _should_warn) in urls {
            // We pass warning=false to avoid printing warnings in tests
            let (actual_id, actual_is_download) = parse_url(&url, false);
            assert_eq!(
                actual_id, expected_id,
                "file_id mismatch for url: {}",
                url
            );
            assert_eq!(
                actual_is_download, expected_is_download,
                "is_download_link mismatch for url: {}",
                url
            );
        }
    }

    #[test]
    fn test_is_google_drive_url() {
        assert!(is_google_drive_url("https://drive.google.com/uc?id=abc"));
        assert!(is_google_drive_url("https://docs.google.com/document/d/abc/view"));
        assert!(!is_google_drive_url("https://example.com/file"));
    }
}
