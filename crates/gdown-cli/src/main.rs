use std::process;
use std::sync::Arc;

use clap::Parser;
use gdown::download_folder::DownloadFolderOptions;
use gdown::error::Error;
use gdown::progress::IndicatifProgress;
use gdown::types::CommonOptions;
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
#[allow(
    clippy::struct_excessive_bools,
    reason = "CLI flag structs naturally contain many booleans"
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

    /// (folder only) Allow downloading folders with more than 50 files
    #[arg(long)]
    ignore_file_limit: bool,

    /// Format of Google Docs, Spreadsheets and Slides.
    /// Default is Google Docs: 'docx', Spreadsheet: 'xlsx', Slides: 'pptx'.
    #[arg(long)]
    format: Option<String>,

    /// User-Agent to use for downloading file
    #[arg(long)]
    user_agent: Option<String>,

    /// Don't show progress bar
    #[arg(long)]
    no_progress: bool,
}

impl Cli {
    fn common_options(&self) -> CommonOptions {
        let progress = if self.quiet || self.no_progress {
            None
        } else {
            Some(Arc::new(IndicatifProgress::new()) as Arc<dyn gdown::Progress>)
        };

        CommonOptions {
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
            common: cli.common_options(),
            ignore_file_limit: cli.ignore_file_limit,
        };
        gdown::download_folder(&opts).await.map(|_| ())
    } else {
        let opts = DownloadOptions {
            url,
            id,
            common: cli.common_options(),
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
