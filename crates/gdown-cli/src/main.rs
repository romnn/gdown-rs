//! Provides a command-line interface for downloading public Google Drive files and folders.

use std::process;
use std::sync::Arc;

use clap::Parser;
use gdown::download_folder::DownloadFolderOptions;
use gdown::error::Error;
use gdown::progress::IndicatifProgress;
use gdown::types::Options;
use gdown::{DownloadOptions, download};
use tracing_subscriber::EnvFilter;

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
        return Err(format!("invalid size format: '{s}'. Use e.g. 10MB, 1.5GB"));
    };

    let size: f64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("invalid number: '{num_str}'"))?;

    if size < 0.0 {
        return Err(format!("size must be non-negative, got {size}"));
    }

    let bytes = match unit {
        "B" => size,
        "KB" => size * 1024.0,
        "MB" => size * 1024.0 * 1024.0,
        "GB" => size * 1024.0 * 1024.0 * 1024.0,
        _ => return Err(format!("invalid size unit: '{unit}'")),
    };

    Ok(bytes)
}

#[derive(Parser, Debug)]
#[command(
    name = "gdown",
    version,
    about = "Download files from Google Drive and other URLs"
)]
struct Cli {
    /// Selects the source URL or Google Drive file or folder ID.
    url_or_id: String,

    /// Selects the output file or directory path.
    ///
    /// End a directory path with the platform's path separator.
    #[arg(short = 'O', long)]
    output: Option<String>,

    /// Suppresses all logging except errors.
    #[arg(short, long)]
    quiet: bool,

    /// Extracts a file ID from a non-direct Google Drive URL.
    #[arg(long)]
    fuzzy: bool,

    /// Routes downloads through a proxy such as `http://host:port`.
    #[arg(long)]
    proxy: Option<String>,

    /// Limits download speed using a byte suffix such as `10MB`.
    #[arg(long, value_parser = parse_file_size)]
    speed: Option<f64>,

    /// Disables the HTTP cookie store.
    #[arg(long)]
    no_cookies: bool,

    /// Accepts invalid server TLS certificates.
    #[arg(long)]
    no_check_certificate: bool,

    /// Resumes partial downloads and skips files that already exist.
    #[arg(short = 'c', long = "continue")]
    resume: bool,

    /// Downloads an entire folder instead of a single file.
    #[arg(long)]
    folder: bool,

    /// Allows folder downloads containing 50 or more files.
    #[arg(long)]
    ignore_file_limit: bool,

    /// Selects the export format for Google Docs, Sheets, or Slides.
    ///
    /// Defaults are `docx` for Docs, `xlsx` for Sheets, and `pptx` for Slides.
    #[arg(long)]
    format: Option<String>,

    /// Overrides the default HTTP user agent.
    #[arg(long)]
    user_agent: Option<String>,

    /// Disables the terminal progress bar.
    #[arg(long)]
    no_progress: bool,
}

impl Cli {
    fn options(&self) -> Options {
        let progress = if self.quiet || self.no_progress {
            None
        } else {
            Some(Arc::new(IndicatifProgress::new()) as Arc<dyn gdown::Progress>)
        };

        Options {
            output: self.output.clone(),
            quiet: self.quiet,
            proxy: self.proxy.clone(),
            speed: self.speed,
            use_cookies: !self.no_cookies,
            verify: !self.no_check_certificate,
            user_agent: self.user_agent.clone(),
            resume: self.resume,
            progress,
        }
    }
}

#[tracing::instrument(
    name = "gdown",
    level = "info",
    skip(cli),
    fields(folder = cli.folder, quiet = cli.quiet, fuzzy = cli.fuzzy, resume = cli.resume)
)]
async fn run(cli: Cli) -> i32 {
    let (url, id) = if cli.url_or_id.starts_with("http://") || cli.url_or_id.starts_with("https://")
    {
        (Some(cli.url_or_id.clone()), None)
    } else {
        (None, Some(cli.url_or_id.clone()))
    };

    tracing::info!(url = ?url, id = ?id, "starting");

    let result = if cli.folder {
        let opts = DownloadFolderOptions {
            url,
            id,
            options: cli.options(),
            ignore_file_limit: cli.ignore_file_limit,
        };
        gdown::download_folder(&opts).await.map(|_| ())
    } else {
        let opts = DownloadOptions {
            url,
            id,
            options: cli.options(),
            fuzzy: cli.fuzzy,
            format: cli.format,
        };
        download(&opts).await.map(|_| ())
    };

    if let Err(e) = result {
        match e {
            Error::FileUrlRetrieval(ref msg) => {
                tracing::error!(message = %msg, "file url retrieval failed");
            }
            Error::FolderLimit(ref msg) => {
                tracing::error!(message = %msg, "folder file limit exceeded");
            }
            _ => {
                tracing::error!(error = %e, "unhandled error");
            }
        }
        return 1;
    }

    0
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if cli.quiet {
            EnvFilter::new("error")
        } else {
            EnvFilter::new("info")
        }
    });
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    process::exit(run(cli).await);
}
