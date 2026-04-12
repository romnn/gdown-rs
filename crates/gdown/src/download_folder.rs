use std::collections::HashMap;
use std::path::{Path, PathBuf};

use url::Url;

use crate::client::{build_client, default_folder_user_agent};
use crate::download::{DownloadOptions, download};
use crate::error::{Error, Result};
use crate::parse::{FolderChild, extract_resource_key, is_google_drive_url, parse_folder_page};
use crate::types::{CommonOptions, resolve_url_or_id};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const MAX_FILES: usize = 50;

/// An entry in a Google Drive folder listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub id: String,
    pub name: String,
    pub mime_type: String,
}

impl DirEntry {
    #[must_use]
    pub fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_MIME
    }
}

impl From<FolderChild> for DirEntry {
    fn from(c: FolderChild) -> Self {
        Self {
            id: c.id,
            name: c.name,
            mime_type: c.mime_type,
        }
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
    /// client. No HTTP requests are made.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the folder ID cannot be extracted.
    pub fn open(url: &str) -> Result<Self> {
        let client = build_client(None, true, default_folder_user_agent(), true)?;
        Self::open_with_client(url, client)
    }

    /// Open with a pre-built HTTP client.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the folder ID cannot be extracted.
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

    /// Resolve a `/`-separated path to its [`DirEntry`].
    ///
    /// # Errors
    ///
    /// Returns an error if any path component is not found or is not a folder.
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
                    Error::Parse(format!(
                        "not found in Google Drive folder: {component} (in path {path})"
                    ))
                })?
                .clone();

            let is_last = i == components.len().saturating_sub(1);
            if is_last {
                return Ok(child);
            }

            if !child.is_folder() {
                return Err(Error::Parse(format!(
                    "{component} is not a folder (in path {path})"
                )));
            }
            current_folder_id = child.id;
        }

        Err(Error::Parse(format!("unexpected end of path: {path}")))
    }

    /// Recursively list all files and folders.
    ///
    /// # Errors
    ///
    /// Returns an error if any folder page cannot be fetched or parsed.
    pub async fn list_recursive(&self) -> Result<Vec<FlatEntry>> {
        let mut result = Vec::new();
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
    /// # Errors
    ///
    /// Returns an error if the path cannot be resolved or the download fails.
    pub async fn download(&self, remote_path: &str, output: &Path) -> Result<()> {
        let entry = self.resolve(remote_path).await?;
        self.download_by_id(&entry.id, output).await
    }

    /// Download a file by its Google Drive file ID.
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
            common: CommonOptions {
                output: Some(output.to_string_lossy().to_string()),
                ..CommonOptions::default()
            },
            ..DownloadOptions::default()
        })
        .await?;

        Ok(())
    }

    /// Download all files directly under a remote directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the prefix is not a folder or any download fails.
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

    #[must_use]
    pub fn root_id(&self) -> &str {
        &self.root_id
    }

    async fn fetch_children(&self, folder_id: &str) -> Result<Vec<DirEntry>> {
        let mut url = format!("https://drive.google.com/drive/folders/{folder_id}");
        if let Some(ref rk) = self.resource_key {
            url = format!("{url}?resourcekey={rk}");
        }

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
                return Err(Error::Parse(format!(
                    "folder listing returned HTTP {}",
                    res.status()
                )));
            }

            let response_url = res.url().to_string();
            let text = res.text().await?;

            if is_google_drive_url(&fetch_url) || !is_google_drive_url(&response_url) {
                let parsed = parse_folder_page(&fetch_url, &text)?;
                return Ok(parsed.children.into_iter().map(DirEntry::from).collect());
            }

            url = response_url;
        }

        Err(Error::Parse(format!(
            "failed to list folder {folder_id} after retries"
        )))
    }
}

