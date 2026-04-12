pub mod client;
pub mod download;
pub mod download_folder;
pub mod error;
pub mod extract;
pub mod parse;
pub mod types;

pub use download::{DownloadOptions, download};
pub use download_folder::{
    DirEntry, DownloadFolderOptions, DriveFolder, FlatEntry, download_folder,
};
pub use error::{Error, Result};
pub use extract::extractall;
pub use parse::{ParsedUrl, is_google_drive_url, parse_url};
pub use types::CommonOptions;
