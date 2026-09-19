use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Settings {
    pub output_dir: PathBuf,
    pub output_format: String,
    pub dark_theme: bool,
    #[serde(default)]
    pub yt_dlp_channel: YtDlpChannel,
}

/// yt-dlp release channel passed to `yt-dlp --update-to <channel>`.
/// Nightly is the default because YouTube extractor fixes land there days
/// before a stable release.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum YtDlpChannel {
    Stable,
    #[default]
    Nightly,
}

impl YtDlpChannel {
    pub const ALL: [YtDlpChannel; 2] = [YtDlpChannel::Stable, YtDlpChannel::Nightly];

    pub fn as_str(self) -> &'static str {
        match self {
            YtDlpChannel::Stable => "stable",
            YtDlpChannel::Nightly => "nightly",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "stable" => Some(YtDlpChannel::Stable),
            "nightly" => Some(YtDlpChannel::Nightly),
            _ => None,
        }
    }
}

impl fmt::Display for YtDlpChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct YtDlpStatus {
    pub version: String,
    pub channel: YtDlpChannel,
    pub binary_path: String,
}

/// Result of a yt-dlp self-update. Emitted as the `ytdlp-update` event by the
/// background startup check and returned by the `update_yt_dlp` command.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum YtDlpUpdateEvent {
    Updated {
        from: String,
        to: String,
        channel: YtDlpChannel,
    },
    AlreadyCurrent {
        version: String,
        channel: YtDlpChannel,
    },
    Failed {
        message: String,
    },
}

#[derive(Serialize, Deserialize)]
pub struct ChannelArgs {
    pub value: YtDlpChannel,
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

#[derive(Serialize, Deserialize)]
pub struct DownloadStateArgs {
    pub id: String,
    pub state: DownloadState,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Download {
    pub metadata: Metadata,
    pub download_state: DownloadState,
}

impl PartialEq for Download {
    fn eq(&self, other: &Self) -> bool {
        self.metadata.url == other.metadata.url
    }
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub enum DownloadState {
    #[default]
    Idle,
    Loading(u8),
    Finished,
    Failure,
    MetadataLoading,
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
pub enum YtDlpError {
    #[error("yt-dlp failed (exit code {exit_code:?}): {stderr}")]
    CommandFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    #[error("yt-dlp update to {channel} failed: {stderr}")]
    UpdateFailed {
        channel: YtDlpChannel,
        stderr: String,
    },
    #[error("installing bundled yt-dlp failed: {0}")]
    Bootstrap(String),
    #[error("yt-dlp state file is invalid: {0}")]
    InvalidState(String),
    /// Kept apart from `CommandFailed` because a failure to even run yt-dlp is
    /// not something a yt-dlp update can repair.
    #[error("yt-dlp I/O error: {0}")]
    Io(String),
}

#[derive(Error, Serialize, Deserialize, Debug)]
pub enum YaydlError {
    #[error(transparent)]
    AddLinkError(#[from] AddLinkError),
    #[error(transparent)]
    YtDlp(#[from] YtDlpError),
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
    #[error("Unknown log level \"{0}\"")]
    UnknownLogLevel(String),
    #[error("Reading the log file failed: {0}")]
    LogsUnavailable(String),
    #[error("Writing to the clipboard failed: {0}")]
    ClipboardWrite(String),
}

/// The tail of the log file, for the in-app log viewer.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LogSnapshot {
    pub path: String,
    pub app_version: String,
    pub lines: Vec<String>,
    /// `true` when the file had more lines than were returned.
    pub truncated: bool,
}

#[derive(Serialize, Deserialize)]
pub struct LogArgs<'a> {
    pub level: &'a str,
    pub message: String,
}

/// Tauri looks up command arguments by their camelCase name.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentLogsArgs {
    pub max_lines: usize,
}

#[derive(Serialize, Deserialize)]
pub struct ClipboardArgs {
    pub text: String,
}
