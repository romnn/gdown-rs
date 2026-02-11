use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use indicatif::{ProgressBar, ProgressStyle};
use regex::Regex;
use reqwest::blocking::{Client, ClientBuilder, Response};
use reqwest::header::HeaderMap;
use scraper::{Html, Selector};
use url::Url;

use crate::error::{Error, Result};
use crate::parse_url::parse_url;

pub const CHUNK_SIZE: usize = 512 * 1024; // 512KB

pub const DEFAULT_FILE_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_10_1) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/39.0.2171.95 Safari/537.36";

pub const DEFAULT_FOLDER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
    AppleWebKit/537.36 (KHTML, like Gecko) Chrome/98.0.4758.102 Safari/537.36";

/// Extract the download URL from a Google Drive confirmation page.
pub fn get_url_from_gdrive_confirmation(contents: &str) -> Result<String> {
    for line in contents.lines() {
        // Check for /uc?export=download link
        let re = Regex::new(r#"href="(/uc\?export=download[^"]+)"#).unwrap();
        if let Some(caps) = re.captures(line) {
            let url = format!("https://docs.google.com{}", &caps[1]);
            let url = url.replace("&amp;", "&");
            return Ok(url);
        }

        // Check for download-form
        let document = Html::parse_fragment(line);
        let form_selector = Selector::parse("#download-form").unwrap();
        if let Some(form) = document.select(&form_selector).next() {
            let action = form
                .value()
                .attr("action")
                .unwrap_or("")
                .replace("&amp;", "&");
            let mut parsed = Url::parse(&action).map_err(|e| {
                Error::FileURLRetrieval(format!("Failed to parse form action URL: {}", e))
            })?;

            // Collect hidden input fields
            let input_selector =
                Selector::parse(r#"input[type="hidden"]"#).unwrap();
            let mut query_pairs: Vec<(String, String)> = parsed
                .query_pairs()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            for input in document.select(&input_selector) {
                if let (Some(name), Some(value)) =
                    (input.value().attr("name"), input.value().attr("value"))
                {
                    // Remove existing entries with same name, then add new one
                    query_pairs.retain(|(k, _)| k != name);
                    query_pairs.push((name.to_string(), value.to_string()));
                }
            }

            parsed.query_pairs_mut().clear().extend_pairs(&query_pairs);
            return Ok(parsed.to_string());
        }

        // Check for downloadUrl in JSON
        let re = Regex::new(r#""downloadUrl":"([^"]+)"#).unwrap();
        if let Some(caps) = re.captures(line) {
            let url = caps[1]
                .replace("\\u003d", "=")
                .replace("\\u0026", "&");
            return Ok(url);
        }

        // Check for error message
        let re = Regex::new(r#"<p class="uc-error-subcaption">(.*)</p>"#).unwrap();
        if let Some(caps) = re.captures(line) {
            return Err(Error::FileURLRetrieval(caps[1].to_string()));
        }
    }

    Err(Error::FileURLRetrieval(
        "Cannot retrieve the public link of the file. \
         You may need to change the permission to \
         'Anyone with the link', or have had many accesses. \
         Check FAQ in https://github.com/wkentaro/gdown?tab=readme-ov-file#faq."
            .to_string(),
    ))
}

/// Extract filename from Content-Disposition header.
pub fn get_filename_from_response(response: &Response) -> Option<String> {
    let content_disposition = response.headers().get("Content-Disposition")?;
    let cd_str = urlencoding::decode(content_disposition.to_str().ok()?).ok()?;

    // Try filename*=UTF-8'' format
    let re = Regex::new(r"filename\*=UTF-8''(.*)").unwrap();
    if let Some(caps) = re.captures(&cd_str) {
        let filename = caps[1].replace(std::path::MAIN_SEPARATOR, "_");
        return Some(filename);
    }

    // Try filename="..." format
    let re = Regex::new(r#"attachment; filename="(.*?)""#).unwrap();
    if let Some(caps) = re.captures(&cd_str) {
        return Some(caps[1].to_string());
    }

    None
}

/// Build an HTTP client (session equivalent).
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
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|e| Error::Http(e))?;
        builder = builder.proxy(proxy);
        eprintln!("Using proxy: {}", proxy_url);
    }

    let client = builder.build()?;
    Ok(client)
}

/// Options for downloading a file.
#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub url: Option<String>,
    pub output: Option<String>,
    pub quiet: bool,
    pub proxy: Option<String>,
    pub speed: Option<f64>,
    pub use_cookies: bool,
    pub verify: bool,
    pub id: Option<String>,
    pub fuzzy: bool,
    pub resume: bool,
    pub format: Option<String>,
    pub user_agent: Option<String>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            url: None,
            output: None,
            quiet: false,
            proxy: None,
            speed: None,
            use_cookies: true,
            verify: true,
            id: None,
            fuzzy: false,
            resume: false,
            format: None,
            user_agent: None,
        }
    }
}

