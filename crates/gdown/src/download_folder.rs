//! Lists, resolves, and downloads shared Google Drive folders.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use url::Url;

use crate::client::{build_client, default_folder_user_agent};
use crate::download::{DownloadOptions, download};
use crate::error::{Error, Result};
use crate::parse::{FolderChild, extract_resource_key, is_google_drive_url, parse_folder_page};
use crate::types::{Options, resolve_url_or_id};

const FOLDER_MIME: &str = "application/vnd.google-apps.folder";
const MAX_FILES: usize = 50;

/// An entry in a Google Drive folder listing.
#[derive(Debug, Clone)]
pub struct DirEntry {
    /// Google Drive identifier used for follow-up listings or downloads.
    pub id: String,
    /// Display name reported by Google Drive.
    pub name: String,
    /// MIME type reported by Google Drive.
    pub mime_type: String,
}

impl DirEntry {
    /// Returns whether this entry represents a Google Drive folder.
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
    /// Google Drive identifier used for downloads or child listings.
    pub id: String,
    /// Slash-separated path relative to the opened folder.
    pub path: String,
    /// Indicates whether the entry is a folder rather than a downloadable file.
    pub is_folder: bool,
}

/// Lazily lists and downloads content from a shared Google Drive folder.
///
/// [`DriveFolder::open`] creates the handle without making an HTTP request.
/// Directory listings are fetched one level at a time and cached by folder ID for reuse.
pub struct DriveFolder {
    root_id: String,
    resource_key: Option<String>,
    client: reqwest::Client,
    cache: tokio::sync::Mutex<HashMap<String, Vec<DirEntry>>>,
    /// Options applied to every file downloaded through this handle.
    options: Options,
}

impl DriveFolder {
    /// Opens a Google Drive folder URL with the default folder HTTP client.
    ///
    /// This constructor extracts the final path segment as the root folder ID and retains the
    /// optional `resourcekey` query parameter.
    /// No HTTP requests are made until a listing or download method is called.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the folder ID cannot be extracted.
    pub fn open(url: &str) -> Result<Self> {
        let client = build_client(None, true, default_folder_user_agent(), true)?;
        Self::open_with_client(url, client)
    }

    /// Opens a Google Drive folder URL with a caller-provided HTTP client.
    ///
    /// The client is retained for all uncached listing requests.
    /// No HTTP requests are made by this constructor.
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
            options: Options::default(),
        })
    }

    /// Replaces the options applied to subsequent file downloads.
    #[must_use]
    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self
    }

    /// Lists the direct children of a folder ID.
    ///
    /// Successful results are cached and later calls return a cloned snapshot.
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

    /// Lists the direct children of the opened root folder.
    ///
    /// # Errors
    ///
    /// Returns an error if the HTTP request fails or the response cannot be parsed.
    pub async fn list_root(&self) -> Result<Vec<DirEntry>> {
        self.list(&self.root_id).await
    }

    /// Resolves a `/`-separated path relative to the opened root.
    ///
    /// Empty path segments are ignored and names are matched exactly.
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

    /// Recursively lists every file and folder below the opened root.
    ///
    /// The root itself is omitted, each path is relative to it, and traversal order is
    /// unspecified.
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

    /// Resolves a remote file path and downloads it to `output`.
    ///
    /// # Errors
    ///
    /// Returns an error if the path cannot be resolved or the download fails.
    pub async fn download(&self, remote_path: &str, output: &Path) -> Result<()> {
        let entry = self.resolve(remote_path).await?;
        self.download_by_id(&entry.id, output).await
    }

    /// Downloads a file by its Google Drive file ID.
    ///
    /// Missing parent directories for `output` are created before the request starts.
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
            options: Options {
                output: Some(output.to_string_lossy().to_string()),
                ..self.options.clone()
            },
            ..DownloadOptions::default()
        })
        .await?;

        Ok(())
    }

    /// Downloads a remote directory tree into `dest_dir`, mirroring its contents.
    ///
    /// Subfolders are descended into so the destination has the same structure as Google Drive.
    /// This preserves directory-backed formats such as Apple `.mlpackage` bundles, whose
    /// useful files may be several levels below the selected prefix.
    ///
    /// The returned paths contain downloaded files but not directories.
    /// Empty remote folders are still created locally.
    ///
    /// # Errors
    ///
    /// Returns an error if the prefix is not a folder or any download fails.
    pub async fn download_prefix(&self, prefix: &str, dest_dir: &Path) -> Result<Vec<PathBuf>> {
        let dir_entry = self.resolve(prefix).await?;
        if !dir_entry.is_folder() {
            return Err(Error::InvalidInput(format!("{prefix} is not a directory")));
        }

        // An explicit stack rather than recursion: an `async fn` that awaits itself needs
        // boxing on every level, and the traversal order does not matter here.
        let mut paths = Vec::new();
        let mut pending = vec![(dir_entry.id, dest_dir.to_path_buf())];

        while let Some((folder_id, local_dir)) = pending.pop() {
            tokio::fs::create_dir_all(&local_dir).await?;
            for child in self.list(&folder_id).await? {
                let local_path = local_dir.join(&child.name);
                if child.is_folder() {
                    pending.push((child.id, local_path));
                } else {
                    self.download_by_id(&child.id, &local_path).await?;
                    paths.push(local_path);
                }
            }
        }

        Ok(paths)
    }

    /// Returns the Google Drive identifier of the opened root folder.
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

