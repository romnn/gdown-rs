use std::fs;

use gdown::download::get_url_from_gdrive_confirmation;
use gdown::download_folder::{
    get_directory_structure, parse_google_drive_file, DownloadFolderOptions,
    GoogleDriveFile, GoogleDriveFileToDownload, MAX_NUMBER_FILES,
};
use gdown::error::Error;
use gdown::parse_url::{is_google_drive_url, parse_url};
use gdown::{download, DownloadOptions};

// ---------------------------------------------------------------------------
// test_parse_url (ports vendor/gdown/tests/test_parse_url.py)
// ---------------------------------------------------------------------------

#[test]
fn test_parse_url_open_link() {
    let file_id = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!("https://drive.google.com/open?id={}", file_id);
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(!is_dl);
}

#[test]
fn test_parse_url_uc_link() {
    let file_id = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!("https://drive.google.com/uc?id={}", file_id);
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(is_dl);
}

#[test]
fn test_parse_url_file_view_link() {
    let file_id = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!(
        "https://drive.google.com/file/d/{}/view?usp=sharing",
        file_id
    );
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(!is_dl);
}

#[test]
fn test_parse_url_subdomain_uc_link() {
    let file_id = "0B_NiLAzvehC9R2stRmQyM3ZiVjQ";
    let url = format!(
        "https://drive.google.com/a/jsk.imi.i.u-tokyo.ac.jp/uc?id={}&export=download",
        file_id
    );
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(is_dl);
}

#[test]
fn test_parse_url_non_gdrive() {
    let url = "https://example.com/somefile.zip";
    let (id, is_dl) = parse_url(url, false);
    assert!(id.is_none());
    assert!(!is_dl);
}

#[test]
fn test_parse_url_document_edit() {
    let file_id = "abcdef123456";
    let url = format!("https://docs.google.com/document/d/{}/edit", file_id);
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(!is_dl);
}

#[test]
fn test_parse_url_spreadsheets_view() {
    let file_id = "spreadsheet_id_xyz";
    let url = format!(
        "https://docs.google.com/spreadsheets/d/{}/view",
        file_id
    );
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(!is_dl);
}

#[test]
fn test_parse_url_presentation_view() {
    let file_id = "slides_id_abc";
    let url = format!(
        "https://docs.google.com/presentation/d/{}/view",
        file_id
    );
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(!is_dl);
}

