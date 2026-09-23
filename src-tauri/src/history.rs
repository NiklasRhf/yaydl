use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use jiff::{civil::Date, Timestamp, ToSpan};
use serde::{Deserialize, Serialize};
use tracing::error;
use yaydl_shared::{
    DayCount, DownloadId, HistoryEntry, NameCount, OutputFormat, Statistics, StatsBucket,
    StatsGranularity, StatsTotals,
};

use crate::persist::{read_json, write_json_atomic, PersistError};

pub const HISTORY_FILE: &str = "history.json";
const HISTORY_VERSION: u64 = 1;
const TOP_UPLOADERS: usize = 5;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryFile {
    #[serde(rename = "version")]
    _version: u64,
    entries: Vec<HistoryEntry>,
}

#[derive(Serialize)]
struct HistoryFileRef<'a> {
    version: u64,
    entries: &'a [HistoryEntry],
}

pub struct History {
    path: PathBuf,
    entries: Vec<HistoryEntry>,
}

impl History {
    /// Loads `path`. A missing file is an empty history. An invalid file or an
    /// unknown version is an error, the caller quarantines it and starts with
    /// [`History::empty`].
    pub fn load(path: PathBuf) -> Result<Self, PersistError> {
        let Some(raw) = read_json::<serde_json::Value>(&path)? else {
            return Ok(Self::empty(path));
        };
        // The version is checked before the entries are parsed, so a file from
        // a newer app reports a version mismatch instead of a field error.
        let version = match raw.get("version") {
            None => {
                return Err(PersistError::new(
                    &path,
                    "history has no \"version\" field or is not a JSON object",
                ))
            }
            Some(v) => v.as_u64().ok_or_else(|| {
                PersistError::new(
                    &path,
                    format!("history version is {v}, expected {HISTORY_VERSION}"),
                )
            })?,
        };
        if version != HISTORY_VERSION {
            return Err(PersistError::new(
                &path,
                format!("history version is {version}, expected {HISTORY_VERSION}"),
            ));
        }
        let file: HistoryFile = serde_json::from_value(raw)
            .map_err(|e| PersistError::new(&path, format!("invalid history: {e}")))?;
        for entry in &file.entries {
            validate_entry(&path, entry)?;
        }
        let mut entries = file.entries;
        entries.sort_by_key(|e| e.finished_at_ms);
        Ok(Self { path, entries })
    }

    pub fn empty(path: PathBuf) -> Self {
        Self {
            path,
            entries: Vec::new(),
        }
    }

    /// Oldest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    pub fn newest_first(&self) -> Vec<HistoryEntry> {
        self.entries.iter().rev().cloned().collect()
    }

    /// Appends and saves.
    pub fn record(&mut self, entry: HistoryEntry) -> Result<(), PersistError> {
        validate_entry(&self.path, &entry)?;
        // Inserting after every entry with an equal timestamp keeps the order a
        // stable sort of the appended list would produce.
        let index = self
            .entries
            .partition_point(|e| e.finished_at_ms <= entry.finished_at_ms);
        self.entries.insert(index, entry);
        if let Err(e) = self.save() {
            self.entries.remove(index);
            return Err(e);
        }
        Ok(())
    }

    /// After a rename on disk. Returns whether an entry matched, and saves if so.
    pub fn update_file_path(
        &mut self,
        download_id: DownloadId,
        new_path: PathBuf,
    ) -> Result<bool, PersistError> {
        // Newest first, because a rename always concerns a download that just
        // finished, and ids are not guaranteed unique across app runs.
        let Some(index) = self
            .entries
            .iter()
            .rposition(|e| e.download_id == download_id)
        else {
            return Ok(false);
        };
        let old_path = std::mem::replace(&mut self.entries[index].file_path, new_path);
        if let Err(e) = self.save() {
            self.entries[index].file_path = old_path;
            return Err(e);
        }
        Ok(true)
    }

    /// The most recent entry with the same extractor (case-insensitive), video
    /// id and format. Does not check whether the file still exists.
    pub fn find_previous(
        &self,
        extractor: &str,
        video_id: &str,
        format: &OutputFormat,
    ) -> Option<&HistoryEntry> {
        self.entries.iter().rev().find(|e| {
            e.video_id == video_id
                && e.format == *format
                && e.extractor
                    .chars()
                    .flat_map(char::to_lowercase)
                    .eq(extractor.chars().flat_map(char::to_lowercase))
        })
    }

    pub fn clear(&mut self) -> Result<(), PersistError> {
        write_json_atomic(
            &self.path,
            &HistoryFileRef {
                version: HISTORY_VERSION,
                entries: &[],
            },
        )?;
        self.entries.clear();
        Ok(())
    }

    fn save(&self) -> Result<(), PersistError> {
        write_json_atomic(
            &self.path,
            &HistoryFileRef {
                version: HISTORY_VERSION,
                entries: &self.entries,
            },
        )
    }
}

