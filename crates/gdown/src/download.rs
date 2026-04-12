use std::path::{Path, PathBuf};

use reqwest::header::{HeaderMap, HeaderValue, RANGE};
use reqwest::{Client, Response};
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::client::{
    build_client, content_length, default_file_user_agent, read_response_limited,
    sanitize_filename, stream_response,
};
use crate::error::{Error, Result};
use crate::parse::{
    confirm_token_from_headers, confirm_token_from_html, extract_resource_key,
    filename_from_response, parse_url, title_from_html, url_from_gdrive_confirmation,
};
use crate::types::{CommonOptions, resolve_url_or_id};

/// Options for downloading a single file.
#[derive(Debug, Clone, Default)]
pub struct DownloadOptions {
    pub url: Option<String>,
    pub id: Option<String>,
    pub common: CommonOptions,
    pub fuzzy: bool,
    pub format: Option<String>,
}

/// Download a file from a URL (supports Google Drive).
///
/// Returns the output filename on success, or `None` when writing to stdout.
///
/// # Errors
///
/// Returns an error if the input options are invalid, HTTP requests fail,
/// Google Drive confirmation pages cannot be parsed, or filesystem
/// operations fail.
#[tracing::instrument(
    name = "download",
    level = "info",
    skip(opts),
    fields(
        url = ?opts.url,
        id = ?opts.id,
        output = ?opts.common.output,
        fuzzy = opts.fuzzy,
        resume = opts.common.resume,
    )
)]
pub async fn download(opts: &DownloadOptions) -> Result<Option<String>> {
    let mut url = resolve_url_or_id(&opts.url, &opts.id, |id| {
        format!("https://drive.google.com/uc?id={id}")
    })?;

    let user_agent = opts
        .common
        .user_agent
        .as_deref()
        .unwrap_or(default_file_user_agent());

    let url_origin = url.clone();
    let resource_key = extract_resource_key(&url_origin);

    let client = build_client(
        opts.common.proxy.as_deref(),
        opts.common.use_cookies,
        user_agent,
        opts.common.verify,
    )?;

    let parsed = parse_url(&url);
    let gdrive_file_id = parsed.file_id;

    if opts.fuzzy
        && let Some(ref fid) = gdrive_file_id
    {
        url = format!("https://drive.google.com/uc?id={fid}");
        if let Some(ref rk) = resource_key {
            url = format!("{url}&resourcekey={rk}");
        }
    }

    let is_gdrive_download_link = if opts.fuzzy && gdrive_file_id.is_some() {
        true
    } else {
        parsed.is_download_link
    };

    if !is_gdrive_download_link
        && !opts.fuzzy
        && let Some(ref fid) = gdrive_file_id
    {
        tracing::warn!(
            file_id = %fid,
            suggested_url = %format!("https://drive.google.com/uc?id={fid}"),
            "not a direct download link; consider using --fuzzy"
        );
    }

    let (res, current_url) = resolve_gdrive_response(
        &client,
        url,
        &url_origin,
        gdrive_file_id.as_deref(),
        is_gdrive_download_link,
        resource_key.as_deref(),
        opts.format.as_deref(),
    )
    .await?;

    let filename_from_url = if gdrive_file_id.is_some() && is_gdrive_download_link {
        filename_from_response(&res)
    } else {
        None
    }
    .or_else(|| {
        let url_path = Url::parse(&current_url)
            .map(|u| u.path().to_string())
            .unwrap_or_default();
        let basename = Path::new(&url_path).file_name().map_or_else(
            || "download".to_string(),
            |f| f.to_string_lossy().to_string(),
        );
        Some(sanitize_filename(&basename))
    })
    .unwrap_or_else(|| "download".to_string());

    let mut output = opts
        .common
        .output
        .clone()
        .unwrap_or_else(|| filename_from_url.clone());

    if output == "-" {
        return download_to_stdout(res, opts, &url_origin, &current_url).await;
    }

    if output.ends_with(std::path::MAIN_SEPARATOR) {
        let output_dir = Path::new(&output);
        if !output_dir.exists() {
            tokio::fs::create_dir_all(output_dir).await?;
        }
        output = output_dir
            .join(&filename_from_url)
            .to_string_lossy()
            .to_string();
    }

    download_to_file(&client, res, &current_url, &url_origin, &output, opts).await
}

