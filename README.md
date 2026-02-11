# gdown-rs

A Rust port of [gdown](https://github.com/wkentaro/gdown) — download files and folders from Google Drive shared links.

Both a **CLI tool** and a **library crate** for use as a dependency.

## Features

- Download files from Google Drive "anyone with the link" shared links
- Download entire shared folders (recursively)
- Download from non-Google-Drive URLs
- Resume interrupted downloads (`--continue`)
- Cached downloads with hash verification (md5, sha1, sha256, sha512)
- Fuzzy file ID extraction from various Google Drive URL formats
- Google Docs/Sheets/Slides export with custom format
- Speed limiting
- Proxy support
- Archive extraction (zip, tar, tar.gz)

## Installation

```bash
cargo install --path .
```

## CLI Usage

```bash
# Download a file by URL
gdown "https://drive.google.com/uc?id=FILE_ID"

# Download a file by ID
gdown FILE_ID

# Download to a specific path
gdown FILE_ID -O output.txt

# Download a shared folder
gdown "https://drive.google.com/drive/folders/FOLDER_ID?usp=sharing" --folder

# Download a folder by ID
gdown FOLDER_ID --folder -O ./output_dir/

# Resume a partial download
gdown FILE_ID -O output.txt --continue

# Fuzzy URL matching
gdown "https://drive.google.com/file/d/FILE_ID/view?usp=sharing" --fuzzy

# Export Google Docs as PDF
gdown DOC_ID --format pdf

# Quiet mode
gdown FILE_ID -q

# With proxy
gdown FILE_ID --proxy http://proxy:8080

# Speed limit
gdown FILE_ID --speed 10MB
```

## Library Usage

Add to your `Cargo.toml`:

```toml
[dependencies]
gdown = { path = "." }
```

### Download a file

```rust
use gdown::{download, DownloadOptions};

let opts = DownloadOptions {
    url: Some("https://drive.google.com/uc?id=FILE_ID".into()),
    output: Some("output.txt".into()),
    quiet: true,
    ..DownloadOptions::default()
};
let result = download(&opts).unwrap();
```

### Download a folder

```rust
use gdown::{download_folder, DownloadFolderOptions};

let opts = DownloadFolderOptions {
    url: Some("https://drive.google.com/drive/folders/FOLDER_ID".into()),
    output: Some("./output/".into()),
    ..DownloadFolderOptions::default()
};
let result = download_folder(&opts).unwrap();
```

### Cached download with hash verification

```rust
use gdown::cached_download::{cached_download, CachedDownloadOptions};

let opts = CachedDownloadOptions {
    url: Some("https://drive.google.com/uc?id=FILE_ID".into()),
    hash: Some("md5:abc123...".into()),
    ..CachedDownloadOptions::default()
};
let path = cached_download(&opts).unwrap();
```

### Parse Google Drive URLs

```rust
use gdown::parse_url;

let (file_id, is_download_link) = parse_url(
    "https://drive.google.com/file/d/FILE_ID/view?usp=sharing",
    false,
);
```

## Testing

```bash
cargo test
```

## License

MIT