/// Statistics rely on every timestamp being convertible to a local date, so
/// entries that are not are rejected at the persistence boundary.
fn validate_entry(path: &Path, entry: &HistoryEntry) -> Result<(), PersistError> {
    Timestamp::from_millisecond(entry.finished_at_ms).map_err(|e| {
        PersistError::new(
            path,
            format!(
                "history entry {} has finished_at_ms {} outside the supported range: {e}",
                entry.download_id, entry.finished_at_ms
            ),
        )
    })?;
    if let Some(duration) = entry.duration_secs {
        if !(duration.is_finite() && duration >= 0.0) {
            return Err(PersistError::new(
                path,
                format!(
                    "history entry {} has duration_secs {duration}, expected a finite value >= 0",
                    entry.download_id
                ),
            ));
        }
    }
    Ok(())
}

/// Pure, so it is testable with fixed entries and a fixed `now`.
pub fn compute_statistics(
    entries: &[HistoryEntry],
    granularity: StatsGranularity,
    now: &jiff::Zoned,
) -> Statistics {
    let tz = now.time_zone();
    let today = now.date();
    let starts = bucket_starts(granularity, today);
    let bucket_index: HashMap<Date, usize> =
        starts.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut buckets: Vec<StatsBucket> = starts
        .iter()
        .map(|start| StatsBucket {
            start_date: start.to_string(),
            count: 0,
            bytes: 0,
        })
        .collect();

    let mut totals = StatsTotals::default();
    let mut activity = vec![[0u32; 24]; 7];
    let mut per_day: BTreeMap<Date, u32> = BTreeMap::new();
    let mut uploaders: HashMap<&str, u32> = HashMap::new();
    let mut formats: HashMap<String, u32> = HashMap::new();

    for entry in entries {
        let bytes = entry.file_size_bytes.unwrap_or(0);
        totals.downloads += 1;
        totals.bytes += bytes;
        totals.duration_secs += entry.duration_secs.unwrap_or(0.0);
        totals.first_download_ms = Some(
            totals
                .first_download_ms
                .map_or(entry.finished_at_ms, |first| {
                    first.min(entry.finished_at_ms)
                }),
        );
        if let Some(uploader) = entry.uploader.as_deref() {
            *uploaders.entry(uploader).or_default() += 1;
        }
        *formats.entry(entry.format.to_string()).or_default() += 1;

        // `History` rejects these on load and record, so this only triggers for
        // entries that bypassed it.
        let local = match Timestamp::from_millisecond(entry.finished_at_ms) {
            Ok(ts) => ts.to_zoned(tz.clone()),
            Err(e) => {
                error!(
                    download_id = entry.download_id,
                    finished_at_ms = entry.finished_at_ms,
                    "history entry left out of calendar statistics, timestamp out of range: {e}"
                );
                continue;
            }
        };
        let date = local.date();
        let weekday = local.weekday().to_monday_zero_offset() as usize;
        activity[weekday][local.hour() as usize] += 1;
        *per_day.entry(date).or_default() += 1;

        if date >= starts[0] {
            let start = period_start(granularity, date)
                .expect("a date inside the window has a representable period start");
            if let Some(&i) = bucket_index.get(&start) {
                buckets[i].count += 1;
                buckets[i].bytes += bytes;
            }
        }
    }
    totals.distinct_uploaders = uploaders.len() as u32;

    let busiest_weekday = first_argmax(activity.iter().map(|row| row.iter().sum()));
    let busiest_hour = first_argmax((0..24).map(|hour| activity.iter().map(|row| row[hour]).sum()));
    let mut busiest_day: Option<(Date, u32)> = None;
    for (&date, &count) in &per_day {
        if busiest_day.is_none_or(|(_, best)| count >= best) {
            busiest_day = Some((date, count));
        }
    }
    let (current_streak_days, longest_streak_days) = streaks(&per_day, today);

    let mut top_uploaders = ranked(
        uploaders
            .into_iter()
            .map(|(name, count)| (name.to_string(), count)),
    );
    top_uploaders.truncate(TOP_UPLOADERS);

    Statistics {
        granularity,
        buckets,
        totals,
        activity,
        busiest_weekday,
        busiest_hour,
        busiest_day: busiest_day.map(|(date, count)| DayCount {
            date: date.to_string(),
            count,
        }),
        current_streak_days,
        longest_streak_days,
        top_uploaders,
        formats: ranked(formats.into_iter()),
    }
}