#[test]
fn test_parse_url_file_with_user_number() {
    let file_id = "abcdef";
    let url = format!(
        "https://drive.google.com/file/u/0/d/{}/view",
        file_id
    );
    let (id, is_dl) = parse_url(&url, false);
    assert_eq!(id.as_deref(), Some(file_id));
    assert!(!is_dl);
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

// ---------------------------------------------------------------------------
// test_download (ports vendor/gdown/tests/test_download.py)
// ---------------------------------------------------------------------------

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn test_download_either_url_or_id_required() -> TestResult {
    let opts = DownloadOptions {
        url: None,
        id: None,
        ..DownloadOptions::default()
    };
    let err = match download(&opts) {
        Ok(_) => return Err("expected error".into()),
        Err(e) => e,
    };
    match err {
        Error::InvalidInput(msg) => {
            assert!(msg.contains("Either url or id"));
        }
        other => {
            return Err(format!("Expected InvalidInput, got {other:?}").into());
        }
    }

    Ok(())
}

#[test]
fn test_download_both_url_and_id_is_error() -> TestResult {
    let opts = DownloadOptions {
        url: Some("https://example.com".to_string()),
        id: Some("abc".to_string()),
        ..DownloadOptions::default()
    };
    let result = download(&opts);
    assert!(result.is_err());

    Ok(())
}

// ---------------------------------------------------------------------------
// test_get_url_from_gdrive_confirmation
// ---------------------------------------------------------------------------

#[test]
fn test_gdrive_confirmation_uc_export_link() -> TestResult {
    let html = r#"<a href="/uc?export=download&amp;id=ABCDEF&amp;confirm=t">Download</a>"#;
    let url = get_url_from_gdrive_confirmation(html)?;
    assert!(url.starts_with("https://docs.google.com/uc?export=download"));
    assert!(url.contains("id=ABCDEF"));
    assert!(url.contains("confirm=t"));
    // &amp; should be decoded
    assert!(!url.contains("&amp;"));

    Ok(())
}

#[test]
fn test_gdrive_confirmation_download_url_json() -> TestResult {
    let html = "something \"downloadUrl\":\"https://example.com/download\\u003did\\u0026token\" else";
    let url = get_url_from_gdrive_confirmation(html)?;
    assert_eq!(url, "https://example.com/download=id&token");

    Ok(())
}

#[test]
fn test_gdrive_confirmation_error_subcaption() -> TestResult {
    let html = r#"<p class="uc-error-subcaption">Sorry, quota exceeded</p>"#;
    let result = get_url_from_gdrive_confirmation(html);
    assert!(result.is_err());
    let err = match result {
        Ok(_) => return Err("expected error".into()),
        Err(e) => e,
    };
    match err {
        Error::FileURLRetrieval(msg) => {
            assert!(msg.contains("Sorry, quota exceeded"));
        }
        other => {
            return Err(format!("Expected FileURLRetrieval, got {other:?}").into());
        }
    }

    Ok(())
}

#[test]
fn test_gdrive_confirmation_empty_page() -> TestResult {
    let html = "<html><body>Nothing useful here</body></html>";
    let result = get_url_from_gdrive_confirmation(html);
    assert!(result.is_err());
    let err = match result {
        Ok(_) => return Err("expected error".into()),
        Err(e) => e,
    };
    match err {
        Error::FileURLRetrieval(msg) => {
            assert!(msg.contains("Cannot retrieve the public link"));
        }
        other => {
            return Err(format!("Expected FileURLRetrieval, got {other:?}").into());
        }
    }

    Ok(())
}

#[test]
fn test_gdrive_confirmation_download_form() -> TestResult {
    let html = concat!(
        r#"<form id="download-form" action="https://drive.usercontent.google.com/download?id=FILEID&amp;export=download">"#,
        r#"<input type="hidden" name="confirm" value="t">"#,
        r#"<input type="hidden" name="uuid" value="abc-123">"#,
        r#"</form>"#
    );
    let url = get_url_from_gdrive_confirmation(html)?;
    assert!(url.contains("id=FILEID"));
    assert!(url.contains("confirm=t"));
    assert!(url.contains("uuid=abc-123"));

    Ok(())
}

// ---------------------------------------------------------------------------
// test_download_folder (ports vendor/gdown/tests/test_download_folder.py)
// ---------------------------------------------------------------------------

#[test]
fn test_valid_page_parse() -> TestResult {
    let html_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/folder-page-sample.html"
    );
    let content = fs::read_to_string(html_path)?;
    let folder_url =
        "https://drive.google.com/drive/folders/1KpLl_1tcK0eeehzN980zbG-3M2nhbVks";

    let (gdrive_file, id_name_type_iter) = parse_google_drive_file(folder_url, &content)?;

    assert_eq!(gdrive_file.id, "1KpLl_1tcK0eeehzN980zbG-3M2nhbVks");
    assert_eq!(gdrive_file.name, "gdown_folder_test");
    assert_eq!(gdrive_file.file_type, "application/vnd.google-apps.folder");
    assert!(gdrive_file.children.is_empty());
    assert!(gdrive_file.is_folder());

    let expected_ids = vec![
        "1aMZqPaU03E7XOQNXtjSCdguRHBaIQ82m",
        "1hVAxfM7_doToqQ24eVd65cgiaoLi0TtO",
        "1Z2VYnXb01h-3uvEptoQ48Fo__eAn0wc1",
        "14xzOzvKjP0at07jfonV7qVrTKoctFijz",
        "1wlapSEt6N9Ayf7fzCTOkra_4GIg-cqeD",
    ];
    let expected_names = vec![
        "directory-0",
        "directory-1",
        "fractal.jpg",
        "this is a file.txt",
        "tux.jpg",
    ];
    let expected_types = vec![
        "application/vnd.google-apps.folder",
        "application/vnd.google-apps.folder",
        "image/jpeg",
        "text/plain",
        "image/jpeg",
    ];

    let actual_ids: Vec<&str> = id_name_type_iter.iter().map(|(id, _, _)| id.as_str()).collect();
    let actual_names: Vec<&str> = id_name_type_iter
        .iter()
        .map(|(_, n, _)| n.as_str())
        .collect();
    let actual_types: Vec<&str> = id_name_type_iter
        .iter()
        .map(|(_, _, t)| t.as_str())
        .collect();

    assert_eq!(actual_ids, expected_ids);
    assert_eq!(actual_names, expected_names);
    assert_eq!(actual_types, expected_types);

    Ok(())
}

