use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use scraper::{Html, Selector};

use crate::download::{build_client, download, DownloadOptions, DEFAULT_FOLDER_USER_AGENT};
use crate::error::{Error, Result};
use crate::parse_url::is_google_drive_url;

pub const MAX_NUMBER_FILES: usize = 50;

const FOLDER_TYPE: &str = "application/vnd.google-apps.folder";

/// Represents a file or folder in Google Drive.
#[derive(Debug, Clone)]
pub struct GoogleDriveFile {
    pub id: String,
    pub name: String,
    pub file_type: String,
    pub children: Vec<GoogleDriveFile>,
}

impl GoogleDriveFile {
    pub fn new(id: String, name: String, file_type: String) -> Self {
        Self {
            id,
            name,
            file_type,
            children: Vec::new(),
        }
    }

    pub fn is_folder(&self) -> bool {
        self.file_type == FOLDER_TYPE
    }
}

/// Represents a file to download from Google Drive (for skip_download / dry-run mode).
#[derive(Debug, Clone)]
pub struct GoogleDriveFileToDownload {
    pub id: String,
    pub path: String,
    pub local_path: String,
}

/// Extracts information about the current page file and its children.
///
/// Returns `(gdrive_file, id_name_type_iter)` where `id_name_type_iter` is a
/// vec of `(id, name, type)` tuples for the folder's direct children.
pub fn parse_google_drive_file(
    url: &str,
    content: &str,
) -> Result<(GoogleDriveFile, Vec<(String, String, String)>)> {
    let document = Html::parse_document(content);

    // Find the script tag with window['_DRIVE_ivd']
    let script_selector = Selector::parse("script").unwrap();
    let mut encoded_data: Option<String> = None;

    for script in document.select(&script_selector) {
        let inner_html = script.inner_html();
        if inner_html.contains("_DRIVE_ivd") {
            // Extract the second JS string (the encoded array)
            let re = Regex::new(r"'((?:[^'\\]|\\.)*)'").unwrap();
            let matches: Vec<_> = re.captures_iter(&inner_html).collect();
            if matches.len() >= 2 {
                encoded_data = Some(matches[1][1].to_string());
            } else {
                return Err(Error::ParseError(
                    "Couldn't find the folder encoded JS string".to_string(),
                ));
            }
            break;
        }
    }

    let encoded_data = encoded_data.ok_or_else(|| {
        Error::ParseError(
            "Cannot retrieve the folder information from the link. \
             You may need to change the permission to \
             'Anyone with the link', or have had many accesses. \
             Check FAQ in https://github.com/wkentaro/gdown?tab=readme-ov-file#faq."
                .to_string(),
        )
    })?;

    // Decode the unicode escape sequences
    let decoded = decode_unicode_escapes(&encoded_data);

    // Parse the decoded string as JSON
    let folder_arr: serde_json::Value = serde_json::from_str(&decoded)?;

    let folder_contents = if folder_arr[0].is_null() {
        vec![]
    } else {
        folder_arr[0]
            .as_array()
            .cloned()
            .unwrap_or_default()
    };

    // Extract folder name from title
    let title_selector = Selector::parse("title").unwrap();
    let title_text = document
        .select(&title_selector)
        .next()
        .map(|t| t.text().collect::<String>())
        .unwrap_or_default();

    let sep = "\u{00a0}-\u{00a0}"; // unicode non-breaking space dash
    // Also try regular " - "
    let name = if let Some(pos) = title_text.rfind(sep) {
        title_text[..pos].to_string()
    } else if let Some(pos) = title_text.rfind(" - ") {
        title_text[..pos].to_string()
    } else {
        return Err(Error::ParseError(format!(
            "file/folder name cannot be extracted from: {}",
            title_text
        )));
    };

    // Extract folder ID from URL
    let folder_id = url.split('/').last().unwrap_or("").to_string();
    // Strip query params from folder_id
    let folder_id = folder_id.split('?').next().unwrap_or("").to_string();

    let gdrive_file = GoogleDriveFile::new(folder_id, name, FOLDER_TYPE.to_string());

    // Extract children info: (id, name, type)
    let mut id_name_type_iter = Vec::new();
    for entry in &folder_contents {
        if let Some(arr) = entry.as_array() {
            if arr.len() >= 4 {
                let child_id = arr[0].as_str().unwrap_or("").to_string();
                let child_name_raw = arr[2].as_str().unwrap_or("").to_string();
                // The Python code does: e[2].encode("raw_unicode_escape").decode("utf-8")
                // In Rust, JSON parsing already handles unicode, so we just use the string directly
                let child_name = child_name_raw;
                let child_type = arr[3].as_str().unwrap_or("").to_string();
                id_name_type_iter.push((child_id, child_name, child_type));
            }
        }
    }

    Ok((gdrive_file, id_name_type_iter))
}