fn period_start(granularity: StatsGranularity, date: Date) -> Result<Date, jiff::Error> {
    match granularity {
        StatsGranularity::Day => Ok(date),
        StatsGranularity::Week => {
            date.checked_sub(i64::from(date.weekday().to_monday_zero_offset()).days())
        }
        StatsGranularity::Month => Ok(date.first_of_month()),
    }
}

fn bucket_starts(granularity: StatsGranularity, today: Date) -> Vec<Date> {
    let current = period_start(granularity, today)
        .expect("the current period starts within jiff's supported date range");
    let count = granularity.bucket_count() as i64;
    (0..count)
        .rev()
        .map(|back| {
            let span = match granularity {
                StatsGranularity::Day => back.days(),
                StatsGranularity::Week => back.weeks(),
                StatsGranularity::Month => back.months(),
            };
            current
                .checked_sub(span)
                .expect("the statistics window starts within jiff's supported date range")
        })
        .collect()
}

/// Index of the largest non-zero value, the smaller index on ties.
fn first_argmax(values: impl Iterator<Item = u32>) -> Option<u8> {
    let mut best: Option<(u8, u32)> = None;
    for (i, value) in (0u8..).zip(values) {
        if value > 0 && best.is_none_or(|(_, b)| value > b) {
            best = Some((i, value));
        }
    }
    best.map(|(i, _)| i)
}

/// `(current, longest)` over the days that have at least one download.
fn streaks(per_day: &BTreeMap<Date, u32>, today: Date) -> (u32, u32) {
    let follows = |day: Date, prev: Date| day.yesterday().is_ok_and(|y| y == prev);

    let mut longest = 0;
    let mut run = 0;
    let mut prev: Option<Date> = None;
    for &day in per_day.keys() {
        run = if prev.is_some_and(|p| follows(day, p)) {
            run + 1
        } else {
            1
        };
        longest = longest.max(run);
        prev = Some(day);
    }

    let anchor = if per_day.contains_key(&today) {
        Some(today)
    } else {
        today.yesterday().ok().filter(|y| per_day.contains_key(y))
    };
    let mut current = 0;
    if let Some(anchor) = anchor {
        let mut expected = anchor;
        for (&day, _) in per_day.range(..=anchor).rev() {
            if day != expected {
                break;
            }
            current += 1;
            match day.yesterday() {
                Ok(y) => expected = y,
                Err(_) => break,
            }
        }
    }
    (current, longest)
}

/// Count descending, name ascending on ties.
fn ranked(counts: impl Iterator<Item = (String, u32)>) -> Vec<NameCount> {
    let mut list: Vec<NameCount> = counts
        .map(|(name, count)| NameCount { name, count })
        .collect();
    list.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    list
}

#[cfg(test)]
mod tests {
    // AGENT CODE: claude-opus-5
    use std::fs;

    use jiff::Zoned;
    use yaydl_shared::{AudioCodec, VideoQuality};

    use super::*;

    const MP3: OutputFormat = OutputFormat::Audio {
        codec: AudioCodec::Mp3,
    };
    const FLAC: OutputFormat = OutputFormat::Audio {
        codec: AudioCodec::Flac,
    };
    const P720: OutputFormat = OutputFormat::Video {
        quality: VideoQuality::P720,
    };