#[test]
fn test_download_folder_either_url_or_id_required() {
    let opts = DownloadFolderOptions {
        url: None,
        id: None,
        ..DownloadFolderOptions::default()
    };
    let result = gdown::download_folder(&opts);
    assert!(result.is_err());
}

#[test]
fn test_download_folder_both_url_and_id_is_error() {
    let opts = DownloadFolderOptions {
        url: Some("https://drive.google.com/drive/folders/abc".to_string()),
        id: Some("abc".to_string()),
        ..DownloadFolderOptions::default()
    };
    let result = gdown::download_folder(&opts);
    assert!(result.is_err());
}

// ---------------------------------------------------------------------------
// test_get_directory_structure
// ---------------------------------------------------------------------------

#[test]
fn test_get_directory_structure_flat() -> TestResult {
    let root = GoogleDriveFile {
        id: "root".to_string(),
        name: "root_folder".to_string(),
        file_type: "application/vnd.google-apps.folder".to_string(),
        children: vec![
            GoogleDriveFile {
                id: "f1".to_string(),
                name: "file1.txt".to_string(),
                file_type: "text/plain".to_string(),
                children: vec![],
            },
            GoogleDriveFile {
                id: "f2".to_string(),
                name: "file2.jpg".to_string(),
                file_type: "image/jpeg".to_string(),
                children: vec![],
            },
        ],
    };

    let structure = get_directory_structure(&root, "");
    assert_eq!(structure.len(), 2);
    let first = structure.first().ok_or("missing first entry")?;
    let second = structure.get(1).ok_or("missing second entry")?;
    assert_eq!(first, &(Some("f1".to_string()), "file1.txt".to_string()));
    assert_eq!(second, &(Some("f2".to_string()), "file2.jpg".to_string()));

    Ok(())
}

#[test]
fn test_get_directory_structure_nested() -> TestResult {
    let root = GoogleDriveFile {
        id: "root".to_string(),
        name: "root_folder".to_string(),
        file_type: "application/vnd.google-apps.folder".to_string(),
        children: vec![
            GoogleDriveFile {
                id: "subfolder".to_string(),
                name: "sub".to_string(),
                file_type: "application/vnd.google-apps.folder".to_string(),
                children: vec![GoogleDriveFile {
                    id: "f1".to_string(),
                    name: "nested.txt".to_string(),
                    file_type: "text/plain".to_string(),
                    children: vec![],
                }],
            },
            GoogleDriveFile {
                id: "f2".to_string(),
                name: "top.txt".to_string(),
                file_type: "text/plain".to_string(),
                children: vec![],
            },
        ],
    };

    let structure = get_directory_structure(&root, "");
    assert_eq!(structure.len(), 3);
    let first = structure.first().ok_or("missing first entry")?;
    let second = structure.get(1).ok_or("missing second entry")?;
    let third = structure.get(2).ok_or("missing third entry")?;
    assert_eq!(first, &(None, "sub".to_string()));
    assert_eq!(
        second,
        &(
            Some("f1".to_string()),
            format!("sub{}nested.txt", std::path::MAIN_SEPARATOR)
        )
    );
    assert_eq!(third, &(Some("f2".to_string()), "top.txt".to_string()));

    Ok(())
}

// ---------------------------------------------------------------------------
// test_extractall (ports vendor/gdown/gdown/extractall.py)
// ---------------------------------------------------------------------------

#[test]
fn test_extractall_unsupported_format() {
    let result = gdown::extractall("file.rar", None);
    assert!(result.is_err());
}