/// Decode JavaScript unicode escape sequences like \x5b, \x22, etc.
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
                Some('\\') => result.push('\\'),
                Some('\'') => result.push('\''),
                Some('"') => result.push('"'),
                Some('/') => result.push('/'),
                Some(other) => {
                    result.push('\\');
                    result.push(other);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }

    result
}

/// Get folder structure of Google Drive folder URL.
fn download_and_parse_google_drive_link(
    client: &reqwest::blocking::Client,
    url: &str,
    quiet: bool,
    remaining_ok: bool,
    verify: bool,
) -> Result<Option<GoogleDriveFile>> {
    let mut url = url.to_string();

    for _ in 0..2 {
        if is_google_drive_url(&url) {
            if url.contains('?') {
                url = format!("{}&hl=en", url);
            } else {
                url = format!("{}?hl=en", url);
            }
        }

        let res = client.get(&url).send()?;
        if res.status().as_u16() != 200 {
            return Ok(None);
        }

        let response_url = res.url().to_string();
        let text = res.text()?;

        if is_google_drive_url(&url) {
            // Parse and process
            let (mut gdrive_file, id_name_type_iter) =
                parse_google_drive_file(&url, &text)?;

            process_children(
                client,
                &mut gdrive_file,
                id_name_type_iter,
                quiet,
                remaining_ok,
                verify,
            )?;

            return Ok(Some(gdrive_file));
        }

        if !is_google_drive_url(&response_url) {
            let (mut gdrive_file, id_name_type_iter) =
                parse_google_drive_file(&url, &text)?;

            process_children(
                client,
                &mut gdrive_file,
                id_name_type_iter,
                quiet,
                remaining_ok,
                verify,
            )?;

            return Ok(Some(gdrive_file));
        }

        // Retry with canonical URL
        url = response_url;
    }

    Ok(None)
}

fn process_children(
    client: &reqwest::blocking::Client,
    gdrive_file: &mut GoogleDriveFile,
    id_name_type_iter: Vec<(String, String, String)>,
    quiet: bool,
    remaining_ok: bool,
    verify: bool,
) -> Result<()> {
    for (child_id, child_name, child_type) in id_name_type_iter {
        if child_type != FOLDER_TYPE {
            if !quiet {
                eprintln!("Processing file {} {}", child_id, child_name);
            }
            gdrive_file.children.push(GoogleDriveFile::new(
                child_id,
                child_name,
                child_type,
            ));
            continue;
        }

        if !quiet {
            eprintln!("Retrieving folder {} {}", child_id, child_name);
        }
        let child_url = format!("https://drive.google.com/drive/folders/{}", child_id);
        match download_and_parse_google_drive_link(client, &child_url, quiet, remaining_ok, verify)?
        {
            Some(child) => {
                gdrive_file.children.push(child);
            }
            None => {
                return Ok(());
            }
        }
    }

    if !remaining_ok && gdrive_file.children.len() == MAX_NUMBER_FILES {
        return Err(Error::FolderContentsMaximumLimit(format!(
            "The gdrive folder has more than {} files, \
             gdrive can't download more than this limit.",
            MAX_NUMBER_FILES
        )));
    }

    Ok(())
}

