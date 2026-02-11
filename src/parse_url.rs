use url::Url;

fn is_action(action: &str, allowed: &[&str]) -> bool {
    allowed.contains(&action)
}

/// Check if a URL is a Google Drive URL.
#[must_use]
pub fn is_google_drive_url(url_str: &str) -> bool {
    if let Ok(parsed) = Url::parse(url_str)
        && let Some(host) = parsed.host_str()
    {
        return host == "drive.google.com" || host == "docs.google.com";
    }
    false
}

/// Parse URLs especially for Google Drive links.
///
/// Returns `(file_id, is_download_link)` where:
/// - `file_id`: ID of file on Google Drive (None if not a Google Drive URL or ID not found).
/// - `is_download_link`: Flag if it is a download link of Google Drive.
#[must_use]
pub fn parse_url(url_str: &str, warning: bool) -> (Option<String>, bool) {
    let Ok(parsed) = Url::parse(url_str) else {
        return (None, false);
    };

    let is_gdrive = is_google_drive_url(url_str);
    let is_download_link = parsed.path().ends_with("/uc");

    if !is_gdrive {
        return (None, is_download_link);
    }

    let mut id_iter = parsed
        .query_pairs()
        .filter(|(k, _)| k.as_ref() == "id")
        .map(|(_, v)| v.into_owned());

    let first_id = id_iter.next();
    let second_id = id_iter.next();

    let file_id = if second_id.is_some() {
        None
    } else if let Some(id) = first_id {
        Some(id)
    } else {
        let segments: Vec<&str> = match parsed.path_segments() {
            Some(s) => s.collect(),
            None => Vec::new(),
        };

        match segments.as_slice() {
            ["file", "d", fid, action, ..] if is_action(action, &["edit", "view"]) => {
                Some((*fid).to_string())
            }
            ["file", "u", _user, "d", fid, action, ..] if is_action(action, &["edit", "view"]) => {
                Some((*fid).to_string())
            }
            ["document", "d", fid, action, ..]
                if is_action(action, &["edit", "htmlview", "view"]) =>
            {
                Some((*fid).to_string())
            }
            ["document", "u", _user, "d", fid, action, ..]
                if is_action(action, &["edit", "htmlview", "view"]) =>
            {
                Some((*fid).to_string())
            }
            ["presentation", "d", fid, action, ..]
                if is_action(action, &["edit", "htmlview", "view"]) =>
            {
                Some((*fid).to_string())
            }
            ["presentation", "u", _user, "d", fid, action, ..]
                if is_action(action, &["edit", "htmlview", "view"]) =>
            {
                Some((*fid).to_string())
            }
            ["spreadsheets", "d", fid, action, ..]
                if is_action(action, &["edit", "htmlview", "view"]) =>
            {
                Some((*fid).to_string())
            }
            ["spreadsheets", "u", _user, "d", fid, action, ..]
                if is_action(action, &["edit", "htmlview", "view"]) =>
            {
                Some((*fid).to_string())
            }
            _ => None,
        }
    };

    if warning
        && !is_download_link
        && let Some(ref fid) = file_id
    {
        tracing::warn!(
            file_id = %fid,
            suggested_url = %format!("https://drive.google.com/uc?id={fid}"),
            "specified google drive link is not a direct download link; consider using --fuzzy"
        );
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
                format!("https://drive.google.com/open?id={file_id}"),
                Some(file_id.to_string()),
                false,
                true,
            ),
            (
                format!("https://drive.google.com/uc?id={file_id}"),
                Some(file_id.to_string()),
                true,
                false,
            ),
            (
                format!("https://drive.google.com/file/d/{file_id}/view?usp=sharing"),
                Some(file_id.to_string()),
                false,
                true,
            ),
            (
                format!(
                    "https://drive.google.com/a/jsk.imi.i.u-tokyo.ac.jp/uc?id={file_id}&export=download"
                ),
                Some(file_id.to_string()),
                true,
                false,
            ),
        ];

        for (url, expected_id, expected_is_download, _should_warn) in urls {
            // We pass warning=false to avoid printing warnings in tests
            let (actual_id, actual_is_download) = parse_url(&url, false);
            assert_eq!(actual_id, expected_id, "file_id mismatch for url: {url}");
            assert_eq!(
                actual_is_download, expected_is_download,
                "is_download_link mismatch for url: {url}"
            );
        }
    }

    #[test]
    fn test_is_google_drive_url() {
        assert!(is_google_drive_url("https://drive.google.com/uc?id=abc"));
        assert!(is_google_drive_url(
            "https://docs.google.com/document/d/abc/view"
        ));
        assert!(!is_google_drive_url("https://example.com/file"));
    }
}