/// Configures a recursive folder download.
#[derive(Debug, Clone, Default)]
pub struct DownloadFolderOptions {
    /// Supplies the folder URL when `id` is [`None`].
    pub url: Option<String>,
    /// Supplies a Google Drive folder ID when `url` is [`None`].
    pub id: Option<String>,
    /// Controls output, transport, resumption, and progress reporting.
    pub options: Options,
    /// Allows folders containing 50 or more files when `true`.
    pub ignore_file_limit: bool,
}

/// Downloads an entire Google Drive folder tree.
///
/// Exactly one of [`DownloadFolderOptions::url`] and [`DownloadFolderOptions::id`] must
/// be present.
/// The returned strings are local file paths that were downloaded or skipped as already complete.
/// When the configured output ends with a path separator, the folder ID is appended as the local
/// root directory name.
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
        output = ?opts.options.output,
        quiet = opts.options.quiet,
        ignore_file_limit = opts.ignore_file_limit,
        resume = opts.options.resume
    )
)]
pub async fn download_folder(opts: &DownloadFolderOptions) -> Result<Vec<String>> {
    let url = resolve_url_or_id(&opts.url, &opts.id, |id| {
        format!("https://drive.google.com/drive/folders/{id}")
    })?;

    let user_agent = opts
        .options
        .user_agent
        .as_deref()
        .unwrap_or(default_folder_user_agent());

    let client = build_client(
        opts.options.proxy.as_deref(),
        opts.options.use_cookies,
        user_agent,
        opts.options.verify,
    )?;

    let folder = DriveFolder::open_with_client(&url, client)?.with_options(opts.options.clone());

    if !opts.options.quiet {
        tracing::info!("retrieving folder contents");
    }

    let entries = folder.list_recursive().await?;

    if !opts.options.quiet {
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
        .options
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

        if opts.options.resume && local_path.is_file() {
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
            options: Options {
                output: Some(local_path.to_string_lossy().to_string()),
                quiet: opts.options.quiet,
                proxy: opts.options.proxy.clone(),
                speed: opts.options.speed,
                use_cookies: opts.options.use_cookies,
                verify: opts.options.verify,
                resume: opts.options.resume,
                user_agent: opts.options.user_agent.clone(),
                progress: opts.options.progress.clone(),
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

    if !opts.options.quiet {
        tracing::info!(files_downloaded = files.len(), "download completed");
    }

    Ok(files)
}
