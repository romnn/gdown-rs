use std::process;

use clap::Parser;
use gdown::download_folder::DownloadFolderOptions;
use gdown::error::Error;
use gdown::{DownloadOptions, download};

/// Parse a file size string like "10MB" into bytes.
fn parse_file_size(s: &str) -> Result<f64, String> {
    let s = s.trim();
    let (num_str, unit) = if let Some(prefix) = s.strip_suffix("GB") {
        (prefix, "GB")
    } else if let Some(prefix) = s.strip_suffix("MB") {
        (prefix, "MB")
    } else if let Some(prefix) = s.strip_suffix("KB") {
        (prefix, "KB")
    } else if let Some(prefix) = s.strip_suffix('B') {
        (prefix, "B")
    } else {
        return Err(format!("Invalid size format: '{s}'. Use e.g. 10MB, 256KB"));
    };

    if num_str.is_empty() || !num_str.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("Invalid number: {num_str}"));
    }

    let size: f64 = num_str
        .parse()
        .map_err(|_| format!("Invalid number: {num_str}"))?;
    let bytes = match unit {
        "B" => size,
        "KB" => size * 1024.0,
        "MB" => size * 1024.0 * 1024.0,
        "GB" => size * 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("Invalid size unit: '{unit}'. Use e.g. 10MB, 256KB")),
    };

    Ok(bytes)
}

#[derive(Parser, Debug)]
#[command(
    name = "gdown",
    version,
    about = "Download files from Google Drive and other URLs"
)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "CLI flag structs naturally contain many booleans; refactoring into enums/state machines would hurt ergonomics"
)]
struct Cli {
    /// URL or file/folder ID to download from
    url_or_id: String,

    /// Output file name/path; end with "/" to create a new directory
    #[arg(short = 'O', long)]
    output: Option<String>,

    /// Suppress logging except errors
    #[arg(short, long)]
    quiet: bool,

    /// (file only) Extract Google Drive's file ID from a fuzzy URL
    #[arg(long)]
    fuzzy: bool,

    /// <protocol://host:port> download using the specified proxy
    #[arg(long)]
    proxy: Option<String>,

    /// Download speed limit per second (e.g., '10MB' -> 10MB/s)
    #[arg(long, value_parser = parse_file_size)]
    speed: Option<f64>,

    /// Don't use cookies
    #[arg(long)]
    no_cookies: bool,

    /// Don't check the server's TLS certificate
    #[arg(long)]
    no_check_certificate: bool,

    /// Resume getting partially-downloaded files while skipping fully downloaded ones
    #[arg(short = 'c', long = "continue")]
    resume: bool,

    /// Download entire folder instead of a single file
    #[arg(long)]
    folder: bool,

    /// (folder only) Asserts that it is ok to download max files per folder
    #[arg(long)]
    remaining_ok: bool,

    /// Format of Google Docs, Spreadsheets and Slides.
    /// Default is Google Docs: 'docx', Spreadsheet: 'xlsx', Slides: 'pptx'.
    #[arg(long)]
    format: Option<String>,

    /// User-Agent to use for downloading file
    #[arg(long)]
    user_agent: Option<String>,
}

fn main() {
    let cli = Cli::parse();

    // Determine if input is URL or ID
    let (url, id) = if cli.url_or_id.starts_with("http://") || cli.url_or_id.starts_with("https://")
    {
        (Some(cli.url_or_id.clone()), None)
    } else {
        (None, Some(cli.url_or_id.clone()))
    };

    let result = if cli.folder {
        let opts = DownloadFolderOptions {
            url,
            id,
            output: cli.output,
            quiet: cli.quiet,
            proxy: cli.proxy,
            speed: cli.speed,
            use_cookies: !cli.no_cookies,
            remaining_ok: cli.remaining_ok,
            verify: !cli.no_check_certificate,
            user_agent: cli.user_agent,
            skip_download: false,
            resume: cli.resume,
        };
        gdown::download_folder(&opts).map(|_| ())
    } else {
        let opts = DownloadOptions {
            url,
            output: cli.output,
            quiet: cli.quiet,
            proxy: cli.proxy,
            speed: cli.speed,
            use_cookies: !cli.no_cookies,
            verify: !cli.no_check_certificate,
            id,
            fuzzy: cli.fuzzy,
            resume: cli.resume,
            format: cli.format,
            user_agent: cli.user_agent,
        };
        download(&opts).map(|_| ())
    };

    if let Err(e) = result {
        match e {
            Error::FileURLRetrieval(ref msg) => {
                eprintln!("{msg}");
                process::exit(1);
            }
            Error::FolderContentsMaximumLimit(ref msg) => {
                eprintln!(
                    "Failed to retrieve folder contents:\n\n\t{msg}\n\n\
                     You can use `--remaining-ok` option to ignore this error."
                );
                process::exit(1);
            }
            _ => {
                eprintln!("Error:\n\n\t{e}");
                process::exit(1);
            }
        }
    }
}
