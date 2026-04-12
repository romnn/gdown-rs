use color_eyre::eyre;
use std::path::Path;

use gdown::download_folder::DownloadFolderOptions;
use gdown::error::Error;
use gdown::parse::{
    confirm_token_from_headers, confirm_token_from_html, parse_folder_page,
    url_from_gdrive_confirmation,
};
use gdown::{DownloadOptions, download, is_google_drive_url, parse_url};

#[test]
fn test_parse_url_open_link() {
    let fid = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!("https://drive.google.com/open?id={fid}");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
    assert!(!result.is_download_link);
}

#[test]
fn test_parse_url_uc_link() {
    let fid = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!("https://drive.google.com/uc?id={fid}");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
    assert!(result.is_download_link);
}

#[test]
fn test_parse_url_file_view_link() {
    let fid = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!("https://drive.google.com/file/d/{fid}/view?usp=sharing");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
    assert!(!result.is_download_link);
}

#[test]
fn test_parse_url_subdomain_uc_link() {
    let fid = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url =
        format!("https://drive.google.com/a/jsk.imi.i.u-tokyo.ac.jp/uc?id={fid}&export=download");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
    assert!(result.is_download_link);
}

#[test]
fn test_parse_url_non_gdrive() {
    let result = parse_url("https://example.com/somefile.zip");
    assert!(result.file_id.is_none());
    assert!(!result.is_download_link);
}

#[test]
fn test_parse_url_document_edit() {
    let fid = "abcdef123456";
    let url = format!("https://docs.google.com/document/d/{fid}/edit");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
    assert!(!result.is_download_link);
}

#[test]
fn test_parse_url_spreadsheets_view() {
    let fid = "spreadsheet_id_xyz";
    let url = format!("https://docs.google.com/spreadsheets/d/{fid}/view");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
}

#[test]
fn test_parse_url_presentation_view() {
    let fid = "slides_id_abc";
    let url = format!("https://docs.google.com/presentation/d/{fid}/view");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
}

#[test]
fn test_parse_url_file_with_user_number() {
    let fid = "abcdef";
    let url = format!("https://drive.google.com/file/u/0/d/{fid}/view");
    let result = parse_url(&url);
    assert_eq!(result.file_id.as_deref(), Some(fid));
}

#[test]
fn test_is_google_drive_url_positive() {
    assert!(is_google_drive_url("https://drive.google.com/uc?id=abc"));
    assert!(is_google_drive_url(
        "https://docs.google.com/document/d/abc/view"
    ));
}

#[test]
fn test_is_google_drive_url_negative() {
    assert!(!is_google_drive_url("https://example.com/file"));
    assert!(!is_google_drive_url("https://google.com/drive"));
}

#[tokio::test]
async fn test_download_either_url_or_id_required() {
    let opts = DownloadOptions::default();
    let result = download(&opts).await;
    assert!(result.is_err());
    if let Err(Error::InvalidInput(msg)) = result {
        assert!(msg.contains("exactly one"));
    }
}

#[tokio::test]
async fn test_download_both_url_and_id_is_error() {
    let opts = DownloadOptions {
        url: Some("https://example.com".to_string()),
        id: Some("abc".to_string()),
        ..DownloadOptions::default()
    };
    let result = download(&opts).await;
    assert!(result.is_err());
}

#[test]
fn test_gdrive_confirmation_uc_export_link() -> eyre::Result<()> {
    let html = r#"<a href="/uc?export=download&amp;id=ABCDEF&amp;confirm=t">Download</a>"#;
    let url = url_from_gdrive_confirmation(html)?;
    assert!(url.starts_with("https://docs.google.com/uc?export=download"));
    assert!(url.contains("id=ABCDEF"));
    assert!(!url.contains("&amp;"));
    Ok(())
}

