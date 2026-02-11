use std::process;

use clap::Parser;
use regex::Regex;

use gdown::download_folder::DownloadFolderOptions;
use gdown::error::Error;
use gdown::{download, DownloadOptions};

/// Parse a file size string like "10MB" into bytes.
fn parse_file_size(s: &str) -> Result<f64, String> {
    let re = Regex::new(r"^([0-9]+)(GB|MB|KB|B)$").unwrap();
    let caps = re
        .captures(s)
        .ok_or_else(|| format!("Invalid size format: '{}'. Use e.g. 10MB, 256KB", s))?;
    let size: f64 = caps[1]
        .parse()
        .map_err(|_| format!("Invalid number: {}", &caps[1]))?;
    let unit = &caps[2];
    let bytes = match unit {
        "B" => size,
        "KB" => size * 1024.0,
        "MB" => size * 1024.0 * 1024.0,
        "GB" => size * 1024.0 * 1024.0 * 1024.0,
        _ => unreachable!(),
    };
    Ok(bytes)
}

#[derive(Parser, Debug)]
#[command(name = "gdown", version, about = "Download files from Google Drive and other URLs")]
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
    let (url, id) = if Regex::new(r"^https?://").unwrap().is_match(&cli.url_or_id) {
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
                eprintln!("{}", msg);
                process::exit(1);
            }
            Error::FolderContentsMaximumLimit(ref msg) => {
                eprintln!(
                    "Failed to retrieve folder contents:\n\n\t{}\n\n\
                     You can use `--remaining-ok` option to ignore this error.",
                    msg
                );
                process::exit(1);
            }
            _ => {
                eprintln!("Error:\n\n\t{}", e);
                process::exit(1);
            }
        }
    }
}
