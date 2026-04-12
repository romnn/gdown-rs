use std::sync::LazyLock;

use regex::Regex;
use reqwest::Response;
use reqwest::header::{HeaderMap, SET_COOKIE};
use scraper::{Html, Selector};
use url::Url;

use crate::client::sanitize_filename;
use crate::error::{Error, Result};

macro_rules! lazy_regex {
    ($pattern:expr) => {{
        #[allow(
            clippy::expect_used,
            reason = "regex pattern is a compile-time literal"
        )]
        static RE: LazyLock<Regex> =
            LazyLock::new(|| Regex::new($pattern).expect("valid regex literal"));
        &RE
    }};
}

fn uc_export_re() -> &'static Regex {
    lazy_regex!(r#"href="(/uc\?export=download[^"]+)""#)
}
fn download_url_re() -> &'static Regex {
    lazy_regex!(r#""downloadUrl":"([^"]+)""#)
}
fn error_subcaption_re() -> &'static Regex {
    lazy_regex!(r#"<p class="uc-error-subcaption">(.*)</p>"#)
}
fn title_re() -> &'static Regex {
    lazy_regex!(r"<title>(.+)</title>")
}
fn drive_string_re() -> &'static Regex {
    lazy_regex!(r"'((?:[^'\\]|\\.)*)'")
}

fn is_action(action: &str, allowed: &[&str]) -> bool {
    allowed.contains(&action)
}

#[must_use]
pub fn is_google_drive_url(url_str: &str) -> bool {
    if let Ok(parsed) = Url::parse(url_str)
        && let Some(host) = parsed.host_str()
    {
        return host == "drive.google.com" || host == "docs.google.com";
    }
    false
}

/// Result of parsing a Google Drive URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedUrl {
    pub file_id: Option<String>,
    pub is_download_link: bool,
}

