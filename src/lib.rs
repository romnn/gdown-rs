pub mod download;
pub mod download_folder;
pub mod error;
pub mod extractall;
pub mod parse_url;

pub use download::{DownloadOptions, download};
pub use download_folder::{
    DirEntry, DownloadFolderOptions, DownloadFolderResult, DriveFolder, FlatEntry,
    GoogleDriveFileToDownload, MAX_NUMBER_FILES, download_folder,
};
pub use error::{Error, Result};
pub use extractall::extractall;
pub use parse_url::{is_google_drive_url, parse_url};
