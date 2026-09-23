// Contract stub. The history track replaces every `todo!()` and keeps the public
// signatures unchanged, the queue track codes against them.

use std::path::PathBuf;

use yaydl_shared::{DownloadId, HistoryEntry, OutputFormat, Statistics, StatsGranularity};

use crate::persist::PersistError;

pub const HISTORY_FILE: &str = "history.json";

pub struct History {
    #[allow(dead_code)]
    path: PathBuf,
    #[allow(dead_code)]
    entries: Vec<HistoryEntry>,
}

impl History {
    /// Loads `path`. A missing file is an empty history. An invalid file or an
    /// unknown version is an error, the caller quarantines it and starts with
    /// [`History::empty`].
    pub fn load(path: PathBuf) -> Result<Self, PersistError> {
        let _ = path;
        todo!("history track")
    }

    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            entries: Vec::new(),
        }
    }

    /// Oldest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        todo!("history track")
    }

    pub fn newest_first(&self) -> Vec<HistoryEntry> {
        todo!("history track")
    }

    /// Appends and saves.
    pub fn record(&mut self, entry: HistoryEntry) -> Result<(), PersistError> {
        let _ = entry;
        todo!("history track")
    }

    /// After a rename on disk. Returns whether an entry matched, and saves if so.
    pub fn update_file_path(
        &mut self,
        download_id: DownloadId,
        new_path: PathBuf,
    ) -> Result<bool, PersistError> {
        let _ = (download_id, new_path);
        todo!("history track")
    }

    /// The most recent entry with the same extractor (case-insensitive), video
    /// id and format. Does not check whether the file still exists.
    pub fn find_previous(
        &self,
        extractor: &str,
        video_id: &str,
        format: &OutputFormat,
    ) -> Option<&HistoryEntry> {
        let _ = (extractor, video_id, format);
        todo!("history track")
    }

    pub fn clear(&mut self) -> Result<(), PersistError> {
        todo!("history track")
    }
}

/// Pure, so it is testable with fixed entries and a fixed `now`.
pub fn compute_statistics(
    entries: &[HistoryEntry],
    granularity: StatsGranularity,
    now: &jiff::Zoned,
) -> Statistics {
    let _ = (entries, granularity, now);
    todo!("history track")
}