/// Parse a Google Drive URL, extracting the file ID and whether it's a direct download link.
#[must_use]
pub fn parse_url(url_str: &str) -> ParsedUrl {
    let Ok(parsed) = Url::parse(url_str) else {
        return ParsedUrl {
            file_id: None,
            is_download_link: false,
        };
    };

    let is_gdrive = is_google_drive_url(url_str);
    let is_download_link = parsed.path().ends_with("/uc");

    if !is_gdrive {
        return ParsedUrl {
            file_id: None,
            is_download_link,
        };
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

    ParsedUrl {
        file_id,
        is_download_link,
    }
}

#[must_use]
pub fn extract_resource_key(url: &str) -> Option<String> {
    Url::parse(url).ok().and_then(|u| {
        u.query_pairs()
            .find(|(k, _)| k.as_ref() == "resourcekey")
            .map(|(_, v)| v.into_owned())
    })
}

/// Extract the download URL from a Google Drive confirmation page.
///
/// # Errors
///
/// Returns an error if the page indicates the file is inaccessible or
/// the download URL cannot be extracted.
pub fn url_from_gdrive_confirmation(contents: &str) -> Result<String> {
    let form_selector = Selector::parse("#download-form")
        .map_err(|e| Error::Parse(format!("failed to parse selector: {e}")))?;
    let input_selector = Selector::parse(r#"input[type="hidden"]"#)
        .map_err(|e| Error::Parse(format!("failed to parse selector: {e}")))?;

    if let Some(caps) = uc_export_re().captures(contents)
        && let Some(m) = caps.get(1)
    {
        let url = format!("https://docs.google.com{}", m.as_str());
        let url = url.replace("&amp;", "&");
        return Ok(url);
    }

    let document = Html::parse_document(contents);
    if let Some(form) = document.select(&form_selector).next() {
        let action = form
            .value()
            .attr("action")
            .unwrap_or("")
            .replace("&amp;", "&");

        let parsed = Url::parse(&action).or_else(|_| {
            if action.starts_with('/') {
                Url::parse(&format!("https://docs.google.com{action}"))
            } else {
                Url::parse(&format!("https://docs.google.com/{action}"))
            }
        });

        let mut parsed = parsed.map_err(|e| {
            Error::FileUrlRetrieval(format!("failed to parse form action URL: {e}"))
        })?;

        let mut query_pairs: Vec<(String, String)> = parsed
            .query_pairs()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        for input in form.select(&input_selector) {
            if let (Some(name), Some(value)) =
                (input.value().attr("name"), input.value().attr("value"))
            {
                query_pairs.retain(|(k, _)| k != name);
                query_pairs.push((name.to_string(), value.to_string()));
            }
        }

        parsed.query_pairs_mut().clear().extend_pairs(&query_pairs);
        return Ok(parsed.to_string());
    }

    if let Some(caps) = download_url_re().captures(contents)
        && let Some(m) = caps.get(1)
    {
        let url = m.as_str().replace("\\u003d", "=").replace("\\u0026", "&");
        return Ok(url);
    }

    if let Some(caps) = error_subcaption_re().captures(contents)
        && let Some(m) = caps.get(1)
    {
        return Err(Error::FileUrlRetrieval(m.as_str().to_string()));
    }

    Err(Error::FileUrlRetrieval(
        "Cannot retrieve the public link of the file. \
         You may need to change the permission to \
         'Anyone with the link', or have had many accesses."
            .to_string(),
    ))
}

#[must_use]
pub fn filename_from_response(response: &Response) -> Option<String> {
    let content_disposition = response.headers().get("Content-Disposition")?;
    let cd_str = content_disposition.to_str().ok()?;
    filename_from_content_disposition(cd_str)
}

#[must_use]
pub fn filename_from_content_disposition(raw: &str) -> Option<String> {
    let decoded = urlencoding::decode(raw).ok()?;

    if let Some((_, rest)) = decoded.split_once("filename*=UTF-8''") {
        let filename = sanitize_filename(rest);
        if !filename.is_empty() {
            return Some(filename);
        }
    }

    if let Some((_, rest)) = decoded.split_once("attachment; filename=\"")
        && let Some((filename, _)) = rest.split_once('"')
    {
        let filename = sanitize_filename(filename);
        if !filename.is_empty() {
            return Some(filename);
        }
    }

    None
}

pub fn confirm_token_from_headers(headers: &HeaderMap) -> Option<String> {
    for value in &headers.get_all(SET_COOKIE) {
        let Ok(cookie) = value.to_str() else {
            continue;
        };

        if let Some((name, rest)) = cookie.split_once('=')
            && name.starts_with("download_warning")
        {
            let token = rest.split(';').next().unwrap_or("").trim();
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    None
}

pub fn confirm_token_from_html(html: &str) -> Option<String> {
    let html = html.replace("&amp;", "&");
    let needle = "confirm=";

    let mut tokens: Vec<String> = Vec::new();
    let mut search_start = 0usize;
    while let Some(relative_pos) = html.get(search_start..).and_then(|s| s.find(needle)) {
        let start = search_start
            .saturating_add(relative_pos)
            .saturating_add(needle.len());
        let Some(rest) = html.get(start..) else {
            break;
        };
        let end = rest
            .find(|c: char| c == '&' || c == '"' || c == '\'' || c.is_whitespace())
            .unwrap_or(rest.len());

        let token = rest.get(..end).unwrap_or("").trim();
        if !token.is_empty() {
            tokens.push(token.to_string());
        }

        search_start = start;
        if search_start >= html.len() {
            break;
        }
    }

    tokens.retain(|t| t != "t");
    tokens.sort_by_key(std::string::String::len);
    tokens.pop()
}

#[must_use]
pub fn title_from_html(html: &str) -> Option<String> {
    title_re()
        .captures(html)
        .and_then(|caps| caps.get(1))
        .map(|m| m.as_str().to_string())
}

/// A direct child entry parsed from a Google Drive folder page.
#[derive(Debug, Clone)]
pub struct FolderChild {
    pub id: String,
    pub name: String,
    pub mime_type: String,
}

/// Result of parsing a Google Drive folder page.
#[derive(Debug, Clone)]
pub struct ParsedFolderPage {
    pub folder_id: String,
    pub folder_name: String,
    pub children: Vec<FolderChild>,
}

/// Parse a Google Drive folder page to extract folder metadata and children.
///
/// # Errors
///
/// Returns an error if the page does not contain expected Google Drive
/// folder metadata, or if the metadata cannot be parsed.
pub fn parse_folder_page(url: &str, content: &str) -> Result<ParsedFolderPage> {
    let document = Html::parse_document(content);

    let encoded_data = extract_drive_ivd(&document)?;
    let decoded = decode_unicode_escapes(&encoded_data);
    let folder_arr: serde_json::Value = serde_json::from_str(&decoded)?;

    let folder_contents: Vec<serde_json::Value> = folder_arr
        .get(0)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let title_selector =
        Selector::parse("title").map_err(|e| Error::Parse(format!("invalid selector: {e}")))?;
    let title_text = document
        .select(&title_selector)
        .next()
        .map(|t| t.text().collect::<String>())
        .unwrap_or_default();

    let sep = "\u{00a0}-\u{00a0}";
    let folder_name = if let Some(pos) = title_text.rfind(sep) {
        title_text[..pos].to_string()
    } else if let Some(pos) = title_text.rfind(" - ") {
        title_text[..pos].to_string()
    } else {
        return Err(Error::Parse(format!(
            "folder name cannot be extracted from: {title_text}"
        )));
    };

    let folder_id = Url::parse(url)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back())
                .map(std::string::ToString::to_string)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            url.rsplit('/')
                .next()
                .and_then(|s| s.split('?').next())
                .unwrap_or("")
                .to_string()
        });

    let mut children = Vec::new();
    for entry in &folder_contents {
        if let Some(arr) = entry.as_array() {
            let child_id = arr.first().and_then(|v| v.as_str());
            let child_name = arr.get(2).and_then(|v| v.as_str());
            let child_type = arr.get(3).and_then(|v| v.as_str());

            if let (Some(id), Some(name), Some(mime)) = (child_id, child_name, child_type) {
                children.push(FolderChild {
                    id: id.to_string(),
                    name: name.to_string(),
                    mime_type: mime.to_string(),
                });
            }
        }
    }

    Ok(ParsedFolderPage {
        folder_id,
        folder_name,
        children,
    })
}

fn extract_drive_ivd(document: &Html) -> Result<String> {
    let script_selector =
        Selector::parse("script").map_err(|e| Error::Parse(format!("invalid selector: {e}")))?;

    for script in document.select(&script_selector) {
        let inner_html = script.inner_html();
        if !inner_html.contains("_DRIVE_ivd") {
            continue;
        }

        let strings: Vec<String> = drive_string_re()
            .captures_iter(&inner_html)
            .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
            .collect();

        for (idx, s) in strings.iter().enumerate() {
            if s != "_DRIVE_ivd" {
                continue;
            }

            for candidate in strings.iter().skip(idx.saturating_add(1)) {
                let decoded = decode_unicode_escapes(candidate);
                let parsed: serde_json::Value = match serde_json::from_str(&decoded) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if parsed.get(0).and_then(|v| v.as_array()).is_some() {
                    return Ok(candidate.clone());
                }
            }
        }
    }

    Err(Error::Parse(
        "Cannot retrieve the folder information from the link. \
         You may need to change the permission to \
         'Anyone with the link', or have had many accesses."
            .to_string(),
    ))
}

fn decode_unicode_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('x') => {
                    let mut hex = String::new();
                    if let Some(h1) = chars.next() {
                        hex.push(h1);
                    }
                    if let Some(h2) = chars.next() {
                        hex.push(h2);
                    }
                    if let Ok(code) = u8::from_str_radix(&hex, 16) {
                        result.push(code as char);
                    } else {
                        result.push('\\');
                        result.push('x');
                        result.push_str(&hex);
                    }
                }
                Some('u') => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        if let Some(h) = chars.next() {
                            hex.push(h);
                        }
                    }
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(code) {
                            result.push(c);
                        }
                    } else {
                        result.push('\\');
                        result.push('u');
                        result.push_str(&hex);
                    }
                }
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') | None => result.push('\\'),
                Some('\'') => result.push('\''),
                Some('"') => result.push('"'),
                Some('/') => result.push('/'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
            }
        } else {
            result.push(ch);
        }
    }

    result
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn test_parse_url() {
        let fid = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";

        let cases = vec![
            (
                format!("https://drive.google.com/open?id={fid}"),
                Some(fid),
                false,
            ),
            (
                format!("https://drive.google.com/uc?id={fid}"),
                Some(fid),
                true,
            ),
            (
                format!("https://drive.google.com/file/d/{fid}/view?usp=sharing"),
                Some(fid),
                false,
            ),
            (
                format!(
                    "https://drive.google.com/a/jsk.imi.i.u-tokyo.ac.jp/uc?id={fid}&export=download"
                ),
                Some(fid),
                true,
            ),
            (
                format!("https://docs.google.com/document/d/{fid}/edit"),
                Some(fid),
                false,
            ),
            (
                format!("https://docs.google.com/spreadsheets/d/{fid}/view"),
                Some(fid),
                false,
            ),
            (
                format!("https://docs.google.com/presentation/d/{fid}/view"),
                Some(fid),
                false,
            ),
            (
                format!("https://drive.google.com/file/u/0/d/{fid}/view"),
                Some(fid),
                false,
            ),
            ("https://example.com/somefile.zip".to_string(), None, false),
        ];

        for (url, expected_id, expected_dl) in cases {
            let result = parse_url(&url);
            assert_eq!(
                result.file_id.as_deref(),
                expected_id,
                "file_id mismatch for {url}"
            );
            assert_eq!(
                result.is_download_link, expected_dl,
                "is_download_link mismatch for {url}"
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
        assert!(!is_google_drive_url("https://google.com/drive"));
    }

    #[test]
    fn test_gdrive_confirmation_uc_export() {
        let html = r#"<a href="/uc?export=download&amp;id=ABCDEF&amp;confirm=t">Download</a>"#;
        let url = url_from_gdrive_confirmation(html).unwrap();
        assert!(url.starts_with("https://docs.google.com/uc?export=download"));
        assert!(url.contains("id=ABCDEF"));
        assert!(!url.contains("&amp;"));
    }

    #[test]
    fn test_gdrive_confirmation_download_url_json() {
        let html = r#"something "downloadUrl":"https://example.com/dl\u003did\u0026tok" else"#;
        let url = url_from_gdrive_confirmation(html).unwrap();
        assert_eq!(url, "https://example.com/dl=id&tok");
    }

    #[test]
    fn test_gdrive_confirmation_error_subcaption() {
        let html = r#"<p class="uc-error-subcaption">Sorry, quota exceeded</p>"#;
        let result = url_from_gdrive_confirmation(html);
        assert!(result.is_err());
        if let Err(Error::FileUrlRetrieval(msg)) = result {
            assert!(msg.contains("Sorry, quota exceeded"));
        }
    }

    #[test]
    fn test_gdrive_confirmation_empty_page() {
        let html = "<html><body>Nothing useful here</body></html>";
        let result = url_from_gdrive_confirmation(html);
        assert!(result.is_err());
        if let Err(Error::FileUrlRetrieval(msg)) = result {
            assert!(msg.contains("Cannot retrieve the public link"));
        }
    }

    #[test]
    fn test_gdrive_confirmation_download_form() {
        let html = concat!(
            r#"<form id="download-form" action="https://drive.usercontent.google.com/download?id=FILEID&amp;export=download">"#,
            r#"<input type="hidden" name="confirm" value="t">"#,
            r#"<input type="hidden" name="uuid" value="abc-123">"#,
            r#"</form>"#
        );
        let url = url_from_gdrive_confirmation(html).unwrap();
        assert!(url.contains("id=FILEID"));
        assert!(url.contains("confirm=t"));
        assert!(url.contains("uuid=abc-123"));
    }

    #[test]
    fn test_confirm_token_from_html_basic() {
        let html = r#"href="...&confirm=abc123&other=...""#;
        let token = confirm_token_from_html(html);
        assert_eq!(token, Some("abc123".to_string()));
    }

    #[test]
    fn test_confirm_token_from_html_filters_t() {
        let html = r"confirm=t&other confirm=realtoken&x";
        let token = confirm_token_from_html(html);
        assert_eq!(token, Some("realtoken".to_string()));
    }

    #[test]
    fn test_confirm_token_from_html_none() {
        let html = "no tokens here";
        assert!(confirm_token_from_html(html).is_none());
    }

    #[test]
    fn test_decode_unicode_escapes_hex() {
        assert_eq!(decode_unicode_escapes(r"\x5b\x5d"), "[]");
        assert_eq!(decode_unicode_escapes(r"\x22hello\x22"), "\"hello\"");
    }

    #[test]
    fn test_decode_unicode_escapes_unicode() {
        assert_eq!(decode_unicode_escapes(r"\u0041"), "A");
        assert_eq!(decode_unicode_escapes(r"\u00e9"), "é");
    }

    #[test]
    fn test_decode_unicode_escapes_special() {
        assert_eq!(decode_unicode_escapes(r"\n"), "\n");
        assert_eq!(decode_unicode_escapes(r"\r"), "\r");
        assert_eq!(decode_unicode_escapes(r"\t"), "\t");
        assert_eq!(decode_unicode_escapes(r"\\"), "\\");
        assert_eq!(decode_unicode_escapes(r"\'"), "'");
        assert_eq!(decode_unicode_escapes(r#"\""#), "\"");
        assert_eq!(decode_unicode_escapes(r"\/"), "/");
    }

    #[test]
    fn test_decode_unicode_escapes_passthrough() {
        assert_eq!(decode_unicode_escapes("plain text"), "plain text");
        assert_eq!(decode_unicode_escapes(r"\z"), "\\z");
    }

    #[test]
    fn test_decode_unicode_escapes_malformed() {
        assert_eq!(decode_unicode_escapes(r"\xZZ"), "\\xZZ");
        assert_eq!(decode_unicode_escapes(r"\uZZZZ"), "\\uZZZZ");
    }

    #[test]
    fn test_parse_folder_page() {
        let html_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/folder-page-sample.html"
        );
        let content = std::fs::read_to_string(html_path).unwrap();
        let folder_url = "https://drive.google.com/drive/folders/1KpLl_1tcK0eeehzN980zbG-3M2nhbVks";

        let parsed = parse_folder_page(folder_url, &content).unwrap();

        assert_eq!(parsed.folder_id, "1KpLl_1tcK0eeehzN980zbG-3M2nhbVks");
        assert_eq!(parsed.folder_name, "gdown_folder_test");
        assert_eq!(parsed.children.len(), 5);

        let ids: Vec<&str> = parsed.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "1aMZqPaU03E7XOQNXtjSCdguRHBaIQ82m",
                "1hVAxfM7_doToqQ24eVd65cgiaoLi0TtO",
                "1Z2VYnXb01h-3uvEptoQ48Fo__eAn0wc1",
                "14xzOzvKjP0at07jfonV7qVrTKoctFijz",
                "1wlapSEt6N9Ayf7fzCTOkra_4GIg-cqeD",
            ]
        );

        let names: Vec<&str> = parsed.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "directory-0",
                "directory-1",
                "fractal.jpg",
                "this is a file.txt",
                "tux.jpg",
            ]
        );
    }

    #[test]
    fn test_extract_resource_key() {
        let url = "https://drive.google.com/drive/folders/abc?resourcekey=xyz";
        assert_eq!(extract_resource_key(url), Some("xyz".to_string()));

        let url = "https://drive.google.com/drive/folders/abc";
        assert_eq!(extract_resource_key(url), None);
    }

    #[test]
    fn test_filename_from_content_disposition_utf8() {
        let cd = "attachment; filename*=UTF-8''my%20file.txt";
        assert_eq!(
            filename_from_content_disposition(cd),
            Some("my file.txt".to_string())
        );
    }

    #[test]
    fn test_filename_from_content_disposition_quoted() {
        let cd = r#"attachment; filename="report.pdf""#;
        assert_eq!(
            filename_from_content_disposition(cd),
            Some("report.pdf".to_string())
        );
    }

    #[test]
    fn test_filename_from_content_disposition_sanitizes() {
        let cd = r#"attachment; filename="../../etc/passwd""#;
        assert_eq!(
            filename_from_content_disposition(cd),
            Some(".._.._etc_passwd".to_string())
        );
    }

    #[test]
    fn test_filename_from_content_disposition_none() {
        assert!(filename_from_content_disposition("inline").is_none());
        assert!(filename_from_content_disposition("").is_none());
    }

    #[test]
    fn test_filename_from_content_disposition_url_encoded() {
        let cd = "attachment; filename*=UTF-8''hello%20world%21.txt";
        assert_eq!(
            filename_from_content_disposition(cd),
            Some("hello world!.txt".to_string())
        );
    }
}