    fn entry(id: DownloadId, finished_at_ms: i64) -> HistoryEntry {
        HistoryEntry {
            download_id: id,
            extractor: "Youtube".into(),
            video_id: format!("vid{id}"),
            title: format!("Title {id}"),
            uploader: None,
            url: format!("https://www.youtube.com/watch?v=vid{id}"),
            format: MP3,
            file_path: PathBuf::from(format!("/music/{id}.mp3")),
            file_size_bytes: Some(1000),
            duration_secs: Some(60.0),
            finished_at_ms,
        }
    }

    fn berlin(local: &str) -> Zoned {
        format!("{local}[Europe/Berlin]").parse().unwrap()
    }

    fn ms(local: &str) -> i64 {
        berlin(local).timestamp().as_millisecond()
    }

    fn now() -> Zoned {
        berlin("2026-09-23T10:00")
    }

    fn stats_at(local_times: &[&str], granularity: StatsGranularity) -> Statistics {
        let entries: Vec<_> = local_times
            .iter()
            .zip(1..)
            .map(|(t, id)| entry(id, ms(t)))
            .collect();
        compute_statistics(&entries, granularity, &now())
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let history = History::load(dir.path().join(HISTORY_FILE)).unwrap();
        assert!(history.entries().is_empty());
    }

    #[test]
    fn record_sorts_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        let mut history = History::load(path.clone()).unwrap();
        history.record(entry(1, 300)).unwrap();
        history.record(entry(2, 100)).unwrap();
        history.record(entry(3, 300)).unwrap();
        let ids: Vec<_> = history.entries().iter().map(|e| e.download_id).collect();
        assert_eq!(ids, vec![2, 1, 3]);
        let newest: Vec<_> = history
            .newest_first()
            .iter()
            .map(|e| e.download_id)
            .collect();
        assert_eq!(newest, vec![3, 1, 2]);