#[allow(clippy::too_many_arguments)]
async fn resolve_gdrive_response(
    client: &Client,
    initial_url: String,
    url_origin: &str,
    gdrive_file_id: Option<&str>,
    is_gdrive_download_link: bool,
    resource_key: Option<&str>,
    format: Option<&str>,
) -> Result<(Response, String)> {
    let url_after_fuzzy = initial_url.clone();
    let mut current_url = initial_url;
    let mut res;

    loop {
        res = client.get(&current_url).send().await?;

        if !(gdrive_file_id.is_some() && is_gdrive_download_link) {
            break;
        }

        if current_url == url_after_fuzzy
            && res.status().as_u16() == 500
            && let Some(fid) = gdrive_file_id
        {
            current_url = format!("https://drive.google.com/open?id={fid}");
            if let Some(rk) = resource_key {
                current_url = format!("{current_url}&resourcekey={rk}");
            }
            continue;
        }

        let content_type = res
            .headers()
            .get("Content-Type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.starts_with("text/html") {
            let headers = res.headers().clone();
            let text = read_response_limited(res).await?;

            if let Some(title) = title_from_html(&text)
                && let Some(fid) = gdrive_file_id
            {
                let redirect = if title.ends_with(" - Google Docs") {
                    let fmt = format.unwrap_or("docx");
                    Some(format!(
                        "https://docs.google.com/document/d/{fid}/export?format={fmt}"
                    ))
                } else if title.ends_with(" - Google Sheets") {
                    let fmt = format.unwrap_or("xlsx");
                    Some(format!(
                        "https://docs.google.com/spreadsheets/d/{fid}/export?format={fmt}"
                    ))
                } else if title.ends_with(" - Google Slides") {
                    let fmt = format.unwrap_or("pptx");
                    Some(format!(
                        "https://docs.google.com/presentation/d/{fid}/export?format={fmt}"
                    ))
                } else {
                    None
                };
                if let Some(new_url) = redirect {
                    current_url = new_url;
                    continue;
                }
            }

            match url_from_gdrive_confirmation(&text) {
                Ok(new_url) => {
                    current_url = append_resource_key(new_url, resource_key);
                    continue;
                }
                Err(e) => {
                    if let Some(fid) = gdrive_file_id {
                        let token = confirm_token_from_headers(&headers)
                            .or_else(|| confirm_token_from_html(&text));
                        if let Some(token) = token {
                            let new_url = format!(
                                "https://drive.google.com/uc?export=download&id={fid}&confirm={token}"
                            );
                            current_url = append_resource_key(new_url, resource_key);
                            continue;
                        }
                    }
                    return Err(Error::FileUrlRetrieval(format!(
                        "Failed to retrieve file url:\n\n\t{e}\n\n\
                         You may still be able to access the file from the browser:\
                         \n\n\t{url_origin}\n"
                    )));
                }
            }
        }

        if let Some(cd) = res.headers().get("Content-Disposition")
            && let Ok(cd_str) = cd.to_str()
            && cd_str.ends_with("pptx")
            && let Some(fmt) = format
            && fmt != "pptx"
            && let Some(fid) = gdrive_file_id
        {
            current_url =
                format!("https://docs.google.com/presentation/d/{fid}/export?format={fmt}");
            continue;
        }

        break;
    }

    Ok((res, current_url))
}