#[test]
fn test_extractall_zip() -> TestResult {
    use std::io::Write;

    let dir = tempfile::tempdir()?;
    let zip_path = dir.path().join("test.zip");

    let file = fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("hello.txt", options)?;
    zip_writer.write_all(b"Hello, world!")?;
    zip_writer.finish()?;

    let extract_dir = dir.path().join("extracted");
    let zip_path_str = zip_path.to_str().ok_or("zip path is not utf-8")?;
    let extract_dir_str = extract_dir.to_str().ok_or("extract dir is not utf-8")?;
    let files = gdown::extractall(zip_path_str, Some(extract_dir_str))?;

    assert_eq!(files.len(), 1);
    assert!(extract_dir.join("hello.txt").exists());
    assert_eq!(
        fs::read_to_string(extract_dir.join("hello.txt"))?,
        "Hello, world!"
    );

    Ok(())
}

#[test]
fn test_extractall_tar_gz() -> TestResult {
    let dir = tempfile::tempdir()?;
    let tar_gz_path = dir.path().join("test.tar.gz");

    let file = fs::File::create(&tar_gz_path)?;
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
    let tar_gz_path_str = tar_gz_path.to_str().ok_or("tar.gz path is not utf-8")?;
    let extract_dir_str = extract_dir.to_str().ok_or("extract dir is not utf-8")?;
    let files = gdown::extractall(tar_gz_path_str, Some(extract_dir_str))?;

    assert!(!files.is_empty());
    assert!(extract_dir.join("greeting.txt").exists());
    assert_eq!(
        fs::read_to_string(extract_dir.join("greeting.txt"))?,
        "Hello from tar!"
    );

    Ok(())
}

#[test]
fn test_extractall_default_dest() -> TestResult {
    use std::io::Write;

    let dir = tempfile::tempdir()?;
    let zip_path = dir.path().join("test.zip");

    let file = fs::File::create(&zip_path)?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip_writer.start_file("file.txt", options)?;
    zip_writer.write_all(b"content")?;
    zip_writer.finish()?;

    // Extract without specifying `to` — should go to parent of zip
    let zip_path_str = zip_path.to_str().ok_or("zip path is not utf-8")?;
    let files = gdown::extractall(zip_path_str, None)?;
    assert_eq!(files.len(), 1);
    assert!(dir.path().join("file.txt").exists());

    Ok(())
}

// ---------------------------------------------------------------------------
// test_max_number_files constant
// ---------------------------------------------------------------------------

#[test]
fn test_max_number_files() {
    assert_eq!(MAX_NUMBER_FILES, 50);
}

// ---------------------------------------------------------------------------
// CLI argument parsing tests (ports test___main__.py parse_file_size etc.)
// ---------------------------------------------------------------------------

#[test]
fn test_cli_version_flag() -> TestResult {
    #[allow(deprecated)]
    let mut cmd = assert_cmd::Command::cargo_bin("gdown")?;
    cmd.arg("--version").assert().success();

    Ok(())
}

#[test]
fn test_cli_help_flag() -> TestResult {
    #[allow(deprecated)]
    let mut cmd = assert_cmd::Command::cargo_bin("gdown")?;
    cmd.arg("--help").assert().success();

    Ok(())
}

#[test]
fn test_cli_no_args_fails() -> TestResult {
    #[allow(deprecated)]
    let mut cmd = assert_cmd::Command::cargo_bin("gdown")?;
    cmd.assert().failure();

    Ok(())
}

// ---------------------------------------------------------------------------
// GoogleDriveFile struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_google_drive_file_is_folder() {
    let folder = GoogleDriveFile::new(
        "id".to_string(),
        "name".to_string(),
        "application/vnd.google-apps.folder".to_string(),
    );
    assert!(folder.is_folder());

    let file = GoogleDriveFile::new(
        "id".to_string(),
        "name".to_string(),
        "text/plain".to_string(),
    );
    assert!(!file.is_folder());
}

// ---------------------------------------------------------------------------
// GoogleDriveFileToDownload struct tests
// ---------------------------------------------------------------------------

#[test]
fn test_google_drive_file_to_download_fields() {
    let f = GoogleDriveFileToDownload {
        id: "abc".to_string(),
        path: "folder/file.txt".to_string(),
        local_path: "/tmp/folder/file.txt".to_string(),
    };
    assert_eq!(f.id, "abc");
    assert_eq!(f.path, "folder/file.txt");
    assert_eq!(f.local_path, "/tmp/folder/file.txt");
}
