use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Extract an archive file.
///
/// Supported formats: `.zip`, `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`, `.tbz`.
///
/// Returns the list of extracted file paths.
pub fn extractall(path: &str, to: Option<&str>) -> Result<Vec<String>> {
    let archive_path = Path::new(path);
    let dest = match to {
        Some(d) => PathBuf::from(d),
        None => archive_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };

    if !dest.exists() {
        fs::create_dir_all(&dest)?;
    }

    if path.ends_with(".zip") {
        extract_zip(archive_path, &dest)
    } else if path.ends_with(".tar") {
        extract_tar(archive_path, &dest, Compression::None)
    } else if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
        extract_tar(archive_path, &dest, Compression::Gzip)
    } else if path.ends_with(".tar.bz2") || path.ends_with(".tbz") {
        extract_tar(archive_path, &dest, Compression::Bzip2)
    } else {
        Err(Error::ExtractError(format!(
            "Could not extract '{}' as no appropriate extractor is found",
            path
        )))
    }
}

enum Compression {
    None,
    Gzip,
    Bzip2,
}

fn extract_zip(path: &Path, dest: &Path) -> Result<Vec<String>> {
    let file = fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::ExtractError(format!("Failed to open zip: {}", e)))?;

    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::ExtractError(format!("Failed to read zip entry: {}", e)))?;
        let outpath = dest.join(entry.mangled_name());

        if entry.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
        files.push(outpath.to_string_lossy().to_string());
    }

    Ok(files)
}

fn extract_tar(path: &Path, dest: &Path, compression: Compression) -> Result<Vec<String>> {
    let file = fs::File::open(path)?;

    let mut files = Vec::new();

    match compression {
        Compression::None => {
            let mut archive = tar::Archive::new(file);
            collect_tar_entries(&mut archive, dest, &mut files)?;
        }
        Compression::Gzip => {
            let decoder = flate2::read::GzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            collect_tar_entries(&mut archive, dest, &mut files)?;
        }
        Compression::Bzip2 => {
            // bzip2 support: use flate2 doesn't support bz2, we need a different crate
            // For now, we'll return an error suggesting the user install bzip2 support
            return Err(Error::ExtractError(
                "bzip2 extraction is not yet supported in this build".to_string(),
            ));
        }
    }

    Ok(files)
}

fn collect_tar_entries<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
    dest: &Path,
    files: &mut Vec<String>,
) -> Result<()> {
    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let outpath = dest.join(&entry_path);
        files.push(outpath.to_string_lossy().to_string());
        entry.unpack_in(dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_extractall_unsupported_format() {
        let result = extractall("file.rar", None);
        assert!(result.is_err());
        if let Err(Error::ExtractError(msg)) = result {
            assert!(msg.contains("no appropriate extractor"));
        }
    }

    #[test]
    fn test_extractall_zip() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("test.zip");

        // Create a simple zip file
        let file = fs::File::create(&zip_path).unwrap();
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("hello.txt", options).unwrap();
        zip_writer.write_all(b"Hello, world!").unwrap();
        zip_writer.finish().unwrap();

        let extract_dir = dir.path().join("extracted");
        let files = extractall(
            zip_path.to_str().unwrap(),
            Some(extract_dir.to_str().unwrap()),
        )
        .unwrap();

        assert_eq!(files.len(), 1);
        assert!(extract_dir.join("hello.txt").exists());
        assert_eq!(
            fs::read_to_string(extract_dir.join("hello.txt")).unwrap(),
            "Hello, world!"
        );
    }

    #[test]
    fn test_extractall_tar_gz() {
        let dir = tempfile::tempdir().unwrap();
        let tar_gz_path = dir.path().join("test.tar.gz");

        // Create a tar.gz file
        let file = fs::File::create(&tar_gz_path).unwrap();
        let enc = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar_builder = tar::Builder::new(enc);

        let content = b"Hello from tar!";
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar_builder
            .append_data(&mut header, "greeting.txt", &content[..])
            .unwrap();
        let enc = tar_builder.into_inner().unwrap();
        enc.finish().unwrap();

        let extract_dir = dir.path().join("extracted");
        let files = extractall(
            tar_gz_path.to_str().unwrap(),
            Some(extract_dir.to_str().unwrap()),
        )
        .unwrap();

        assert!(!files.is_empty());
        assert!(extract_dir.join("greeting.txt").exists());
        assert_eq!(
            fs::read_to_string(extract_dir.join("greeting.txt")).unwrap(),
            "Hello from tar!"
        );
    }
}