        let raw: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["version"], 1);
        assert_eq!(raw["entries"].as_array().unwrap().len(), 3);

        let reloaded = History::load(path).unwrap();
        assert_eq!(reloaded.entries(), history.entries());
    }

    #[test]
    fn load_sorts_unsorted_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        let file = serde_json::json!({
            "version": 1,
            "entries": [entry(1, 500), entry(2, 100), entry(3, 500)],
        });
        fs::write(&path, file.to_string()).unwrap();
        let ids: Vec<_> = History::load(path)
            .unwrap()
            .entries()
            .iter()
            .map(|e| e.download_id)
            .collect();
        assert_eq!(ids, vec![2, 1, 3]);
    }

    #[test]
    fn invalid_json_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        fs::write(&path, "{\"version\": 1, \"entries\": [").unwrap();
        assert!(History::load(path.clone()).is_err());
        fs::write(&path, "[]").unwrap();
        assert!(History::load(path.clone()).is_err());
        fs::write(&path, r#"{"version": 1, "entries": [], "extra": true}"#).unwrap();
        assert!(History::load(path).is_err());
    }

    #[test]
    fn unknown_version_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        fs::write(&path, r#"{"version": 2, "entries": [{"shape": "new"}]}"#).unwrap();
        let err = History::load(path.clone()).err().unwrap();
        assert_eq!(err.path, path);
        assert_eq!(err.message, "history version is 2, expected 1");
    }

    #[test]
    fn out_of_range_timestamp_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        let mut history = History::empty(path.clone());
        let err = history.record(entry(7, i64::MAX)).err().unwrap();
        assert!(err.message.contains("history entry 7"), "{}", err.message);
        assert!(history.entries().is_empty());
        assert!(!path.exists());

        let file = serde_json::json!({ "version": 1, "entries": [entry(8, i64::MIN)] });
        fs::write(&path, file.to_string()).unwrap();
        assert!(History::load(path).is_err());
    }

    #[test]
    fn failed_save_leaves_memory_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        fs::write(&blocker, "").unwrap();
        let mut history = History::empty(blocker.join(HISTORY_FILE));
        assert!(history.record(entry(1, 100)).is_err());
        assert!(history.entries().is_empty());

        let path = dir.path().join(HISTORY_FILE);
        let mut history = History::empty(path.clone());
        history.record(entry(1, 100)).unwrap();
        history.path = blocker.join(HISTORY_FILE);
        assert!(history
            .update_file_path(1, PathBuf::from("/music/renamed.mp3"))
            .is_err());
        assert_eq!(
            history.entries()[0].file_path,
            PathBuf::from("/music/1.mp3")
        );
        assert!(history.clear().is_err());
        assert_eq!(history.entries().len(), 1);
    }

    #[test]
    fn update_file_path_hit_and_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        let mut history = History::empty(path.clone());
        history.record(entry(1, 100)).unwrap();
        history.record(entry(2, 200)).unwrap();

        assert!(history
            .update_file_path(2, PathBuf::from("/music/renamed.mp3"))
            .unwrap());
        assert!(!history
            .update_file_path(99, PathBuf::from("/music/nope.mp3"))
            .unwrap());

        let reloaded = History::load(path).unwrap();
        assert_eq!(
            reloaded.entries()[1].file_path,
            PathBuf::from("/music/renamed.mp3")
        );
        assert_eq!(
            reloaded.entries()[0].file_path,
            PathBuf::from("/music/1.mp3")
        );
    }

    #[test]
    fn update_file_path_on_duplicate_id_touches_newest() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::empty(dir.path().join(HISTORY_FILE));
        history.record(entry(1, 100)).unwrap();
        history.record(entry(1, 200)).unwrap();
        history
            .update_file_path(1, PathBuf::from("/music/renamed.mp3"))
            .unwrap();
        assert_eq!(
            history.entries()[0].file_path,
            PathBuf::from("/music/1.mp3")
        );
        assert_eq!(
            history.entries()[1].file_path,
            PathBuf::from("/music/renamed.mp3")
        );
    }

    #[test]
    fn find_previous_picks_most_recent_match() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = History::empty(dir.path().join(HISTORY_FILE));
        let mut older = entry(1, 100);
        older.video_id = "abc".into();
        let mut newer = entry(2, 200);
        newer.video_id = "abc".into();
        newer.extractor = "youtube".into();
        let mut other_format = entry(3, 300);
        other_format.video_id = "abc".into();
        other_format.format = P720;
        history.record(newer).unwrap();
        history.record(older).unwrap();
        history.record(other_format).unwrap();

        let hit = history.find_previous("YouTube", "abc", &MP3).unwrap();
        assert_eq!(hit.download_id, 2);
        assert_eq!(
            history
                .find_previous("YOUTUBE", "abc", &P720)
                .unwrap()
                .download_id,
            3
        );
        assert!(history.find_previous("youtube", "abc", &FLAC).is_none());
        assert!(history.find_previous("vimeo", "abc", &MP3).is_none());
        assert!(history.find_previous("youtube", "ABC", &MP3).is_none());
    }

    #[test]
    fn clear_empties_memory_and_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        let mut history = History::empty(path.clone());
        history.record(entry(1, 100)).unwrap();
        history.clear().unwrap();
        assert!(history.entries().is_empty());
        assert!(path.exists());
        assert!(History::load(path).unwrap().entries().is_empty());
    }

    #[test]
    fn day_buckets() {
        let mut entries = vec![
            entry(1, ms("2026-09-23T00:00")),
            entry(2, ms("2026-09-22T23:59:59.999")),
            entry(3, ms("2026-08-25T00:00")),
            entry(4, ms("2026-08-24T23:59:59.999")),
        ];
        entries[1].file_size_bytes = None;
        let stats = compute_statistics(&entries, StatsGranularity::Day, &now());

        assert_eq!(stats.granularity, StatsGranularity::Day);
        assert_eq!(stats.buckets.len(), 30);
        let first = &stats.buckets[0];
        assert_eq!(
            (first.start_date.as_str(), first.count, first.bytes),
            ("2026-08-25", 1, 1000)
        );
        let yesterday = &stats.buckets[28];
        assert_eq!(yesterday.start_date, "2026-09-22");
        assert_eq!((yesterday.count, yesterday.bytes), (1, 0));
        let today = &stats.buckets[29];
        assert_eq!((today.start_date.as_str(), today.count), ("2026-09-23", 1));
        assert_eq!(stats.buckets[7].start_date, "2026-09-01");
        assert_eq!(stats.buckets.iter().map(|b| b.count).sum::<u32>(), 3);
        assert_eq!(stats.totals.downloads, 4);
        assert_eq!(stats.totals.bytes, 3000);
    }

    #[test]
    fn week_buckets_split_on_iso_monday() {
        let stats = stats_at(
            &["2026-09-20T23:30", "2026-09-21T00:10", "2026-07-05T23:59"],
            StatsGranularity::Week,
        );
        assert_eq!(stats.buckets.len(), 12);
        let first = &stats.buckets[0];
        assert_eq!((first.start_date.as_str(), first.count), ("2026-07-06", 0));
        let previous = &stats.buckets[10];
        assert_eq!(
            (previous.start_date.as_str(), previous.count),
            ("2026-09-14", 1)
        );
        let current = &stats.buckets[11];
        assert_eq!(
            (current.start_date.as_str(), current.count),
            ("2026-09-21", 1)
        );
        assert_eq!(stats.totals.downloads, 3);
    }

    #[test]
    fn week_bucket_starts_on_the_iso_monday_across_new_year() {
        let entries = [entry(1, ms("2027-01-01T12:00"))];
        let stats = compute_statistics(
            &entries,
            StatsGranularity::Week,
            &berlin("2027-01-02T12:00"),
        );
        let current = stats.buckets.last().unwrap();
        assert_eq!(
            (current.start_date.as_str(), current.count),
            ("2026-12-28", 1)
        );
    }

    #[test]
    fn month_buckets() {
        let stats = stats_at(
            &[
                "2026-09-01T00:00",
                "2026-08-31T23:59:59",
                "2025-10-01T00:00",
                "2025-09-30T23:00",
            ],
            StatsGranularity::Month,
        );
        assert_eq!(stats.buckets.len(), 12);
        assert_eq!(stats.buckets[0].start_date, "2025-10-01");
        assert_eq!(stats.buckets[3].start_date, "2026-01-01");
        assert_eq!(stats.buckets[11].start_date, "2026-09-01");
        let counts: Vec<_> = stats.buckets.iter().map(|b| b.count).collect();
        assert_eq!(counts, vec![1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1]);
        assert_eq!(stats.totals.downloads, 4);
    }

    #[test]
    fn day_buckets_follow_calendar_days_across_dst_end() {
        let entries: Vec<_> = [
            "2026-10-24T23:30+02:00",
            "2026-10-25T00:30+02:00",
            "2026-10-25T02:30+02:00",
            "2026-10-25T02:30+01:00",
            "2026-10-25T23:30+01:00",
            "2026-10-26T00:10+01:00",
        ]
        .iter()
        .zip(1..)
        .map(|(t, id)| entry(id, ms(t)))
        .collect();
        let now = berlin("2026-10-26T09:00");
        let stats = compute_statistics(&entries, StatsGranularity::Day, &now);

        let dates: Vec<Date> = stats
            .buckets
            .iter()
            .map(|b| b.start_date.parse().unwrap())
            .collect();
        for pair in dates.windows(2) {
            assert_eq!(pair[0].tomorrow().unwrap(), pair[1]);
        }
        assert_eq!(dates.last().unwrap().to_string(), "2026-10-26");
        let tail: Vec<_> = stats.buckets[27..]
            .iter()
            .map(|b| (b.start_date.as_str(), b.count))
            .collect();
        assert_eq!(
            tail,
            vec![("2026-10-24", 1), ("2026-10-25", 4), ("2026-10-26", 1)]
        );
        assert_eq!(stats.activity[6][2], 2);
        assert_eq!(stats.activity[6][23], 1);
    }

    #[test]
    fn streak_ending_today() {
        let stats = stats_at(
            &[
                "2026-09-01T12:00",
                "2026-09-02T12:00",
                "2026-09-03T12:00",
                "2026-09-04T12:00",
                "2026-09-05T12:00",
                "2026-09-21T12:00",
                "2026-09-22T12:00",
                "2026-09-22T18:00",
                "2026-09-23T08:00",
            ],
            StatsGranularity::Day,
        );
        assert_eq!(stats.current_streak_days, 3);
        assert_eq!(stats.longest_streak_days, 5);
    }

    #[test]
    fn streak_ending_yesterday() {
        let stats = stats_at(
            &["2026-09-20T12:00", "2026-09-21T12:00", "2026-09-22T23:59"],
            StatsGranularity::Day,
        );
        assert_eq!(stats.current_streak_days, 3);
        assert_eq!(stats.longest_streak_days, 3);
    }

    #[test]
    fn streak_broken() {
        let stats = stats_at(
            &["2026-09-20T12:00", "2026-09-21T12:00"],
            StatsGranularity::Day,
        );
        assert_eq!(stats.current_streak_days, 0);
        assert_eq!(stats.longest_streak_days, 2);
    }

    #[test]
    fn busiest_ties_pick_smaller_index_and_latest_day() {
        // Monday 10:15 and Tuesday 09:45.
        let stats = stats_at(
            &["2026-09-14T10:15", "2026-09-15T09:45"],
            StatsGranularity::Day,
        );
        assert_eq!(stats.busiest_weekday, Some(0));
        assert_eq!(stats.busiest_hour, Some(9));
        assert_eq!(
            stats.busiest_day,
            Some(DayCount {
                date: "2026-09-15".into(),
                count: 1
            })
        );
        assert_eq!(stats.activity[0][10], 1);
        assert_eq!(stats.activity[1][9], 1);
    }

    #[test]
    fn busiest_clear_winner() {
        let stats = stats_at(
            &[
                "2026-09-13T20:00",
                "2026-09-13T20:30",
                "2026-09-16T08:00",
                "2026-09-17T20:10",
            ],
            StatsGranularity::Day,
        );
        assert_eq!(stats.busiest_weekday, Some(6));
        assert_eq!(stats.busiest_hour, Some(20));
        assert_eq!(
            stats.busiest_day,
            Some(DayCount {
                date: "2026-09-13".into(),
                count: 2
            })
        );
    }

    #[test]
    fn totals_uploaders_and_formats() {
        let uploaders = [
            Some("Carol"),
            Some("Alice"),
            Some("Bob"),
            Some("Alice"),
            Some("Bob"),
            Some("Dave"),
            Some("Erin"),
            Some("Frank"),
            None,
            None,
        ];
        let formats = [MP3, MP3, FLAC, P720, P720, FLAC, MP3, P720, FLAC, MP3];
        let mut entries: Vec<_> = uploaders
            .iter()
            .zip(formats)
            .zip(1..)
            .map(|((uploader, format), id)| {
                let mut e = entry(id, ms("2026-09-10T12:00") + id as i64 * 1000);
                e.uploader = uploader.map(String::from);
                e.format = format;
                e
            })
            .collect();
        entries[0].duration_secs = None;
        entries[1].duration_secs = Some(30.5);
        let stats = compute_statistics(&entries, StatsGranularity::Day, &now());

        assert_eq!(stats.totals.downloads, 10);
        assert_eq!(stats.totals.bytes, 10_000);
        assert_eq!(stats.totals.duration_secs, 30.5 + 8.0 * 60.0);
        assert_eq!(stats.totals.distinct_uploaders, 6);
        assert_eq!(
            stats.totals.first_download_ms,
            Some(ms("2026-09-10T12:00") + 1000)
        );

        let top: Vec<_> = stats
            .top_uploaders
            .iter()
            .map(|n| (n.name.as_str(), n.count))
            .collect();
        assert_eq!(
            top,
            vec![
                ("Alice", 2),
                ("Bob", 2),
                ("Carol", 1),
                ("Dave", 1),
                ("Erin", 1)
            ]
        );

        let formats: Vec<_> = stats
            .formats
            .iter()
            .map(|n| (n.name.as_str(), n.count))
            .collect();
        assert_eq!(
            formats,
            vec![("MP3 audio", 4), ("FLAC audio", 3), ("MP4 video 720p", 3)]
        );
    }

    #[test]
    fn empty_history() {
        for granularity in StatsGranularity::ALL {
            let stats = compute_statistics(&[], granularity, &now());
            assert_eq!(stats.buckets.len(), granularity.bucket_count());
            assert!(stats.buckets.iter().all(|b| b.count == 0 && b.bytes == 0));
            assert_eq!(stats.totals, StatsTotals::default());
            assert_eq!(stats.activity, vec![[0u32; 24]; 7]);
            assert_eq!(stats.busiest_weekday, None);
            assert_eq!(stats.busiest_hour, None);
            assert_eq!(stats.busiest_day, None);
            assert_eq!(stats.current_streak_days, 0);
            assert_eq!(stats.longest_streak_days, 0);
            assert!(stats.top_uploaders.is_empty());
            assert!(stats.formats.is_empty());
        }
    }
}
