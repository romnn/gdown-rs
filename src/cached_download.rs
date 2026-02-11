use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::download::{download, DownloadOptions};
use crate::error::{Error, Result};

/// Get the default cache root directory (~/.cache/gdown).
pub fn cache_root() -> PathBuf {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    home.join(".cache").join("gdown")
}

/// Compute the hash of a file using the specified algorithm.
///
/// Returns a string in the format `{algorithm}:{hex_digest}`.
pub fn compute_filehash(path: &Path, algorithm: &str) -> Result<String> {
    const BLOCKSIZE: usize = 65536;

    let mut file = fs::File::open(path)?;
    let mut buf = vec![0u8; BLOCKSIZE];

    let hex = match algorithm {
        "md5" => {
            use md5::Digest;
            let mut hasher = md5::Md5::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            format!("{:x}", hasher.finalize())
        }
        "sha1" => {
            use sha1::Digest;
            let mut hasher = sha1::Sha1::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            format!("{:x}", hasher.finalize())
        }
        "sha256" => {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            format!("{:x}", hasher.finalize())
        }
        "sha512" => {
            use sha2::Digest;
            let mut hasher = sha2::Sha512::new();
            loop {
                let n = file.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            format!("{:x}", hasher.finalize())
        }
        _ => {
            return Err(Error::InvalidInput(format!(
                "Unsupported hash algorithm: {}. Supported algorithms: md5, sha1, sha256, sha512",
                algorithm
            )));
        }
    };

    Ok(format!("{}:{}", algorithm, hex))
}

/// Assert that a file's hash matches the expected hash.
///
/// `hash` must be in format `{algorithm}:{hex_value}`.
pub fn assert_filehash(path: &Path, hash: &str, quiet: bool) -> Result<bool> {
    if !hash.contains(':') {
        return Err(Error::InvalidInput(format!(
            "Invalid hash: {}. Hash must be in the format of {{algorithm}}:{{hash_value}}.",
            hash
        )));
    }
    let algorithm = hash.split(':').next().unwrap();

    let hash_actual = compute_filehash(path, algorithm)?;

    if hash_actual == hash {
        if !quiet {
            eprintln!("Hash matches: {:?} == {:?}", path, hash);
        }
        return Ok(true);
    }

    Err(Error::HashMismatch(format!(
        "File hash doesn't match:\nactual: {}\nexpected: {}",
        hash_actual, hash
    )))
}

/// Options for cached download.
#[derive(Debug, Clone)]
pub struct CachedDownloadOptions {
    pub url: Option<String>,
    pub path: Option<String>,
    pub hash: Option<String>,
    pub quiet: bool,
    pub proxy: Option<String>,
    pub speed: Option<f64>,
    pub use_cookies: bool,
    pub verify: bool,
    pub id: Option<String>,
    pub fuzzy: bool,
    pub user_agent: Option<String>,
}

impl Default for CachedDownloadOptions {
    fn default() -> Self {
        Self {
            url: None,
            path: None,
            hash: None,
            quiet: false,
            proxy: None,
            speed: None,
            use_cookies: true,
            verify: true,
            id: None,
            fuzzy: false,
            user_agent: None,
        }
    }
}

/// Cached download from URL.
///
/// Downloads a file and caches it. If the file already exists and the hash
/// matches, returns the cached path without re-downloading.
pub fn cached_download(opts: &CachedDownloadOptions) -> Result<String> {
    let root = cache_root();
    let _ = fs::create_dir_all(&root);

    let path = match &opts.path {
        Some(p) => PathBuf::from(p),
        None => {
            let url = opts.url.as_deref().unwrap_or("");
            let sanitized = url
                .replace('/', "-SLASH-")
                .replace(':', "-COLON-")
                .replace('=', "-EQUAL-")
                .replace('?', "-QUESTION-");
            root.join(sanitized)
        }
    };

    let path_str = path.to_string_lossy().to_string();

    // Check existence
    if path.exists() {
        if opts.hash.is_none() {
            if !opts.quiet {
                eprintln!("File exists: {}", path_str);
            }
            return Ok(path_str);
        }
        if let Some(ref hash) = opts.hash {
            match assert_filehash(&path, hash, opts.quiet) {
                Ok(_) => return Ok(path_str),
                Err(e) => {
                    eprintln!("{}", e);
                    // Continue to re-download
                }
            }
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Download to a temp directory
    let temp_dir = tempfile::tempdir_in(&root)?;
    let temp_path = temp_dir.path().join("dl");

    let download_opts = DownloadOptions {
        url: opts.url.clone(),
        output: Some(temp_path.to_string_lossy().to_string()),
        quiet: opts.quiet,
        proxy: opts.proxy.clone(),
        speed: opts.speed,
        use_cookies: opts.use_cookies,
        verify: opts.verify,
        id: opts.id.clone(),
        fuzzy: opts.fuzzy,
        user_agent: opts.user_agent.clone(),
        ..DownloadOptions::default()
    };

    download(&download_opts)?;

    if let Some(ref hash) = opts.hash {
        assert_filehash(&temp_path, hash, opts.quiet)?;
    }

    // Move to final location
    fs::rename(&temp_path, &path).or_else(|_| {
        // rename may fail across filesystems, fall back to copy+delete
        fs::copy(&temp_path, &path)?;
        fs::remove_file(&temp_path)?;
        Ok::<(), std::io::Error>(())
    })?;

    Ok(path_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_compute_filehash_md5() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"hello world\n").unwrap();
        drop(f);

        let hash = compute_filehash(&file_path, "md5").unwrap();
        assert!(hash.starts_with("md5:"));
    }

    #[test]
    fn test_compute_filehash_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"hello world\n").unwrap();
        drop(f);

        let hash = compute_filehash(&file_path, "sha256").unwrap();
        assert!(hash.starts_with("sha256:"));
    }

    #[test]
    fn test_assert_filehash_invalid_format() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, b"test").unwrap();

        let result = assert_filehash(&file_path, "nocolon", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_assert_filehash_match() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, b"test content").unwrap();

        let hash = compute_filehash(&file_path, "md5").unwrap();
        assert!(assert_filehash(&file_path, &hash, true).is_ok());
    }

    #[test]
    fn test_assert_filehash_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, b"test content").unwrap();

        let result = assert_filehash(&file_path, "md5:0000000000000000000000000000dead", true);
        assert!(result.is_err());
    }
}
