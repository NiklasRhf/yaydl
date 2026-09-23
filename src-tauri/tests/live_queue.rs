// AGENT CODE: claude-opus-5-5
//! End to end through the real queue and the real yt-dlp against YouTube. Ignored by
//! default, run with
//! `YAYDL_LIVE_YT_DLP=<yt-dlp binary> YAYDL_LIVE_FFMPEG_DIR=<dir with ffmpeg, ffprobe>
//!  cargo test -p yaydl --test live_queue -- --ignored --nocapture`.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use yaydl_lib::{
    history::{compute_statistics, History},
    queue::{Queue, QueueDeps, Sink},
    settings::{default_settings, SettingsStore},
    ytdlp::{ProcessRunner, YtDlp},
};
use yaydl_shared::{
    DownloadItem, DownloadStatus, Notice, OutputFormat, StatsGranularity, VideoQuality,
};

const ZOO: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
const PLAYLIST: &str = "https://www.youtube.com/playlist?list=PLbpi6ZahtOH6Blw3RGYpWkSByi_T7Rygb";

#[derive(Clone, Default)]
struct Recorder {
    notices: Arc<Mutex<Vec<Notice>>>,
    batches: Arc<Mutex<Vec<(u32, u32)>>>,
}

impl Sink for Recorder {
    fn queue_replaced(&self, _items: &[DownloadItem]) {}
    fn item_updated(&self, _item: &DownloadItem) {}
    fn history_changed(&self) {}
    fn notice(&self, notice: Notice) {
        println!("notice: {notice:?}");
        self.notices.lock().unwrap().push(notice);
    }
    fn batch_finished(&self, finished: u32, failed: u32) {
        self.batches.lock().unwrap().push((finished, failed));
    }
}

fn required_env(name: &str) -> PathBuf {
    PathBuf::from(std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set")))
}

async fn wait_until<F: Fn(&[DownloadItem]) -> bool>(
    queue: &Queue<YtDlp<ProcessRunner>, Recorder>,
    what: &str,
    timeout: Duration,
    done: F,
) -> Vec<DownloadItem> {
    let start = Instant::now();
    loop {
        let items = queue.get_queue();
        if done(&items) {
            return items;
        }
        if start.elapsed() > timeout {
            panic!("timed out waiting for {what}: {items:#?}");
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs network and the real yt-dlp"]
async fn queue_downloads_playlist_entries_in_parallel_and_tracks_history() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("out");
    std::fs::create_dir_all(&out).unwrap();

    let ytdlp = YtDlp::new(
        ProcessRunner,
        dir.path().join("ytdlp"),
        required_env("YAYDL_LIVE_YT_DLP"),
        required_env("YAYDL_LIVE_FFMPEG_DIR"),
    );
    ytdlp.ensure_installed().unwrap();
    let settings = Arc::new(SettingsStore::new(
        dir.path().join("settings.toml"),
        default_settings(out.clone()),
    ));
    let history = Arc::new(Mutex::new(
        History::load(dir.path().join("history.json")).unwrap(),
    ));
    let sink = Recorder::default();
    let queue = Queue::restore(QueueDeps {
        engine: Arc::new(ytdlp),
        sink: sink.clone(),
        settings,
        history: Arc::clone(&history),
        path: dir.path().join("queue.json"),
        runtime: tokio::runtime::Handle::current(),
    })
    .unwrap();

    let added = queue.add_urls(&format!("{ZOO}\n{PLAYLIST} not-a-link"));
    assert_eq!(added.added.len(), 2, "{added:?}");
    assert_eq!(added.invalid, vec!["not-a-link".to_string()]);

    let items = wait_until(&queue, "metadata", Duration::from_secs(90), |items| {
        items
            .iter()
            .all(|i| !matches!(i.status, DownloadStatus::ResolvingMetadata))
    })
    .await;
    assert!(items.len() > 2, "the playlist was not expanded: {items:#?}");
    assert!(items.iter().all(|i| i.status == DownloadStatus::Ready));

    let zoo = items
        .iter()
        .find(|i| i.metadata.as_ref().unwrap().video_id == "jNQXAC9IVRw")
        .unwrap()
        .id;
    let other = items.iter().find(|i| i.id != zoo).unwrap().id;
    for item in &items {
        if item.id != zoo && item.id != other {
            queue.remove_item(item.id).unwrap();
        }
    }
    queue
        .set_item_format(
            other,
            OutputFormat::Video {
                quality: VideoQuality::P480,
            },
        )
        .unwrap();
    assert_eq!(queue.start_all(), 2);

    let items = wait_until(
        &queue,
        "both downloads",
        Duration::from_secs(300),
        |items| {
            items.iter().all(|i| {
                matches!(
                    i.status,
                    DownloadStatus::Finished | DownloadStatus::Failed { .. }
                )
            })
        },
    )
    .await;
    for item in &items {
        assert_eq!(item.status, DownloadStatus::Finished, "{item:#?}");
        let path = item.file_path.as_ref().unwrap();
        println!("finished {} -> {}", item.id, path.display());
        assert!(path.exists());
    }
    assert_eq!(*sink.batches.lock().unwrap(), vec![(2, 0)]);

    queue.rename_item(zoo, "renamed zoo").unwrap();
    let renamed = queue
        .get_queue()
        .into_iter()
        .find(|i| i.id == zoo)
        .unwrap()
        .file_path
        .unwrap();
    assert!(renamed.ends_with("renamed zoo.mp3"), "{renamed:?}");
    assert!(renamed.exists());

    {
        let history = history.lock().unwrap();
        assert_eq!(history.entries().len(), 2);
        assert!(history.entries().iter().any(|e| e.file_path == renamed));
        let stats = compute_statistics(
            history.entries(),
            StatsGranularity::Day,
            &jiff::Zoned::now(),
        );
        assert_eq!(stats.totals.downloads, 2);
        assert_eq!(stats.buckets.last().unwrap().count, 2);
        println!("stats totals: {:?}", stats.totals);
    }

    assert_eq!(queue.clear_finished(), 2);
    queue.add_urls(ZOO);
    let items = wait_until(
        &queue,
        "re-added metadata",
        Duration::from_secs(90),
        |items| items.iter().all(|i| i.status == DownloadStatus::Ready),
    )
    .await;
    let again = &items[0];
    assert_eq!(
        again.previous_download.as_ref().unwrap().file_path,
        renamed,
        "the duplicate warning should point at the renamed file"
    );
    queue.start_download(again.id).unwrap();
    assert_eq!(
        queue.get_queue()[0].status,
        DownloadStatus::AwaitingDuplicateConfirmation
    );
    queue.confirm_duplicate(again.id, false).unwrap();
    assert_eq!(queue.get_queue()[0].status, DownloadStatus::Ready);
}