#[test]
fn test_gdrive_confirmation_download_url_json() -> eyre::Result<()> {
    let html =
        "something \"downloadUrl\":\"https://example.com/download\\u003did\\u0026token\" else";
    let url = url_from_gdrive_confirmation(html)?;
    assert_eq!(url, "https://example.com/download=id&token");
    Ok(())
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
fn test_gdrive_confirmation_download_form() -> eyre::Result<()> {
    let html = concat!(
        r#"<form id="download-form" action="https://drive.usercontent.google.com/download?id=FILEID&amp;export=download">"#,
        r#"<input type="hidden" name="confirm" value="t">"#,
        r#"<input type="hidden" name="uuid" value="abc-123">"#,
        r#"</form>"#
    );
    let url = url_from_gdrive_confirmation(html)?;
    assert!(url.contains("id=FILEID"));
    assert!(url.contains("confirm=t"));
    assert!(url.contains("uuid=abc-123"));
    Ok(())
}

#[tokio::test]
async fn test_download_folder_either_url_or_id_required() {
    let opts = DownloadFolderOptions::default();
    let result = gdown::download_folder(&opts).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_download_folder_both_url_and_id_is_error() {
    let opts = DownloadFolderOptions {
        url: Some("https://drive.google.com/drive/folders/abc".to_string()),
        id: Some("abc".to_string()),
        ..DownloadFolderOptions::default()
    };
    let result = gdown::download_folder(&opts).await;
    assert!(result.is_err());
}

#[test]
fn test_parse_folder_page() -> eyre::Result<()> {
    let html_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/folder-page-sample.html"
    );
    let content = std::fs::read_to_string(html_path)?;
    let folder_url = "https://drive.google.com/drive/folders/1KpLl_1tcK0eeehzN980zbG-3M2nhbVks";

    let parsed = parse_folder_page(folder_url, &content)?;

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

    Ok(())
}

#[test]
fn test_extractall_unsupported_format() {
    let result = gdown::extractall(Path::new("file.rar"), None);
    assert!(result.is_err());
}

#[test]
fn test_extractall_zip() -> eyre::Result<()> {
    use std::io::Write;

    let dir = tempfile::tempdir()?;
    let zip_path = dir.path().join("test.zip");

    let file = std::fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("hello.txt", options)?;
    zip_writer.write_all(b"Hello, world!")?;
    zip_writer.finish()?;

    let extract_dir = dir.path().join("extracted");
    let files = gdown::extractall(&zip_path, Some(&extract_dir))?;

    assert_eq!(files.len(), 1);
    assert!(extract_dir.join("hello.txt").exists());
    assert_eq!(
        std::fs::read_to_string(extract_dir.join("hello.txt"))?,
        "Hello, world!"
    );

    Ok(())
}

#[test]
fn test_extractall_tar_gz() -> eyre::Result<()> {
    let dir = tempfile::tempdir()?;
    let tar_gz_path = dir.path().join("test.tar.gz");

    let file = std::fs::File::create(&tar_gz_path)?;
    let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar_builder = tar::Builder::new(enc);

    let content = b"Hello from tar!";
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar_builder.append_data(&mut header, "greeting.txt", content.as_slice())?;
    let enc = tar_builder.into_inner()?;
    enc.finish()?;

    let extract_dir = dir.path().join("extracted");
    let files = gdown::extractall(&tar_gz_path, Some(&extract_dir))?;

    assert!(!files.is_empty());
    assert!(extract_dir.join("greeting.txt").exists());
    assert_eq!(
        std::fs::read_to_string(extract_dir.join("greeting.txt"))?,
        "Hello from tar!"
    );

    Ok(())
}

#[test]
fn test_extractall_default_dest() -> eyre::Result<()> {
    use std::io::Write;

    let dir = tempfile::tempdir()?;
    let zip_path = dir.path().join("test.zip");

    let file = std::fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("file.txt", options)?;
    zip_writer.write_all(b"content")?;
    zip_writer.finish()?;

    let files = gdown::extractall(&zip_path, None)?;
    assert_eq!(files.len(), 1);
    assert!(dir.path().join("file.txt").exists());

    Ok(())
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
    assert!(confirm_token_from_html("no tokens here").is_none());
}

#[test]
fn test_confirm_token_from_headers_empty() {
    let headers = reqwest::header::HeaderMap::new();
    assert!(confirm_token_from_headers(&headers).is_none());
}

#[test]
fn test_confirm_token_from_headers_with_warning() -> eyre::Result<()> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append(
        reqwest::header::SET_COOKIE,
        "download_warning_abc=mytoken; path=/".parse()?,
    );
    let token = confirm_token_from_headers(&headers);
    assert_eq!(token, Some("mytoken".to_string()));
    Ok(())
}

#[test]
fn test_confirm_token_from_headers_ignores_other_cookies() -> eyre::Result<()> {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append(
        reqwest::header::SET_COOKIE,
        "other_cookie=value; path=/".parse()?,
    );
    assert!(confirm_token_from_headers(&headers).is_none());
    Ok(())
}

#[test]
fn test_cli_version_flag() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("gdown");
    cmd.arg("--version").assert().success();
}

#[test]
fn test_cli_help_flag() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("gdown");
    cmd.arg("--help").assert().success();
}

#[test]
fn test_cli_no_args_fails() {
    let mut cmd = assert_cmd::cargo::cargo_bin_cmd!("gdown");
    cmd.assert().failure();
}
