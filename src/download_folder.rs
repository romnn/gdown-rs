use std::collections::HashMap;
use std::path::{Path, PathBuf};

use regex::Regex;
use scraper::{Html, Selector};
use url::Url;

use crate::download::{DEFAULT_FOLDER_USER_AGENT, DownloadOptions, build_client, download};
use crate::error::{Error, Result};
use crate::parse_url::is_google_drive_url;

pub const MAX_NUMBER_FILES: usize = 50;

type ChildInfo = (String, String, String);
type ChildInfoList = Vec<ChildInfo>;
type ParsedGoogleDriveFolder = (GoogleDriveFile, ChildInfoList);

const FOLDER_TYPE: &str = "application/vnd.google-apps.folder";

// ── Existing types ─────────────────────────────────────────────────

/// Represents a file or folder in Google Drive.
#[derive(Debug, Clone)]
pub struct GoogleDriveFile {
    pub id: String,
    pub name: String,
    pub file_type: String,
    pub children: Vec<GoogleDriveFile>,
}

impl GoogleDriveFile {
    #[must_use]
    pub fn new(id: String, name: String, file_type: String) -> Self {
        Self {
            id,
            name,
            file_type,
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn is_folder(&self) -> bool {
        self.file_type == FOLDER_TYPE
    }
}

/// Represents a file to download from Google Drive (for `skip_download` / dry-run mode).
#[derive(Debug, Clone)]
pub struct GoogleDriveFileToDownload {
    pub id: String,
    pub path: String,
    pub local_path: String,
}

// ── Sync parsing functions ─────────────────────────────────────────

/// Extracts information about the current page file and its children.
///
/// Returns `(gdrive_file, id_name_type_iter)` where `id_name_type_iter` is a
/// vec of `(id, name, type)` tuples for the folder's direct children.
///
/// # Errors
///
/// Returns an error if the page content does not contain expected Google Drive folder
/// metadata or if the metadata cannot be parsed.
#[allow(
    clippy::too_many_lines,
    reason = "Function contains parsing logic that is clearer in one place; splitting would add indirection without reducing complexity"
)]
pub fn parse_google_drive_file(url: &str, content: &str) -> Result<ParsedGoogleDriveFolder> {
    let document = Html::parse_document(content);

    // Find the script tag with window['_DRIVE_ivd']
    let script_selector = Selector::parse("script")
        .map_err(|e| Error::ParseError(format!("Failed to parse selector: {e}")))?;
    let drive_string_re = Regex::new(r"'((?:[^'\\]|\\.)*)'")
        .map_err(|e| Error::ParseError(format!("Failed to compile regex: {e}")))?;
    let mut encoded_data: Option<String> = None;

    for script in document.select(&script_selector) {
        let inner_html = script.inner_html();
        if inner_html.contains("_DRIVE_ivd") {
            let strings: Vec<String> = drive_string_re
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
                        encoded_data = Some(candidate.clone());
                        break;
                    }
                }

                if encoded_data.is_some() {
                    break;
                }
            }

            if encoded_data.is_some() {
                break;
            }
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

    let folder_contents: Vec<serde_json::Value> = folder_arr
        .get(0)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Extract folder name from title
    let title_selector = Selector::parse("title")
        .map_err(|e| Error::ParseError(format!("Failed to parse selector: {e}")))?;
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
            "file/folder name cannot be extracted from: {title_text}"
        )));
    };

    // Extract folder ID from URL
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

    let gdrive_file = GoogleDriveFile::new(folder_id, name, FOLDER_TYPE.to_string());

    // Extract children info: (id, name, type)
    let mut id_name_type_iter = Vec::new();
    for entry in &folder_contents {
        if let Some(arr) = entry.as_array() {
            let child_id = arr.first().and_then(|v| v.as_str());
            let child_name = arr.get(2).and_then(|v| v.as_str());
            let child_type = arr.get(3).and_then(|v| v.as_str());

            if let (Some(child_id), Some(child_name), Some(child_type)) =
                (child_id, child_name, child_type)
            {
                id_name_type_iter.push((
                    child_id.to_string(),
                    child_name.to_string(),
                    child_type.to_string(),
                ));
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

/// Converts a Google Drive folder structure into a local directory list.
///
/// Returns a vec of `(Option<file_id>, relative_path)`.
/// If `file_id` is None, it's a directory entry.
#[must_use]
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

// ── DriveFolder: lazy, cached folder listing ───────────────────────

/// An entry in a Google Drive folder listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub id: String,
    pub name: String,
    pub mime_type: String,
}

impl DirEntry {
    /// Whether this entry is a folder.
    #[must_use]
    pub fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_TYPE
    }
}

/// A flat entry with full relative path, for recursive listings.
#[derive(Debug, Clone)]
pub struct FlatEntry {
    pub id: String,
    pub path: String,
    pub is_folder: bool,
}

/// A handle to a Google Drive shared folder with lazy, cached resolution.
///
/// Created cheaply via [`open()`](Self::open) (no HTTP). Directory listings
/// are fetched on demand one level at a time and cached for reuse.
pub struct DriveFolder {
    root_id: String,
    resource_key: Option<String>,
    client: reqwest::Client,
    cache: tokio::sync::Mutex<HashMap<String, Vec<DirEntry>>>,
}

impl DriveFolder {
    /// Open a Google Drive folder URL.
    ///
    /// Parses the folder ID and resource key from the URL and builds an HTTP
    /// client. **No HTTP requests are made.**
    ///
    /// # Errors
    ///
    /// Returns an error if the URL cannot be parsed or the folder ID cannot
    /// be extracted.
    pub fn open(url: &str) -> Result<Self> {
        let client = build_client(None, true, DEFAULT_FOLDER_USER_AGENT, true)?;
        Self::open_with_client(url, client)
    }

    /// Open with a pre-built HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL cannot be parsed or the folder ID cannot
    /// be extracted.
    pub fn open_with_client(url: &str, client: reqwest::Client) -> Result<Self> {
        let parsed =
            Url::parse(url).map_err(|e| Error::InvalidInput(format!("invalid folder URL: {e}")))?;

        let root_id = parsed
            .path_segments()
            .and_then(|mut s| s.next_back())
            .map(std::string::ToString::to_string)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                Error::InvalidInput(format!("cannot extract folder ID from URL: {url}"))
            })?;

        let resource_key = parsed
            .query_pairs()
            .find(|(k, _)| k.as_ref() == "resourcekey")
            .map(|(_, v)| v.into_owned());

        Ok(Self {
            root_id,
            resource_key,
            client,
            cache: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// List direct children of a folder by ID (cached after first fetch).
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be parsed.
    pub async fn list(&self, folder_id: &str) -> Result<Vec<DirEntry>> {
        // Check cache
        {
            let cache = self.cache.lock().await;
            if let Some(entries) = cache.get(folder_id) {
                return Ok(entries.clone());
            }
        }

        let entries = self.fetch_children(folder_id).await?;
        self.cache
            .lock()
            .await
            .insert(folder_id.to_string(), entries.clone());
        Ok(entries)
    }

    /// List direct children of the root folder.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be parsed.
    pub async fn list_root(&self) -> Result<Vec<DirEntry>> {
        self.list(&self.root_id).await
    }

    /// Resolve a `/`-separated path to its [`DirEntry`], fetching one
    /// directory level at a time.
    ///
    /// # Errors
    ///
    /// Returns an error if any path component is not found or an intermediate
    /// component is not a folder.
    pub async fn resolve(&self, path: &str) -> Result<DirEntry> {
        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if components.is_empty() {
            return Err(Error::InvalidInput("empty path".to_string()));
        }

        let mut current_folder_id = self.root_id.clone();

        for (i, component) in components.iter().enumerate() {
            let children = self.list(&current_folder_id).await?;
            let child = children
                .iter()
                .find(|c| c.name == *component)
                .ok_or_else(|| {
                    Error::ParseError(format!(
                        "not found in Google Drive folder: {component} (in path {path})"
                    ))
                })?
                .clone();

            let is_last = i == components.len().saturating_sub(1);
            if is_last {
                return Ok(child);
            }

            if !child.is_folder() {
                return Err(Error::ParseError(format!(
                    "{component} is not a folder (in path {path})"
                )));
            }
            current_folder_id = child.id;
        }

        Err(Error::ParseError(format!("unexpected end of path: {path}")))
    }

    /// Recursively list all files and folders, populating the cache.
    ///
    /// # Errors
    ///
    /// Returns an error if any folder page cannot be fetched or parsed.
    pub async fn list_recursive(&self) -> Result<Vec<FlatEntry>> {
        let mut result = Vec::new();
        // Iterative DFS to avoid recursive async
        let mut stack: Vec<(String, String)> = vec![(self.root_id.clone(), String::new())];

        while let Some((folder_id, prefix)) = stack.pop() {
            let children = self.list(&folder_id).await?;
            for child in children {
                let path = if prefix.is_empty() {
                    child.name.clone()
                } else {
                    format!("{prefix}/{}", child.name)
                };

                let is_folder = child.is_folder();
                result.push(FlatEntry {
                    id: child.id.clone(),
                    path: path.clone(),
                    is_folder,
                });

                if is_folder {
                    stack.push((child.id, path));
                }
            }
        }

        Ok(result)
    }

    /// Resolve a remote path and download the file.
    ///
    /// Creates parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be resolved or the download fails.
    pub async fn download(&self, remote_path: &str, output: &Path) -> Result<()> {
        let entry = self.resolve(remote_path).await?;
        self.download_by_id(&entry.id, output).await
    }

    /// Download a file by its Google Drive file ID.
    ///
    /// Creates parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the download fails.
    pub async fn download_by_id(&self, id: &str, output: &Path) -> Result<()> {
        if let Some(parent) = output.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        tracing::info!(id = id, output = %output.display(), "downloading from Google Drive");

        download(&DownloadOptions {
            id: Some(id.to_string()),
            output: Some(output.to_string_lossy().to_string()),
            quiet: false,
            ..DownloadOptions::default()
        })
        .await?;

        Ok(())
    }

    /// Download all files directly under a remote directory prefix.
    ///
    /// Resolves the prefix to a folder, lists its direct children, and
    /// downloads each file child into `dest_dir`.
    ///
    /// # Errors
    ///
    /// Returns an error if the prefix cannot be resolved, is not a folder,
    /// or any download fails.
    pub async fn download_prefix(&self, prefix: &str, dest_dir: &Path) -> Result<Vec<PathBuf>> {
        let dir_entry = self.resolve(prefix).await?;
        if !dir_entry.is_folder() {
            return Err(Error::InvalidInput(format!("{prefix} is not a directory")));
        }

        tokio::fs::create_dir_all(dest_dir).await?;

        let children = self.list(&dir_entry.id).await?;
        let mut paths = Vec::new();

        for child in &children {
            if child.is_folder() {
                continue;
            }
            let local_path = dest_dir.join(&child.name);
            self.download_by_id(&child.id, &local_path).await?;
            paths.push(local_path);
        }

        Ok(paths)
    }

    /// The root folder ID extracted from the URL.
    #[must_use]
    pub fn root_id(&self) -> &str {
        &self.root_id
    }

    /// Fetch children for a single folder ID (one HTTP request).
    async fn fetch_children(&self, folder_id: &str) -> Result<Vec<DirEntry>> {
        let mut url = format!("https://drive.google.com/drive/folders/{folder_id}");
        if let Some(ref rk) = self.resource_key {
            url = format!("{url}?resourcekey={rk}");
        }

        // Retry once to handle URL redirects
        for _ in 0..2 {
            let fetch_url = if is_google_drive_url(&url) {
                if url.contains('?') {
                    format!("{url}&hl=en")
                } else {
                    format!("{url}?hl=en")
                }
            } else {
                url.clone()
            };

            let res = self.client.get(&fetch_url).send().await?;
            if res.status().as_u16() != 200 {
                return Err(Error::ParseError(format!(
                    "folder listing returned HTTP {}",
                    res.status()
                )));
            }

            let response_url = res.url().to_string();
            let text = res.text().await?;

            if is_google_drive_url(&fetch_url) || !is_google_drive_url(&response_url) {
                let (_, children) = parse_google_drive_file(&fetch_url, &text)?;
                return Ok(children
                    .into_iter()
                    .map(|(id, name, mime_type)| DirEntry {
                        id,
                        name,
                        mime_type,
                    })
                    .collect());
            }

            // Redirected to a different Google Drive URL — retry with canonical URL
            url = response_url;
        }

        Err(Error::ParseError(format!(
            "failed to list folder {folder_id} after retries"
        )))
    }
}

// ── Legacy folder download ─────────────────────────────────────────

/// Options for downloading a folder.
#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "Mirrors upstream gdown CLI flags; refactoring into enums/state machines would be a breaking API change"
)]
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
///
/// # Errors
///
/// Returns an error if input options are invalid, network/HTTP requests fail,
/// folder contents cannot be parsed, or local filesystem operations fail.
#[allow(
    clippy::too_many_lines,
    reason = "Function is a linear orchestration of folder download flow; splitting further would add indirection without improving clarity"
)]
#[tracing::instrument(
    name = "download_folder",
    level = "info",
    skip(opts),
    fields(
        url = ?opts.url,
        id = ?opts.id,
        output = ?opts.output,
        quiet = opts.quiet,
        proxy = ?opts.proxy,
        use_cookies = opts.use_cookies,
        remaining_ok = opts.remaining_ok,
        verify = opts.verify,
        skip_download = opts.skip_download,
        resume = opts.resume
    )
)]
pub async fn download_folder(opts: &DownloadFolderOptions) -> Result<Option<DownloadFolderResult>> {
    let has_url = opts.url.is_some();
    let has_id = opts.id.is_some();
    if !(has_id ^ has_url) {
        return Err(Error::InvalidInput(
            "Either url or id has to be specified".to_string(),
        ));
    }

    let url = match (&opts.id, &opts.url) {
        (Some(id), None) => format!("https://drive.google.com/drive/folders/{id}"),
        (None, Some(u)) => u.clone(),
        _ => {
            return Err(Error::InvalidInput(
                "Either url or id has to be specified".to_string(),
            ));
        }
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

    let folder = DriveFolder::open_with_client(&url, client)?;

    if !opts.quiet {
        tracing::info!("retrieving folder contents");
    }

    let entries = folder.list_recursive().await?;

    if !opts.quiet {
        tracing::info!(file_count = entries.len(), "folder contents retrieved");
    }

    if !opts.remaining_ok {
        let file_count = entries.iter().filter(|e| !e.is_folder).count();
        if file_count >= MAX_NUMBER_FILES {
            return Err(Error::FolderContentsMaximumLimit(format!(
                "The gdrive folder has more than {MAX_NUMBER_FILES} files, \
                 gdrive can't download more than this limit."
            )));
        }
    }

    let output = opts
        .output
        .clone()
        .unwrap_or_else(|| format!(".{}", std::path::MAIN_SEPARATOR));

    // For the root directory name, list root to get context (folder name is
    // not directly available from list_recursive, use the root_id as fallback).
    let root_name = folder.root_id().to_string();

    let root_dir = if output.ends_with(std::path::MAIN_SEPARATOR) {
        Path::new(&output).join(&root_name)
    } else {
        PathBuf::from(&output)
    };

    if opts.skip_download {
        let mut files = Vec::new();
        for entry in &entries {
            if entry.is_folder {
                continue;
            }
            let local_path = root_dir.join(&entry.path);
            files.push(GoogleDriveFileToDownload {
                id: entry.id.clone(),
                path: entry.path.clone(),
                local_path: local_path.to_string_lossy().to_string(),
            });
        }
        return Ok(Some(DownloadFolderResult::DryRun(files)));
    }

    tokio::fs::create_dir_all(&root_dir).await?;

    let mut files: Vec<String> = Vec::new();
    for entry in &entries {
        let local_path = root_dir.join(&entry.path);

        if entry.is_folder {
            tokio::fs::create_dir_all(&local_path).await?;
            continue;
        }

        if opts.resume && local_path.is_file() {
            if !opts.quiet {
                tracing::debug!(path = %local_path.display(), "skipping already downloaded file");
            }
            files.push(local_path.to_string_lossy().to_string());
            continue;
        }

        let resource_key = Url::parse(&url).ok().and_then(|u| {
            u.query_pairs()
                .find(|(k, _)| k.as_ref() == "resourcekey")
                .map(|(_, v)| v.into_owned())
        });

        let mut file_url = format!("https://drive.google.com/uc?id={}", entry.id);
        if let Some(ref rk) = resource_key {
            file_url = format!("{file_url}&resourcekey={rk}");
        }

        let file_opts = DownloadOptions {
            url: Some(file_url),
            output: Some(local_path.to_string_lossy().to_string()),
            quiet: opts.quiet,
            proxy: opts.proxy.clone(),
            speed: opts.speed,
            use_cookies: opts.use_cookies,
            verify: opts.verify,
            resume: opts.resume,
            ..DownloadOptions::default()
        };

        if let Some(path) = download(&file_opts).await? {
            files.push(path);
        } else {
            if !opts.quiet {
                tracing::error!(
                    file_id = %entry.id,
                    local_path = %local_path.display(),
                    "download ended unsuccessfully"
                );
            }
            return Ok(None);
        }
    }

    if !opts.quiet {
        tracing::info!(files_downloaded = files.len(), "download completed");
    }

    Ok(Some(DownloadFolderResult::Downloaded(files)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_page() -> Result<()> {
        let html_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/folder-page-sample.html"
        );
        let content = std::fs::read_to_string(html_path)?;
        let folder_url = "https://drive.google.com/drive/folders/1KpLl_1tcK0eeehzN980zbG-3M2nhbVks";

        let (gdrive_file, id_name_type_iter) = parse_google_drive_file(folder_url, &content)?;

        assert_eq!(gdrive_file.id, "1KpLl_1tcK0eeehzN980zbG-3M2nhbVks");
        assert_eq!(gdrive_file.name, "gdown_folder_test");
        assert_eq!(gdrive_file.file_type, "application/vnd.google-apps.folder");
        assert!(gdrive_file.children.is_empty());

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

        let actual_ids: Vec<&str> = id_name_type_iter
            .iter()
            .map(|(id, _, _)| id.as_str())
            .collect();
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

        Ok(())
    }

    #[test]
    fn test_decode_unicode_escapes() {
        assert_eq!(decode_unicode_escapes(r"\x5b\x5b"), "[[");
        assert_eq!(decode_unicode_escapes(r"\x22hello\x22"), "\"hello\"");
    }
}
