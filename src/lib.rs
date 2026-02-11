pub mod download;
pub mod download_folder;
pub mod error;
pub mod extractall;
pub mod parse_url;

pub use download::{download, DownloadOptions};
pub use download_folder::{
    download_folder, DownloadFolderOptions, DownloadFolderResult, GoogleDriveFileToDownload,
    MAX_NUMBER_FILES,
};
pub use error::{Error, Result};
pub use extractall::extractall;
pub use parse_url::{is_google_drive_url, parse_url};
