use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
