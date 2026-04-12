use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Extract an archive file.
///
/// Supported formats: `.zip`, `.tar`, `.tar.gz`, `.tgz`, `.tar.bz2`, `.tbz`.
///
/// Returns the list of extracted file paths.
///
/// # Errors
///
/// Returns an error if the format is unsupported, the archive cannot be read,
/// or extraction to disk fails.
pub fn extractall(path: &Path, to: Option<&Path>) -> Result<Vec<PathBuf>> {
    let dest = match to {
        Some(d) => d.to_path_buf(),
        None => path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };

    if !dest.exists() {
        std::fs::create_dir_all(&dest)?;
    }

    if has_extension(path, "zip") {
        extract_zip(path, &dest)
    } else if has_extension(path, "tar") {
        extract_tar(path, &dest, Compression::None)
    } else if has_extension(path, "tgz") || has_double_extension(path, "tar", "gz") {
        extract_tar(path, &dest, Compression::Gzip)
    } else if has_extension(path, "tbz") || has_double_extension(path, "tar", "bz2") {
        extract_tar(path, &dest, Compression::Bzip2)
    } else {
        Err(Error::Extract(format!(
            "could not extract '{}': no appropriate extractor found",
            path.display()
        )))
    }
}

#[derive(Copy, Clone)]
enum Compression {
    None,
    Gzip,
    Bzip2,
}

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension()
        .is_some_and(|actual| actual.eq_ignore_ascii_case(ext))
}

fn has_double_extension(path: &Path, first: &str, second: &str) -> bool {
    if !has_extension(path, second) {
        return false;
    }
    path.file_stem()
        .and_then(|stem| Path::new(stem).extension())
        .is_some_and(|actual| actual.eq_ignore_ascii_case(first))
}

fn extract_zip(path: &Path, dest: &Path) -> Result<Vec<PathBuf>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| Error::Extract(format!("failed to open zip: {e}")))?;

    let mut files = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| Error::Extract(format!("failed to read zip entry: {e}")))?;
        let outpath = dest.join(entry.mangled_name());

        if entry.is_dir() {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut entry, &mut outfile)?;
        }
        files.push(outpath);
    }

    Ok(files)
}

fn extract_tar(path: &Path, dest: &Path, compression: Compression) -> Result<Vec<PathBuf>> {
    let file = std::fs::File::open(path)?;
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
            let decoder = bzip2::read::BzDecoder::new(file);
            let mut archive = tar::Archive::new(decoder);
            collect_tar_entries(&mut archive, dest, &mut files)?;
        }
    }

    Ok(files)
}

fn collect_tar_entries<R: std::io::Read>(
    archive: &mut tar::Archive<R>,
    dest: &Path,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?.to_path_buf();
        let outpath = dest.join(&entry_path);
        files.push(outpath);
        entry.unpack_in(dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use color_eyre::eyre;
    use std::io::Write;

    #[test]
    fn test_unsupported_format() {
        let result = extractall(Path::new("file.rar"), None);
        assert!(result.is_err());
        if let Err(Error::Extract(msg)) = result {
            assert!(msg.contains("no appropriate extractor"));
        }
    }

    #[test]
    fn test_zip() -> eyre::Result<()> {
        let dir = tempfile::tempdir()?;
        let zip_path = dir.path().join("test.zip");

        let file = std::fs::File::create(&zip_path)?;
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("hello.txt", options)?;
        zip_writer.write_all(b"Hello, world!")?;
        zip_writer.finish()?;

        let extract_dir = dir.path().join("extracted");
        let files = extractall(&zip_path, Some(&extract_dir))?;

        assert_eq!(files.len(), 1);
        assert!(extract_dir.join("hello.txt").exists());
        assert_eq!(
            std::fs::read_to_string(extract_dir.join("hello.txt"))?,
            "Hello, world!"
        );

        Ok(())
    }

    #[test]
    fn test_tar_gz() -> eyre::Result<()> {
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
        let files = extractall(&tar_gz_path, Some(&extract_dir))?;

        assert!(!files.is_empty());
        assert!(extract_dir.join("greeting.txt").exists());
        assert_eq!(
            std::fs::read_to_string(extract_dir.join("greeting.txt"))?,
            "Hello from tar!"
        );

        Ok(())
    }

    #[test]
    fn test_default_dest() -> eyre::Result<()> {
        let dir = tempfile::tempdir()?;
        let zip_path = dir.path().join("test.zip");

        let file = std::fs::File::create(&zip_path)?;
        let mut zip_writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        zip_writer.start_file("file.txt", options)?;
        zip_writer.write_all(b"content")?;
        zip_writer.finish()?;

        let files = extractall(&zip_path, None)?;
        assert_eq!(files.len(), 1);
        assert!(dir.path().join("file.txt").exists());

        Ok(())
    }
}