async fn download_to_file(
    client: &Client,
    mut res: Response,
    current_url: &str,
    url_origin: &str,
    output: &str,
    opts: &DownloadOptions,
) -> Result<Option<String>> {
    let mut resume = opts.common.resume;
    let output_path = Path::new(output);

    if resume && output_path.is_file() {
        tracing::debug!(output = %output, "skipping already downloaded file");
        return Ok(Some(output.to_string()));
    }

    let dir = output_path
        .parent()
        .map_or_else(|| PathBuf::from("."), std::path::Path::to_path_buf);
    let basename = output_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut existing_parts: Vec<PathBuf> = Vec::new();
    if dir.is_dir()
        && let Ok(entries) = std::fs::read_dir(&dir)
    {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if fname.starts_with(&basename)
                && Path::new(&fname)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("part"))
            {
                existing_parts.push(entry.path());
            }
        }
    }

    let tmp_file = if resume && !existing_parts.is_empty() {
        if existing_parts.len() != 1 {
            return Err(Error::InvalidInput(format!(
                "multiple .part files found for {basename}; remove all but one to resume"
            )));
        }
        existing_parts.into_iter().next().unwrap_or_else(|| {
            resume = false;
            dir.join(format!("{basename}.part"))
        })
    } else {
        resume = false;
        dir.join(format!("{basename}.part"))
    };

    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&tmp_file)
        .await?;

    let mut start_size = f.metadata().await?.len();
    if start_size > 0 && resume {
        let mut headers = HeaderMap::new();
        let range_value = format!("bytes={start_size}-");
        let header_value: HeaderValue = range_value
            .parse()
            .map_err(|e| Error::Parse(format!("invalid header value: {e}")))?;
        headers.insert(RANGE, header_value);
        res = client.get(current_url).headers(headers).send().await?;

        if res.status().as_u16() == 200 {
            f.flush().await?;
            drop(f);
            let _ = tokio::fs::remove_file(&tmp_file).await;
            f = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&tmp_file)
                .await?;
            start_size = 0;
        }
    }

    if !opts.common.quiet {
        log_download_info(url_origin, current_url, output, resume, &tmp_file);
    }

    let total = content_length(&res).map(|v| v + start_size);
    let progress = opts.common.progress.as_deref();

    stream_response(
        &mut res,
        &mut f,
        progress,
        total,
        opts.common.speed,
        start_size,
    )
    .await?;

    drop(f);
    tokio::fs::rename(&tmp_file, output).await?;

    Ok(Some(output.to_string()))
}

async fn download_to_stdout(
    mut res: Response,
    opts: &DownloadOptions,
    url_origin: &str,
    current_url: &str,
) -> Result<Option<String>> {
    if opts.common.resume {
        return Err(Error::InvalidInput(
            "cannot use --continue when output is stdout".to_string(),
        ));
    }

    if !opts.common.quiet {
        tracing::info!("downloading to stdout");
        if url_origin == current_url {
            tracing::info!(from = %current_url, "source");
        } else {
            tracing::info!(from_original = %url_origin, from_redirected = %current_url, "source");
        }
    }

    let total = content_length(&res);
    let progress = opts.common.progress.as_deref();
    let mut out = tokio::io::stdout();

    stream_response(&mut res, &mut out, progress, total, opts.common.speed, 0).await?;

    out.flush().await?;
    Ok(None)
}

fn append_resource_key(mut url: String, resource_key: Option<&str>) -> String {
    if let Some(rk) = resource_key
        && !url.contains("resourcekey=")
    {
        if url.contains('?') {
            url = format!("{url}&resourcekey={rk}");
        } else {
            url = format!("{url}?resourcekey={rk}");
        }
    }
    url
}

fn log_download_info(
    url_origin: &str,
    current_url: &str,
    output: &str,
    resume: bool,
    tmp_file: &Path,
) {
    tracing::info!("downloading");
    if resume {
        tracing::info!(tmp_file = %tmp_file.display(), "resume enabled");
    }
    if url_origin == current_url {
        tracing::info!(from = %current_url, "source");
    } else {
        tracing::info!(from_original = %url_origin, from_redirected = %current_url, "source");
    }
    let abs_output = std::fs::canonicalize(output).unwrap_or_else(|_| PathBuf::from(output));
    tracing::info!(to = %abs_output.display(), "destination");
}