/// Options for downloading an entire folder.
#[derive(Debug, Clone, Default)]
pub struct DownloadFolderOptions {
    pub url: Option<String>,
    pub id: Option<String>,
    pub common: CommonOptions,
    pub ignore_file_limit: bool,
}

/// Downloads an entire folder from Google Drive.
///
/// Returns the list of local file paths that were downloaded.
///
/// # Errors
///
/// Returns an error if the input is invalid, HTTP requests fail, the file
/// limit is exceeded, or filesystem operations fail.
#[tracing::instrument(
    name = "download_folder",
    level = "info",
    skip(opts),
    fields(
        url = ?opts.url,
        id = ?opts.id,
        output = ?opts.common.output,
        quiet = opts.common.quiet,
        ignore_file_limit = opts.ignore_file_limit,
        resume = opts.common.resume
    )
)]
pub async fn download_folder(opts: &DownloadFolderOptions) -> Result<Vec<String>> {
    let url = resolve_url_or_id(&opts.url, &opts.id, |id| {
        format!("https://drive.google.com/drive/folders/{id}")
    })?;

    let user_agent = opts
        .common
        .user_agent
        .as_deref()
        .unwrap_or(default_folder_user_agent());

    let client = build_client(
        opts.common.proxy.as_deref(),
        opts.common.use_cookies,
        user_agent,
        opts.common.verify,
    )?;

    let folder = DriveFolder::open_with_client(&url, client)?;

    if !opts.common.quiet {
        tracing::info!("retrieving folder contents");
    }

    let entries = folder.list_recursive().await?;

    if !opts.common.quiet {
        tracing::info!(file_count = entries.len(), "folder contents retrieved");
    }

    if !opts.ignore_file_limit {
        let file_count = entries.iter().filter(|e| !e.is_folder).count();
        if file_count >= MAX_FILES {
            return Err(Error::FolderLimit(format!(
                "folder has {file_count} files (limit is {MAX_FILES}); \
                 use --ignore-file-limit to override"
            )));
        }
    }

    let output = opts
        .common
        .output
        .clone()
        .unwrap_or_else(|| format!(".{}", std::path::MAIN_SEPARATOR));

    let root_name = folder.root_id().to_string();
    let root_dir = if output.ends_with(std::path::MAIN_SEPARATOR) {
        Path::new(&output).join(&root_name)
    } else {
        PathBuf::from(&output)
    };

    tokio::fs::create_dir_all(&root_dir).await?;

    let resource_key = extract_resource_key(&url);

    let mut files: Vec<String> = Vec::new();
    for entry in &entries {
        let local_path = root_dir.join(&entry.path);

        if entry.is_folder {
            tokio::fs::create_dir_all(&local_path).await?;
            continue;
        }

        if opts.common.resume && local_path.is_file() {
            tracing::debug!(path = %local_path.display(), "skipping already downloaded file");
            files.push(local_path.to_string_lossy().to_string());
            continue;
        }

        let mut file_url = format!("https://drive.google.com/uc?id={}", entry.id);
        if let Some(ref rk) = resource_key {
            file_url = format!("{file_url}&resourcekey={rk}");
        }

        let file_opts = DownloadOptions {
            url: Some(file_url),
            common: CommonOptions {
                output: Some(local_path.to_string_lossy().to_string()),
                quiet: opts.common.quiet,
                proxy: opts.common.proxy.clone(),
                speed: opts.common.speed,
                use_cookies: opts.common.use_cookies,
                verify: opts.common.verify,
                resume: opts.common.resume,
                user_agent: opts.common.user_agent.clone(),
            },
            ..DownloadOptions::default()
        };

        match download(&file_opts).await? {
            Some(path) => files.push(path),
            None => {
                return Err(Error::FileUrlRetrieval(format!(
                    "download failed for file {} ({})",
                    entry.path, entry.id
                )));
            }
        }
    }

    if !opts.common.quiet {
        tracing::info!(files_downloaded = files.len(), "download completed");
    }

    Ok(files)
}