/// Converts a Google Drive folder structure into a local directory list.
///
/// Returns a vec of `(Option<file_id>, relative_path)`.
/// If file_id is None, it's a directory entry.
pub fn get_directory_structure(
    gdrive_file: &GoogleDriveFile,
    previous_path: &str,
) -> Vec<(Option<String>, String)> {
    let mut structure = Vec::new();
    for file in &gdrive_file.children {
        let safe_name = file.name.replace(std::path::MAIN_SEPARATOR, "_");
        let path = if previous_path.is_empty() {
            safe_name.clone()
        } else {
            format!(
                "{}{}{}",
                previous_path,
                std::path::MAIN_SEPARATOR,
                safe_name
            )
        };

        if file.is_folder() {
            structure.push((None, path.clone()));
            structure.extend(get_directory_structure(file, &path));
        } else if file.children.is_empty() {
            structure.push((Some(file.id.clone()), path));
        }
    }
    structure
}

/// Options for downloading a folder.
#[derive(Debug, Clone)]
pub struct DownloadFolderOptions {
    pub url: Option<String>,
    pub id: Option<String>,
    pub output: Option<String>,
    pub quiet: bool,
    pub proxy: Option<String>,
    pub speed: Option<f64>,
    pub use_cookies: bool,
    pub remaining_ok: bool,
    pub verify: bool,
    pub user_agent: Option<String>,
    pub skip_download: bool,
    pub resume: bool,
}

impl Default for DownloadFolderOptions {
    fn default() -> Self {
        Self {
            url: None,
            id: None,
            output: None,
            quiet: false,
            proxy: None,
            speed: None,
            use_cookies: true,
            remaining_ok: false,
            verify: true,
            user_agent: None,
            skip_download: false,
            resume: false,
        }
    }
}

/// Result of a folder download.
#[derive(Debug)]
pub enum DownloadFolderResult {
    /// List of local file paths that were downloaded.
    Downloaded(Vec<String>),
    /// List of files to download (dry-run mode).
    DryRun(Vec<GoogleDriveFileToDownload>),
}

