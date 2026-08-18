# gdown (Rust)

This repository is a Rust port of the upstream Python project:

- https://github.com/wkentaro/gdown

It provides:

- A `gdown` **CLI** that mirrors the upstream CLI surface as closely as practical.
- A `gdown` **library crate** for embedding in other Rust projects.

The main goal is the same as upstream: download files (and folders) from Google Drive links that are shared as **"Anyone with the link"**.

## What it does

- **Google Drive files**
  - Accepts file IDs (e.g. `1abc...`) and a variety of Google Drive URL shapes.
  - Handles Google Drive confirmation pages (virus-scan / quota / large file warnings).
  - Supports Google Docs/Sheets/Slides export via `--format`.

- **Google Drive folders**
  - Accepts folder IDs and folder URLs.
  - Recursively downloads the folder tree.
  - Matches upstream’s “max 50 entries” behavior unless `--remaining-ok` is passed.

- **Streaming by default**
  - File bodies are streamed to disk (or stdout) using a fixed-size buffer.
  - Confirmation HTML pages are read with a hard limit (`512KiB`) to avoid accidental RAM blowups.

## Installation

### CLI

```bash
cargo install --locked --git https://github.com/romnn/gdown-rs gdown-cli
```

## CLI usage

### Download a file

```bash
gdown FILE_ID
gdown "https://drive.google.com/uc?id=FILE_ID"
gdown "https://drive.google.com/file/d/FILE_ID/view?usp=sharing" --fuzzy
```

### Choose output path

```bash
gdown FILE_ID -O output.bin
gdown FILE_ID -O ./downloads/
```

### Stream to stdout

```bash
gdown FILE_ID -O - > output.bin
```

### Resume

```bash
gdown FILE_ID -O output.bin --continue
```

### Download a folder

```bash
gdown FOLDER_ID --folder -O ./out/
gdown "https://drive.google.com/drive/folders/FOLDER_ID?usp=sharing" --folder -O ./out/
```

### Export Google Docs/Sheets/Slides

```bash
gdown DOC_ID --format pdf
gdown SHEET_ID --format xlsx
gdown SLIDES_ID --format pdf
```

### Proxy / TLS / speed limiting

```bash
gdown FILE_ID --proxy http://proxy:8080
gdown FILE_ID --no-check-certificate
gdown FILE_ID --speed 10MB
```

## Library usage

Add to `Cargo.toml`:

```toml
[dependencies]
gdown = { git = "https://github.com/romnn/gdown-rs" }
```

Rustls is the default TLS backend. To use the platform-native TLS backend instead:

```toml
[dependencies]
gdown = { git = "https://github.com/romnn/gdown-rs", default-features = false, features = ["native-tls"] }
```

On Linux, `native-tls` uses OpenSSL. The `native-tls-vendored` feature builds a vendored copy of
OpenSSL. The `default-tls`, `native-tls-no-alpn`, `native-tls-vendored-no-alpn`, `rustls`, and
`rustls-no-provider` features also forward directly to Reqwest. Applications using
`rustls-no-provider` must install a process-wide Rustls crypto provider before constructing a
client.

### Download a file

```rust
use gdown::{download, DownloadOptions};

fn main() -> Result<(), gdown::Error> {
    let opts = DownloadOptions {
        url: Some("https://drive.google.com/uc?id=FILE_ID".to_string()),
        output: Some("output.bin".to_string()),
        quiet: true,
        ..DownloadOptions::default()
    };

    let _path = download(&opts)?;
    Ok(())
}
```

### Download a folder

```rust
use gdown::{download_folder, DownloadFolderOptions};

fn main() -> Result<(), gdown::Error> {
    let opts = DownloadFolderOptions {
        url: Some("https://drive.google.com/drive/folders/FOLDER_ID".to_string()),
        output: Some("./out/".to_string()),
        ..DownloadFolderOptions::default()
    };

    let _result = download_folder(&opts)?;
    Ok(())
}
```

## Tests

```bash
mise install
task test
task lint:fc
```

## Notes / differences from upstream

- This port aims to match upstream behavior closely, but it is not a byte-for-byte reimplementation.
- Folder listings follow upstream’s pragmatic limitation (50 items per folder page) unless overridden with `--remaining-ok`.

## License

MIT
