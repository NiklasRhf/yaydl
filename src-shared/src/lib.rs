use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    pub output_dir: PathBuf,
    pub output_format: String,
    pub dark_theme: bool,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct Metadata {
    pub id: String,
    pub url: String,
    pub title: String,
    pub duration: String,
    pub thumbnail: String,
    pub loading: bool,
}

#[derive(Serialize, Deserialize)]
pub struct MetadataArgs<'a> {
    pub url: &'a str,
    pub id: &'a str,
}

#[derive(Debug, Default, Clone)]
pub struct Download {
    pub metadata: Metadata,
    pub download_state: DownloadState,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub enum DownloadState {
    #[default]
    Idle,
    Loading(u8),
    Finished,
    Failure,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadEvent {
    pub id: String,
    pub progress: u8,
}

#[derive(Error, Serialize, Deserialize, Debug)]
pub enum AddLinkError {
    #[error("Video has already been added")]
    AlreadyAdded,
    #[error("Clipboard doesn't contain a valid link")]
    NoValidLink,
    #[error("Clipboard read error")]
    ClipboardRead,
}

#[derive(Error, Serialize, Deserialize, Debug)]
pub enum MetadataError {
    #[error("Retreiving metadata failed")]
    RetreivalFailed,
    #[error("Metadata parsing failed")]
    ParsingFailed,
    #[error("Insufficient metadata fields")]
    MissingFields,
}

#[derive(Error, Serialize, Deserialize, Debug)]
pub enum UpdateError {
    #[error("Checking for updates failed")]
    CheckFailed,
    #[error("Building updater failed")]
    BuildFailed,
    #[error("Downloading and installing updates failed")]
    DownloadAndInstallFailed,
}

#[derive(Error, Serialize, Deserialize, Debug)]
pub enum YaydlError {
    #[error(transparent)]
    AddLinkError(#[from] AddLinkError),
    #[error("Shell error: {0}")]
    TauriShellError(String),
    #[error(transparent)]
    MetadataError(#[from] MetadataError),
    #[error(transparent)]
    UpdateError(#[from] UpdateError),
    #[error("Failed to convert output to UTF-8")]
    Utf8Conversion,
    #[error("Unsupported operating system")]
    UnsupportedOs,
    #[error("Folder selection failed")]
    FolderSelectionFailed,
}