/// Downloads an entire folder from a Google Drive URL.
pub fn download_folder(opts: &DownloadFolderOptions) -> Result<Option<DownloadFolderResult>> {
    let has_url = opts.url.is_some();
    let has_id = opts.id.is_some();
    if !(has_id ^ has_url) {
        return Err(Error::InvalidInput(
            "Either url or id has to be specified".to_string(),
        ));
    }

    let url = if let Some(ref id) = opts.id {
        format!("https://drive.google.com/drive/folders/{}", id)
    } else {
        opts.url.clone().unwrap()
    };

    let user_agent = opts
        .user_agent
        .as_deref()
        .unwrap_or(DEFAULT_FOLDER_USER_AGENT);

    let client = build_client(
        opts.proxy.as_deref(),
        opts.use_cookies,
        user_agent,
        opts.verify,
    )?;

    if !opts.quiet {
        eprintln!("Retrieving folder contents");
    }

    let gdrive_file = download_and_parse_google_drive_link(
        &client,
        &url,
        opts.quiet,
        opts.remaining_ok,
        opts.verify,
    )?;

    let gdrive_file = match gdrive_file {
        Some(f) => f,
        None => {
            eprintln!("Failed to retrieve folder contents");
            return Ok(None);
        }
    };

    if !opts.quiet {
        eprintln!("Retrieving folder contents completed");
        eprintln!("Building directory structure");
    }

    let directory_structure = get_directory_structure(&gdrive_file, "");

    if !opts.quiet {
        eprintln!("Building directory structure completed");
    }

    let output = opts
        .output
        .clone()
        .unwrap_or_else(|| format!(".{}", std::path::MAIN_SEPARATOR));

    let root_dir = if output.ends_with(std::path::MAIN_SEPARATOR) {
        Path::new(&output).join(&gdrive_file.name)
    } else {
        PathBuf::from(&output)
    };

    if !opts.skip_download && !root_dir.exists() {
        fs::create_dir_all(&root_dir)?;
    }

    if opts.skip_download {
        let mut files = Vec::new();
        for (id, path) in &directory_structure {
            if let Some(file_id) = id {
                let local_path = root_dir.join(path);
                files.push(GoogleDriveFileToDownload {
                    id: file_id.clone(),
                    path: path.clone(),
                    local_path: local_path.to_string_lossy().to_string(),
                });
            }
        }
        return Ok(Some(DownloadFolderResult::DryRun(files)));
    }

    let mut files: Vec<String> = Vec::new();
    for (id, path) in &directory_structure {
        let local_path = root_dir.join(path);

        if id.is_none() {
            // It's a folder
            if !local_path.exists() {
                fs::create_dir_all(&local_path)?;
            }
            continue;
        }

        let file_id = id.as_ref().unwrap();

        if opts.resume && local_path.is_file() {
            if !opts.quiet {
                eprintln!("Skipping already downloaded file {}", local_path.display());
            }
            files.push(local_path.to_string_lossy().to_string());
            continue;
        }

        let file_opts = DownloadOptions {
            url: Some(format!("https://drive.google.com/uc?id={}", file_id)),
            output: Some(local_path.to_string_lossy().to_string()),
            quiet: opts.quiet,
            proxy: opts.proxy.clone(),
            speed: opts.speed,
            use_cookies: opts.use_cookies,
            verify: opts.verify,
            resume: opts.resume,
            ..DownloadOptions::default()
        };

        match download(&file_opts)? {
            Some(path) => files.push(path),
            None => {
                if !opts.quiet {
                    eprintln!("Download ended unsuccessfully");
                }
                return Ok(None);
            }
        }
    }

    if !opts.quiet {
        eprintln!("Download completed");
    }

    Ok(Some(DownloadFolderResult::Downloaded(files)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_page() {
        let html_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/folder-page-sample.html"
        );
        let content = std::fs::read_to_string(html_path).unwrap();
        let folder_url =
            "https://drive.google.com/drive/folders/1KpLl_1tcK0eeehzN980zbG-3M2nhbVks";

        let (gdrive_file, id_name_type_iter) =
            parse_google_drive_file(folder_url, &content).unwrap();

        assert_eq!(gdrive_file.id, "1KpLl_1tcK0eeehzN980zbG-3M2nhbVks");
        assert_eq!(gdrive_file.name, "gdown_folder_test");
        assert_eq!(gdrive_file.file_type, "application/vnd.google-apps.folder");
        assert!(gdrive_file.children.is_empty());
        assert!(gdrive_file.is_folder());

        let expected_children_ids = vec![
            "1aMZqPaU03E7XOQNXtjSCdguRHBaIQ82m",
            "1hVAxfM7_doToqQ24eVd65cgiaoLi0TtO",
            "1Z2VYnXb01h-3uvEptoQ48Fo__eAn0wc1",
            "14xzOzvKjP0at07jfonV7qVrTKoctFijz",
            "1wlapSEt6N9Ayf7fzCTOkra_4GIg-cqeD",
        ];

        let expected_children_names = vec![
            "directory-0",
            "directory-1",
            "fractal.jpg",
            "this is a file.txt",
            "tux.jpg",
        ];

        let expected_children_types = vec![
            "application/vnd.google-apps.folder",
            "application/vnd.google-apps.folder",
            "image/jpeg",
            "text/plain",
            "image/jpeg",
        ];

        let actual_ids: Vec<&str> = id_name_type_iter.iter().map(|(id, _, _)| id.as_str()).collect();
        let actual_names: Vec<&str> = id_name_type_iter
            .iter()
            .map(|(_, name, _)| name.as_str())
            .collect();
        let actual_types: Vec<&str> = id_name_type_iter
            .iter()
            .map(|(_, _, t)| t.as_str())
            .collect();

        assert_eq!(actual_ids, expected_children_ids);
        assert_eq!(actual_names, expected_children_names);
        assert_eq!(actual_types, expected_children_types);
    }

    #[test]
    fn test_decode_unicode_escapes() {
        assert_eq!(decode_unicode_escapes(r"\x5b\x5b"), "[[");
        assert_eq!(decode_unicode_escapes(r"\x22hello\x22"), "\"hello\"");
        assert_eq!(decode_unicode_escapes(r"no escapes"), "no escapes");
    }
}