/// Download file from URL.
///
/// Google Drive URL is also supported.
/// Returns the output filename on success.
pub fn download(opts: &DownloadOptions) -> Result<Option<String>> {
    let has_url = opts.url.is_some();
    let has_id = opts.id.is_some();
    if !(has_id ^ has_url) {
        return Err(Error::InvalidInput(
            "Either url or id has to be specified".to_string(),
        ));
    }

    let mut url = if let Some(ref id) = opts.id {
        format!("https://drive.google.com/uc?id={}", id)
    } else {
        opts.url.clone().unwrap()
    };

    let user_agent = opts
        .user_agent
        .as_deref()
        .unwrap_or(DEFAULT_FILE_USER_AGENT);

    let url_origin = url.clone();

    let client = build_client(
        opts.proxy.as_deref(),
        opts.use_cookies,
        user_agent,
        opts.verify,
    )?;

    let (gdrive_file_id, is_gdrive_download_link) = parse_url(&url, !opts.fuzzy);

    if opts.fuzzy {
        if let Some(ref fid) = gdrive_file_id {
            url = format!("https://drive.google.com/uc?id={}", fid);
        }
    }

    let is_gdrive_download_link = if opts.fuzzy && gdrive_file_id.is_some() {
        true
    } else {
        is_gdrive_download_link
    };

    let url_after_fuzzy = url.clone();

    // Re-assign for the loop
    let mut current_url = url;
    let mut res;

    loop {
        res = client.get(&current_url).send()?;

        if !(gdrive_file_id.is_some() && is_gdrive_download_link) {
            break;
        }

        if current_url == url_after_fuzzy && res.status().as_u16() == 500 {
            if let Some(ref fid) = gdrive_file_id {
                current_url = format!("https://drive.google.com/open?id={}", fid);
                continue;
            }
        }

        let content_type = res
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.starts_with("text/html") {
            let text = res.text()?;
            let re = Regex::new(r"<title>(.+)</title>").unwrap();
            if let Some(caps) = re.captures(&text) {
                let title = &caps[1];
                if let Some(ref fid) = gdrive_file_id {
                    if title.ends_with(" - Google Docs") {
                        let fmt = opts.format.as_deref().unwrap_or("docx");
                        current_url = format!(
                            "https://docs.google.com/document/d/{}/export?format={}",
                            fid, fmt
                        );
                        continue;
                    } else if title.ends_with(" - Google Sheets") {
                        let fmt = opts.format.as_deref().unwrap_or("xlsx");
                        current_url = format!(
                            "https://docs.google.com/spreadsheets/d/{}/export?format={}",
                            fid, fmt
                        );
                        continue;
                    } else if title.ends_with(" - Google Slides") {
                        let fmt = opts.format.as_deref().unwrap_or("pptx");
                        current_url = format!(
                            "https://docs.google.com/presentation/d/{}/export?format={}",
                            fid, fmt
                        );
                        continue;
                    }
                }
            }

            // Check for Content-Disposition (shouldn't be present if we got text/html and read text)
            // Need to redirect with confirmation
            match get_url_from_gdrive_confirmation(&text) {
                Ok(new_url) => {
                    current_url = new_url;
                    continue;
                }
                Err(e) => {
                    let message = format!(
                        "Failed to retrieve file url:\n\n\t{}\n\n\
                         You may still be able to access the file from the browser:\
                         \n\n\t{}\n\n\
                         but Gdown can't. Please check connections and permissions.",
                        e, url_origin
                    );
                    return Err(Error::FileURLRetrieval(message));
                }
            }
        } else {
            // Check Content-Disposition for pptx redirect
            if let Some(cd) = res.headers().get("Content-Disposition") {
                if let Ok(cd_str) = cd.to_str() {
                    if cd_str.ends_with("pptx") {
                        if let Some(ref fmt) = opts.format {
                            if fmt != "pptx" {
                                if let Some(ref fid) = gdrive_file_id {
                                    current_url = format!(
                                        "https://docs.google.com/presentation/d/{}/export?format={}",
                                        fid, fmt
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                }
            }

            // Check if Content-Disposition is present -> file is ready
            if res.headers().contains_key("Content-Disposition") {
                break;
            }

            // For non-html, non-gdrive content, just break
            break;
        }
    }

    // Determine filename
    let mut filename_from_url: Option<String> = None;
    if gdrive_file_id.is_some() && is_gdrive_download_link {
        filename_from_url = get_filename_from_response(&res);
    }
    if filename_from_url.is_none() {
        // Use basename of URL
        let url_path = Url::parse(&current_url)
            .map(|u| u.path().to_string())
            .unwrap_or_default();
        let basename = Path::new(&url_path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| "download".to_string());
        filename_from_url = Some(basename);
    }

    let filename_from_url = filename_from_url.unwrap();

    let mut output = opts.output.clone().unwrap_or_else(|| filename_from_url.clone());
    let output_is_path = true; // In Rust port we always use paths (no stdout buffer support in lib)

    if output.ends_with(std::path::MAIN_SEPARATOR) {
        let output_dir = Path::new(&output);
        if !output_dir.exists() {
            fs::create_dir_all(output_dir)?;
        }
        output = output_dir.join(&filename_from_url).to_string_lossy().to_string();
    }

    let mut resume = opts.resume;

    if output_is_path {
        let output_path = Path::new(&output);
        if resume && output_path.is_file() {
            if !opts.quiet {
                eprintln!("Skipping already downloaded file {}", output);
            }
            return Ok(Some(output));
        }

        // Look for existing .part files
        let dir = output_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let basename = output_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut existing_tmp_files: Vec<PathBuf> = Vec::new();
        if dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let fname = entry.file_name().to_string_lossy().to_string();
                    if fname.starts_with(&basename) && fname.ends_with(".part") {
                        existing_tmp_files.push(entry.path());
                    }
                }
            }
        }

        let tmp_file: PathBuf;

        if resume && !existing_tmp_files.is_empty() {
            if existing_tmp_files.len() != 1 {
                eprintln!("There are multiple temporary files to resume:");
                eprintln!();
                for file in &existing_tmp_files {
                    eprintln!("\t{}", file.display());
                }
                eprintln!();
                eprintln!("Please remove them except one to resume downloading.");
                return Ok(None);
            }
            tmp_file = existing_tmp_files.into_iter().next().unwrap();
        } else {
            resume = false;
            tmp_file = dir.join(format!("{}.part", basename));
        }

        // Open tmp file for appending
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&tmp_file)?;

        let start_size = f.metadata()?.len();
        if start_size > 0 && resume {
            // Re-request with Range header
            let mut headers = HeaderMap::new();
            headers.insert(
                "Range",
                format!("bytes={}-", start_size).parse().unwrap(),
            );
            res = client
                .get(&current_url)
                .headers(headers)
                .send()?;
        }

        if !opts.quiet {
            eprintln!("Downloading...");
            if resume {
                eprintln!("Resume: {}", tmp_file.display());
            }
            if url_origin != current_url {
                eprintln!("From (original): {}", url_origin);
                eprintln!("From (redirected): {}", current_url);
            } else {
                eprintln!("From: {}", current_url);
            }
            let abs_output = std::fs::canonicalize(&output)
                .unwrap_or_else(|_| PathBuf::from(&output));
            eprintln!("To: {}", abs_output.display());
        }

        let total = res
            .headers()
            .get("Content-Length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|v| v + start_size);

        let pbar = if !opts.quiet {
            let pb = if let Some(total) = total {
                ProgressBar::new(total)
            } else {
                ProgressBar::new_spinner()
            };
            pb.set_style(
                ProgressStyle::default_bar()
                    .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec})")
                    .unwrap_or_else(|_| ProgressStyle::default_bar())
                    .progress_chars("#>-"),
            );
            pb.set_position(start_size);
            Some(pb)
        } else {
            None
        };

        let t_start = Instant::now();
        let mut downloaded: u64 = 0;
        let mut buf = vec![0u8; CHUNK_SIZE];

        loop {
            let bytes_read = std::io::Read::read(&mut res, &mut buf)?;
            if bytes_read == 0 {
                break;
            }
            f.write_all(&buf[..bytes_read])?;
            downloaded += bytes_read as u64;
            if let Some(ref pb) = pbar {
                pb.set_position(start_size + downloaded);
            }
            if let Some(speed_limit) = opts.speed {
                let elapsed = t_start.elapsed().as_secs_f64();
                let expected = downloaded as f64 / speed_limit;
                if elapsed < expected {
                    std::thread::sleep(Duration::from_secs_f64(expected - elapsed));
                }
            }
        }

        if let Some(pb) = pbar {
            pb.finish();
        }

        // Move tmp file to final output
        drop(f);
        fs::rename(&tmp_file, &output)?;
    }

    Ok(Some(output))
}

