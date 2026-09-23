// AGENT CODE: claude-opus-5
use std::{
    collections::HashMap,
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::oneshot;
use yaydl_shared::{
    AudioCodec, DownloadItem, DownloadStatus, ErrorKind, FriendlyError, HistoryEntry, Language,
    Notice, NoticeLevel, OutputFormat, Progress, VideoMetadata, VideoQuality, YaydlError,
};

use super::*;
use crate::{history::History, settings::default_settings};

type Outcome = std::result::Result<DownloadOutcome, YtDlpFailure>;

struct Pending {
    request: DownloadRequest,
    finish: oneshot::Sender<Outcome>,
    on_event: Box<dyn FnMut(DownloadEvent) + Send>,
}

#[derive(Default)]
struct FakeEngine {
    resolves: Mutex<HashMap<String, std::result::Result<Resolved, YtDlpFailure>>>,
    playlists: Mutex<HashMap<String, std::result::Result<Resolved, YtDlpFailure>>>,
    /// Playlist resolves by URL that wait until the test sends on the gate.
    playlist_gates: Mutex<HashMap<String, oneshot::Receiver<()>>>,
    /// Running downloads by URL, completed by the test through `finish`.
    pending: Mutex<HashMap<String, Pending>>,
    started: Mutex<Vec<String>>,
    panics: Mutex<Vec<String>>,
}

impl FakeEngine {
    fn script(&self, url: &str, result: std::result::Result<Resolved, YtDlpFailure>) {
        self.resolves
            .lock()
            .unwrap()
            .insert(url.to_string(), result);
    }

    fn script_playlist(&self, url: &str, result: std::result::Result<Resolved, YtDlpFailure>) {
        self.playlists
            .lock()
            .unwrap()
            .insert(url.to_string(), result);
    }

    fn gate_playlist(&self, url: &str) -> oneshot::Sender<()> {
        let (open, gate) = oneshot::channel();
        self.playlist_gates
            .lock()
            .unwrap()
            .insert(url.to_string(), gate);
        open
    }

    fn running(&self) -> Vec<String> {
        let mut urls: Vec<String> = self.pending.lock().unwrap().keys().cloned().collect();
        urls.sort();
        urls
    }

    fn started(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }

    fn event(&self, url: &str, event: DownloadEvent) {
        let mut pending = self.pending.lock().unwrap();
        let p = pending
            .get_mut(url)
            .unwrap_or_else(|| panic!("{url} is not downloading"));
        (p.on_event)(event);
    }

    fn finish(&self, url: &str, outcome: Outcome) {
        let p = self
            .pending
            .lock()
            .unwrap()
            .remove(url)
            .unwrap_or_else(|| panic!("{url} is not downloading"));
        p.finish.send(outcome).expect("the download task to wait");
    }

    fn request(&self, url: &str) -> DownloadRequest {
        self.pending.lock().unwrap()[url].request.clone()
    }
}

impl Engine for FakeEngine {
    async fn resolve(
        &self,
        url: &str,
        _run: &RunOptions,
    ) -> std::result::Result<Resolved, YtDlpFailure> {
        self.resolves
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or_else(|| {
                Err(YtDlpFailure::Failed(FriendlyError {
                    kind: ErrorKind::Other,
                    message: format!("unscripted resolve of {url}"),
                    detail: String::new(),
                }))
            })
    }

    async fn resolve_playlist(
        &self,
        url: &str,
        _run: &RunOptions,
    ) -> std::result::Result<Resolved, YtDlpFailure> {
        let gate = self.playlist_gates.lock().unwrap().remove(url);
        if let Some(gate) = gate {
            gate.await.expect("the test to open the playlist gate");
        }
        self.playlists
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .unwrap_or_else(|| {
                Err(YtDlpFailure::Failed(FriendlyError {
                    kind: ErrorKind::Other,
                    message: format!("unscripted playlist resolve of {url}"),
                    detail: String::new(),
                }))
            })
    }

    async fn download(
        &self,
        request: &DownloadRequest,
        cancel: CancellationToken,
        on_event: Box<dyn FnMut(DownloadEvent) + Send>,
    ) -> Outcome {
        self.started.lock().unwrap().push(request.url.clone());
        if self.panics.lock().unwrap().contains(&request.url) {
            panic!("scripted engine panic for {}", request.url);
        }
        let (finish, done) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            request.url.clone(),
            Pending {
                request: request.clone(),
                finish,
                on_event,
            },
        );
        tokio::select! {
            outcome = done => outcome.expect("the test to finish or cancel every download"),
            _ = cancel.cancelled() => {
                self.pending.lock().unwrap().remove(&request.url);
                Err(YtDlpFailure::Cancelled)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Replaced(Vec<DownloadId>),
    Updated(DownloadId, String),
    HistoryChanged,
    Notice(Notice),
    BatchFinished(u32, u32),
}

#[derive(Clone, Default)]
struct RecordingSink(Arc<Mutex<Vec<Event>>>);

impl RecordingSink {
    fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }

    fn notices(&self) -> Vec<Notice> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Notice(n) => Some(n),
                _ => None,
            })
            .collect()
    }

    fn batches(&self) -> Vec<(u32, u32)> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::BatchFinished(f, x) => Some((f, x)),
                _ => None,
            })
            .collect()
    }

    fn count(&self, wanted: &Event) -> usize {
        self.events().iter().filter(|e| *e == wanted).count()
    }
}

impl Sink for RecordingSink {
    fn queue_replaced(&self, items: &[DownloadItem]) {
        self.0
            .lock()
            .unwrap()
            .push(Event::Replaced(items.iter().map(|i| i.id).collect()));
    }

    fn item_updated(&self, item: &DownloadItem) {
        self.0
            .lock()
            .unwrap()
            .push(Event::Updated(item.id, item.status.name().to_string()));
    }

    fn history_changed(&self) {
        self.0.lock().unwrap().push(Event::HistoryChanged);
    }

    fn notice(&self, notice: Notice) {
        self.0.lock().unwrap().push(Event::Notice(notice));
    }

    fn batch_finished(&self, finished: u32, failed: u32) {
        self.0
            .lock()
            .unwrap()
            .push(Event::BatchFinished(finished, failed));
    }
}

struct Harness {
    queue: Queue<FakeEngine, RecordingSink>,
    engine: Arc<FakeEngine>,
    sink: RecordingSink,
    settings: Arc<SettingsStore>,
    history: Arc<Mutex<History>>,
    dir: tempfile::TempDir,
}

impl Harness {
    fn new() -> Self {
        Self::with(|_| {})
    }

    /// `prepare` runs before the queue is restored, to seed files.
    fn with(prepare: impl FnOnce(&Path)) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        fs::create_dir_all(&out).unwrap();
        prepare(dir.path());
        // Pinned so notice texts do not depend on the machine's locale.
        let settings = Arc::new(SettingsStore::new(
            dir.path().join("settings.toml"),
            Settings {
                language: Language::English,
                ..default_settings(out)
            },
        ));
        let history_path = dir.path().join("history.json");
        let history = Arc::new(Mutex::new(History::load(history_path).unwrap()));
        let engine = Arc::new(FakeEngine::default());
        let sink = RecordingSink::default();
        let queue = Queue::restore(QueueDeps {
            engine: Arc::clone(&engine),
            sink: sink.clone(),
            settings: Arc::clone(&settings),
            history: Arc::clone(&history),
            path: dir.path().join(QUEUE_FILE),
            runtime: Handle::current(),
        })
        .unwrap();
        Self {
            queue,
            engine,
            sink,
            settings,
            history,
            dir,
        }
    }

    fn out(&self) -> PathBuf {
        self.dir.path().join("out")
    }

    fn item(&self, id: DownloadId) -> DownloadItem {
        self.queue
            .get_queue()
            .into_iter()
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("no item {id}"))
    }

    fn status(&self, id: DownloadId) -> String {
        self.item(id).status.name().to_string()
    }

    fn persisted(&self) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(self.dir.path().join(QUEUE_FILE)).unwrap())
            .unwrap()
    }

    async fn until(&self, what: &str, cond: impl Fn(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !cond(self) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}, queue: {:#?}",
                self.queue.get_queue()
            );
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }

    async fn until_status(&self, id: DownloadId, status: &str) {
        self.until(&format!("item {id} to be {status}"), |h| {
            h.status(id) == status
        })
        .await;
    }

    /// Adds `url` scripted to resolve to a single video and waits for `Ready`.
    async fn ready(&self, url: &str) -> DownloadId {
        self.engine
            .script(url, Ok(Resolved::Single(meta(url, &video_id(url)))));
        let added = self.queue.add_urls(url).added;
        assert_eq!(added.len(), 1, "{url} was not added");
        self.until_status(added[0], "ready").await;
        added[0]
    }

    fn record_history(&self, entry: HistoryEntry) {
        self.history.lock().unwrap().record(entry).unwrap();
    }
}

fn video_id(url: &str) -> String {
    url.rsplit('/').next().unwrap().to_string()
}

fn meta(url: &str, id: &str) -> VideoMetadata {
    VideoMetadata {
        extractor: "Youtube".to_string(),
        video_id: id.to_string(),
        title: format!("Title {id}"),
        uploader: Some("Uploader".to_string()),
        duration_secs: Some(60.0),
        thumbnail: None,
        webpage_url: url.to_string(),
    }
}

fn mp3() -> OutputFormat {
    OutputFormat::Audio {
        codec: AudioCodec::Mp3,
    }
}

fn history_entry(download_id: DownloadId, id: &str, file: PathBuf) -> HistoryEntry {
    HistoryEntry {
        download_id,
        extractor: "youtube".to_string(),
        video_id: id.to_string(),
        title: format!("Title {id}"),
        uploader: None,
        url: format!("https://yt.test/{id}"),
        format: mp3(),
        file_path: file,
        file_size_bytes: Some(3),
        duration_secs: None,
        finished_at_ms: 1_750_000_000_000,
    }
}

fn failure(message: &str) -> YtDlpFailure {
    YtDlpFailure::Failed(FriendlyError {
        kind: ErrorKind::Network,
        message: message.to_string(),
        detail: String::new(),
    })
}

fn finished_at(path: &Path) -> Outcome {
    Ok(DownloadOutcome {
        file_path: path.to_path_buf(),
    })
}

#[tokio::test]
async fn add_urls_tokenizes_rejects_and_dedupes() {
    let h = Harness::new();

    let first = h.queue.add_urls(
        "https://yt.test/a\n  http://yt.test/b\tyt.test/c ftp://yt.test/d https:// https://yt.test/a",
    );

    assert_eq!(first.added.len(), 2);
    assert_eq!(
        first.invalid,
        vec!["yt.test/c", "ftp://yt.test/d", "https://"]
    );
    assert_eq!(first.already_queued, vec!["https://yt.test/a"]);

    let second = h.queue.add_urls("HTTPS://YT.TEST/a https://yt.test/e");
    assert_eq!(second.already_queued, vec!["https://yt.test/a"]);
    assert_eq!(second.added.len(), 1);

    let ids: Vec<DownloadId> = first.added.iter().chain(&second.added).copied().collect();
    assert!(ids.windows(2).all(|w| w[0] < w[1]), "ids ascend: {ids:?}");
    let queue = h.queue.get_queue();
    assert_eq!(
        queue.iter().map(|i| i.id).collect::<Vec<_>>(),
        ids.iter().rev().copied().collect::<Vec<_>>(),
        "newest first"
    );
    assert!(queue.iter().all(|i| i.format == mp3()));
    assert_eq!(h.persisted()["items"].as_array().unwrap().len(), 3);
}

#[tokio::test]
async fn single_resolve_reaches_ready_and_flags_an_existing_previous_download() {
    let h = Harness::new();
    let kept = h.out().join("kept.mp3");
    fs::write(&kept, "abc").unwrap();
    h.record_history(history_entry(90, "kept", kept.clone()));
    h.record_history(history_entry(91, "gone", h.out().join("gone.mp3")));

    let kept_id = h.ready("https://yt.test/kept").await;
    let gone_id = h.ready("https://yt.test/gone").await;
    let fresh_id = h.ready("https://yt.test/fresh").await;

    let kept_item = h.item(kept_id);
    assert_eq!(kept_item.metadata.as_ref().unwrap().video_id, "kept");
    assert_eq!(kept_item.previous_download.map(|p| p.file_path), Some(kept));
    assert_eq!(h.item(gone_id).previous_download, None);
    assert_eq!(h.item(fresh_id).previous_download, None);
}

#[tokio::test]
async fn format_change_recomputes_previous_download() {
    let h = Harness::new();
    let kept = h.out().join("kept.mp3");
    fs::write(&kept, "abc").unwrap();
    h.record_history(history_entry(1, "kept", kept));
    let id = h.ready("https://yt.test/kept").await;
    assert!(h.item(id).previous_download.is_some());

    let video = OutputFormat::Video {
        quality: VideoQuality::P720,
    };
    h.queue.set_item_format(id, video).unwrap();

    assert_eq!(h.item(id).format, video);
    assert_eq!(h.item(id).previous_download, None);
}

#[tokio::test]
async fn resolve_failure_goes_to_metadata_failed_and_retry_resolves_again() {
    let h = Harness::new();
    let url = "https://yt.test/x";
    h.engine.script(url, Err(failure("offline")));
    let id = h.queue.add_urls(url).added[0];
    h.until_status(id, "missing metadata").await;

    h.engine.script(url, Ok(Resolved::Single(meta(url, "x"))));
    h.queue.retry_download(id).unwrap();
    h.until_status(id, "ready").await;
}

#[tokio::test]
async fn playlist_expands_in_place_skipping_queued_urls() {
    let h = Harness::new();
    let before = h.ready("https://yt.test/before").await;
    let queued = h.ready("https://yt.test/p2").await;
    let list = "https://yt.test/list";
    h.engine.script(
        list,
        Ok(Resolved::Playlist {
            title: Some("Mix".to_string()),
            entries: vec![
                meta("https://yt.test/p1", "p1"),
                meta("https://yt.test/p2", "p2"),
                meta("https://yt.test/p3", "p3"),
            ],
            unavailable: 2,
        }),
    );
    let placeholder = h.queue.add_urls(list).added[0];
    let after = h.ready("https://yt.test/after").await;
    h.queue
        .set_item_format(
            after,
            OutputFormat::Video {
                quality: VideoQuality::Best,
            },
        )
        .unwrap();

    h.until("the playlist to expand", |h| {
        h.queue.get_queue().iter().all(|i| i.id != placeholder)
    })
    .await;

    let queue = h.queue.get_queue();
    let urls: Vec<&str> = queue.iter().map(|i| i.url.as_str()).collect();
    assert_eq!(
        urls,
        vec![
            "https://yt.test/after",
            "https://yt.test/p3",
            "https://yt.test/p1",
            "https://yt.test/p2",
            "https://yt.test/before",
        ]
    );
    let p1 = queue.iter().find(|i| i.url.ends_with("/p1")).unwrap();
    let p3 = queue.iter().find(|i| i.url.ends_with("/p3")).unwrap();
    assert!(p1.id > after && p3.id > p1.id, "entries get new ids");
    assert!(p1.status == DownloadStatus::Ready && p1.metadata.is_some());
    assert_eq!(p1.format, mp3());
    assert!(queue.iter().any(|i| i.id == queued) && queue.iter().any(|i| i.id == before));
    assert_eq!(
        h.sink.notices(),
        vec![Notice {
            level: NoticeLevel::Info,
            text: "Skipped 2 unavailable videos from Mix.".to_string()
        }]
    );
}

#[tokio::test]
async fn a_language_change_applies_to_the_next_notice() {
    let h = Harness::new();
    let mut settings = h.settings.get();
    settings.language = Language::German;
    h.settings.update(settings).unwrap();
    let list = "https://yt.test/list";
    h.engine.script(
        list,
        Ok(Resolved::Playlist {
            title: Some("Mix".to_string()),
            entries: vec![meta("https://yt.test/p1", "p1")],
            unavailable: 1,
        }),
    );

    h.queue.add_urls(list);
    h.until("the playlist notice", |h| !h.sink.notices().is_empty())
        .await;

    assert_eq!(
        h.sink.notices()[0].text,
        "1 nicht verfügbares Video aus Mix übersprungen."
    );
}

#[tokio::test]
async fn playlist_without_new_entries_fails_the_placeholder() {
    let h = Harness::new();
    h.ready("https://yt.test/only").await;
    let list = "https://yt.test/list";
    h.engine.script(
        list,
        Ok(Resolved::Playlist {
            title: None,
            entries: vec![meta("https://yt.test/only", "only")],
            unavailable: 1,
        }),
    );
    let id = h.queue.add_urls(list).added[0];
    h.until_status(id, "missing metadata").await;

    let DownloadStatus::MetadataFailed { error } = h.item(id).status else {
        unreachable!()
    };
    assert_eq!(error.kind, ErrorKind::EmptyPlaylist);
    assert_eq!(error.message, "This playlist has no downloadable videos.");
    assert_eq!(
        h.sink.notices()[0].text,
        "Skipped 1 unavailable video from the playlist."
    );
}

#[tokio::test]
async fn duplicate_start_waits_for_confirmation() {
    let h = Harness::new();
    let old = h.out().join("dup.mp3");
    fs::write(&old, "abc").unwrap();
    h.record_history(history_entry(1, "dup", old));
    let id = h.ready("https://yt.test/dup").await;

    h.queue.start_download(id).unwrap();
    assert_eq!(h.status(id), "awaiting confirmation");
    assert!(h.engine.started().is_empty());

    h.queue.confirm_duplicate(id, false).unwrap();
    assert_eq!(h.status(id), "ready");

    h.queue.start_download(id).unwrap();
    h.queue.confirm_duplicate(id, true).unwrap();
    h.until_status(id, "downloading").await;
    h.until("the engine to start", |h| h.engine.running().len() == 1)
        .await;
    assert!(matches!(
        h.queue.confirm_duplicate(id, true),
        Err(YaydlError::InvalidState { .. })
    ));
    h.engine.finish(
        "https://yt.test/dup",
        finished_at(&h.out().join("dup2.mp3")),
    );
    h.until_status(id, "finished").await;
}

#[tokio::test]
async fn at_most_two_downloads_run_and_the_next_starts_when_one_finishes() {
    let h = Harness::new();
    let mut ids = Vec::new();
    for name in ["c1", "c2", "c3", "c4"] {
        ids.push(h.ready(&format!("https://yt.test/{name}")).await);
    }

    assert_eq!(h.queue.start_all(), 4);
    h.until("two downloads to run", |h| h.engine.running().len() == 2)
        .await;
    assert_eq!(
        h.engine.started(),
        vec!["https://yt.test/c1", "https://yt.test/c2"],
        "lowest id first"
    );
    assert_eq!(h.status(ids[2]), "queued");
    assert_eq!(h.status(ids[3]), "queued");

    let file = h.out().join("c1.mp3");
    fs::write(&file, "12345").unwrap();
    h.engine.finish("https://yt.test/c1", finished_at(&file));
    h.until("c3 to start", |h| {
        h.engine.running() == vec!["https://yt.test/c2", "https://yt.test/c3"]
    })
    .await;
    assert_eq!(h.status(ids[0]), "finished");
    assert_eq!(h.status(ids[3]), "queued");

    for url in ["https://yt.test/c2", "https://yt.test/c3"] {
        h.engine.finish(url, finished_at(&h.out().join("x.mp3")));
    }
    h.until("c4 to start", |h| {
        h.engine.running() == vec!["https://yt.test/c4"]
    })
    .await;
    h.engine.finish("https://yt.test/c4", Err(failure("boom")));
    h.until_status(ids[3], "failed").await;
    h.until("the batch to finish", |h| !h.sink.batches().is_empty())
        .await;
    assert_eq!(h.sink.batches(), vec![(3, 1)]);

    let entries = h.history.lock().unwrap().newest_first();
    assert_eq!(entries.len(), 3);
    let c1 = entries.iter().find(|e| e.download_id == ids[0]).unwrap();
    assert_eq!(c1.file_size_bytes, Some(5));
    assert_eq!(c1.video_id, "c1");
    assert_eq!(c1.file_path, file);
    assert_eq!(h.sink.count(&Event::HistoryChanged), 3);
}

#[tokio::test]
async fn raising_the_limit_starts_more_downloads() {
    let h = Harness::new();
    for name in ["r1", "r2", "r3"] {
        h.ready(&format!("https://yt.test/{name}")).await;
    }
    h.queue.start_all();
    h.until("two downloads to run", |h| h.engine.running().len() == 2)
        .await;

    let mut settings = h.settings.get();
    settings.max_concurrent_downloads = 3;
    h.settings.update(settings).unwrap();
    h.queue.schedule();

    h.until("three downloads to run", |h| h.engine.running().len() == 3)
        .await;
}

#[tokio::test]
async fn progress_and_processing_events_update_the_item() {
    let h = Harness::new();
    let url = "https://yt.test/p";
    let id = h.ready(url).await;
    h.queue.rename_item(id, "My Name").unwrap();
    h.queue.start_download(id).unwrap();
    h.until("the engine to start", |h| h.engine.running().len() == 1)
        .await;

    let request = h.engine.request(url);
    assert_eq!(request.file_stem.as_deref(), Some("My Name"));
    assert_eq!(request.output_dir, h.out());
    assert!(request.embed_metadata);

    let progress = Progress {
        percent: Some(42.0),
        ..Progress::default()
    };
    h.engine
        .event(url, DownloadEvent::Progress(progress.clone()));
    assert_eq!(h.item(id).status, DownloadStatus::Downloading { progress });
    assert_eq!(
        h.persisted()["items"][0]["status"]["progress"]["percent"],
        serde_json::Value::Null,
        "progress ticks are not persisted"
    );

    h.engine.event(url, DownloadEvent::Processing);
    assert_eq!(h.status(id), "processing");
    assert_eq!(h.persisted()["items"][0]["status"]["state"], "processing");

    h.engine
        .finish(url, finished_at(&h.out().join("My Name.mp3")));
    h.until_status(id, "finished").await;
}

#[tokio::test]
async fn cancelling_running_queued_and_awaiting_items() {
    let h = Harness::new();
    let mut settings = h.settings.get();
    settings.max_concurrent_downloads = 1;
    h.settings.update(settings).unwrap();

    let running = h.ready("https://yt.test/run").await;
    let queued = h.ready("https://yt.test/wait").await;
    let old = h.out().join("dup.mp3");
    fs::write(&old, "abc").unwrap();
    h.record_history(history_entry(1, "dup", old));
    let awaiting = h.ready("https://yt.test/dup").await;

    h.queue.start_all();
    h.until("one download to run", |h| h.engine.running().len() == 1)
        .await;
    assert_eq!(h.status(queued), "queued");
    assert_eq!(h.status(awaiting), "awaiting confirmation");

    h.queue.cancel_download(queued).unwrap();
    assert_eq!(h.status(queued), "cancelled");

    h.queue.cancel_download(awaiting).unwrap();
    assert_eq!(h.status(awaiting), "ready");

    h.queue.cancel_download(running).unwrap();
    h.until_status(running, "cancelled").await;
    assert!(h.engine.running().is_empty());

    assert!(matches!(
        h.queue.cancel_download(running),
        Err(YaydlError::InvalidState { .. })
    ));
    assert!(
        h.sink.batches().is_empty(),
        "cancelled downloads are neither finished nor failed"
    );
    assert!(h.engine.started() == vec!["https://yt.test/run"]);
}

#[tokio::test]
async fn failure_and_retry_without_duplicate_check() {
    let h = Harness::new();
    let url = "https://yt.test/f";
    let id = h.ready(url).await;
    h.queue.start_download(id).unwrap();
    h.until("the engine to start", |h| h.engine.running().len() == 1)
        .await;
    h.engine.finish(url, Err(failure("network down")));
    h.until_status(id, "failed").await;
    h.until("the batch to finish", |h| h.sink.batches() == vec![(0, 1)])
        .await;

    let old = h.out().join("f.mp3");
    fs::write(&old, "abc").unwrap();
    h.record_history(history_entry(1, "f", old));
    h.queue.retry_download(id).unwrap();
    h.until("the retry to start", |h| h.engine.running().len() == 1)
        .await;
    assert_eq!(h.status(id), "downloading");
    h.engine.finish(url, finished_at(&h.out().join("f2.mp3")));
    h.until("the second batch", |h| h.sink.batches().len() == 2)
        .await;
    assert_eq!(h.sink.batches(), vec![(0, 1), (1, 0)]);
}

#[tokio::test]
async fn an_engine_panic_fails_the_item_and_frees_the_slot() {
    let h = Harness::new();
    let mut settings = h.settings.get();
    settings.max_concurrent_downloads = 1;
    h.settings.update(settings).unwrap();
    let boom = h.ready("https://yt.test/boom").await;
    let next = h.ready("https://yt.test/next").await;
    h.engine
        .panics
        .lock()
        .unwrap()
        .push("https://yt.test/boom".to_string());

    h.queue.start_all();

    h.until_status(boom, "failed").await;
    let DownloadStatus::Failed { error } = h.item(boom).status else {
        unreachable!()
    };
    assert_eq!(error.kind, ErrorKind::Other);
    assert!(error.detail.contains("panic"), "{}", error.detail);
    h.until("the next download to take the slot", |h| {
        h.engine.running() == vec!["https://yt.test/next"]
    })
    .await;
    assert_eq!(h.status(next), "downloading");
}

#[tokio::test]
async fn missing_output_dir_fails_without_calling_the_engine() {
    let h = Harness::new();
    let id = h.ready("https://yt.test/m").await;
    fs::remove_dir(h.out()).unwrap();

    h.queue.start_download(id).unwrap();
    h.until_status(id, "failed").await;

    let DownloadStatus::Failed { error } = h.item(id).status else {
        unreachable!()
    };
    assert_eq!(error.kind, ErrorKind::OutputFolderMissing);
    assert_eq!(error.detail, h.out().display().to_string());
    assert!(
        error.message.contains("does not exist"),
        "{}",
        error.message
    );
    assert!(h.engine.started().is_empty());
}

#[tokio::test]
async fn rename_before_download_sets_the_custom_name() {
    let h = Harness::new();
    let id = h.ready("https://yt.test/n").await;

    h.queue.rename_item(id, "Nice name").unwrap();
    assert_eq!(h.item(id).custom_name.as_deref(), Some("Nice name"));

    assert!(matches!(
        h.queue.rename_item(id, "bad/name"),
        Err(YaydlError::InvalidFileName(_))
    ));
    assert_eq!(h.item(id).custom_name.as_deref(), Some("Nice name"));
}

#[tokio::test]
async fn rename_after_finish_moves_the_file_and_updates_history() {
    let h = Harness::new();
    let url = "https://yt.test/done";
    let id = h.ready(url).await;
    h.queue.start_download(id).unwrap();
    h.until("the engine to start", |h| h.engine.running().len() == 1)
        .await;
    let file = h.out().join("Title done [done].mp3");
    fs::write(&file, "abc").unwrap();
    h.engine.finish(url, finished_at(&file));
    h.until_status(id, "finished").await;

    let taken = h.out().join("Taken.mp3");
    fs::write(&taken, "other").unwrap();
    let err = h.queue.rename_item(id, "Taken").unwrap_err();
    assert!(
        matches!(&err, YaydlError::Io(m) if m.contains("already exists")),
        "{err:?}"
    );
    assert!(file.exists());

    h.queue.rename_item(id, "Renamed").unwrap();

    let renamed = h.out().join("Renamed.mp3");
    assert!(renamed.exists() && !file.exists());
    let item = h.item(id);
    assert_eq!(item.file_path.as_deref(), Some(renamed.as_path()));
    assert_eq!(item.custom_name.as_deref(), Some("Renamed"));
    let history = h.history.lock().unwrap().newest_first();
    assert_eq!(history[0].file_path, renamed);
    assert_eq!(h.sink.count(&Event::HistoryChanged), 2);
    assert_eq!(h.queue.finished_file(id).unwrap(), renamed);

    fs::remove_file(&renamed).unwrap();
    let err = h.queue.rename_item(id, "Again").unwrap_err();
    assert!(
        matches!(&err, YaydlError::Io(m) if m.contains("no longer exists")),
        "{err:?}"
    );
    assert!(matches!(
        h.queue.finished_file(id),
        Err(YaydlError::Open(_))
    ));
}

#[tokio::test]
async fn active_items_cannot_be_removed_and_clear_all_keeps_them() {
    let h = Harness::new();
    let url = "https://yt.test/active";
    let active = h.ready(url).await;
    let idle = h.ready("https://yt.test/idle").await;
    h.queue.start_download(active).unwrap();
    h.until("the engine to start", |h| h.engine.running().len() == 1)
        .await;

    assert!(matches!(
        h.queue.remove_item(active),
        Err(YaydlError::InvalidState { .. })
    ));
    assert!(matches!(
        h.queue.rename_item(active, "x"),
        Err(YaydlError::InvalidState { .. })
    ));
    assert!(matches!(
        h.queue.set_item_format(active, mp3()),
        Err(YaydlError::InvalidState { .. })
    ));
    assert!(matches!(
        h.queue.remove_item(999),
        Err(YaydlError::UnknownDownload(999))
    ));

    assert_eq!(h.queue.clear_all(), 1);
    let queue = h.queue.get_queue();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].id, active);
    assert!(queue.iter().all(|i| i.id != idle));

    h.engine
        .finish(url, finished_at(&h.out().join("active.mp3")));
    h.until_status(active, "finished").await;
    assert_eq!(h.queue.clear_finished(), 1);
    assert!(h.queue.get_queue().is_empty());
}

#[tokio::test]
async fn removing_an_item_while_it_resolves_drops_the_result() {
    let h = Harness::new();
    let url = "https://yt.test/late";
    h.engine
        .script(url, Ok(Resolved::Single(meta(url, "late"))));
    let id = h.queue.add_urls(url).added[0];
    h.queue.remove_item(id).unwrap();

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(h.queue.get_queue().is_empty());
}

const LIST_LINK: &str = "https://www.youtube.com/watch?v=e1&list=PLtest";
const MIX_LINK: &str = "https://www.youtube.com/watch?v=e1&list=RDe1";

fn watch(id: &str) -> String {
    format!("https://www.youtube.com/watch?v={id}")
}

fn playlist(title: Option<&str>, ids: &[&str], unavailable: u32) -> Resolved {
    Resolved::Playlist {
        title: title.map(str::to_string),
        entries: ids.iter().map(|id| meta(&watch(id), id)).collect(),
        unavailable,
    }
}

impl Harness {
    async fn until_notice(&self) {
        self.until("a notice", |h| !h.sink.notices().is_empty())
            .await;
    }
}

#[tokio::test]
async fn a_video_link_expands_into_its_playlist_in_place() {
    let h = Harness::new();
    let before = h.ready("https://yt.test/before").await;
    let link = h.ready(LIST_LINK).await;
    let video = OutputFormat::Video {
        quality: VideoQuality::P720,
    };
    h.queue.set_item_format(link, video).unwrap();
    let after = h.ready("https://yt.test/after").await;
    h.engine.script_playlist(
        LIST_LINK,
        Ok(playlist(Some("Road Trip"), &["e1", "e2", "e3"], 0)),
    );

    h.queue.expand_playlist(link).unwrap();
    assert_eq!(
        h.sink
            .count(&Event::Updated(link, "resolving metadata".to_string())),
        1
    );
    h.until("the link to be replaced", |h| {
        h.queue.get_queue().iter().all(|i| i.id != link)
    })
    .await;

    let queue = h.queue.get_queue();
    let urls: Vec<&str> = queue.iter().map(|i| i.url.as_str()).collect();
    assert_eq!(
        urls,
        vec![
            "https://yt.test/after".to_string(),
            watch("e3"),
            watch("e2"),
            watch("e1"),
            "https://yt.test/before".to_string(),
        ]
    );
    let entries: Vec<&DownloadItem> = queue.iter().filter(|i| i.url.contains("watch")).collect();
    assert!(entries.iter().all(|i| i.id > after), "entries get new ids");
    assert!(entries
        .iter()
        .all(|i| i.format == video && i.status == DownloadStatus::Ready && i.metadata.is_some()));
    assert!(queue.iter().any(|i| i.id == before));
    assert_eq!(
        h.sink.notices(),
        vec![Notice {
            level: NoticeLevel::Success,
            text: "Added 3 videos from Road Trip.".to_string()
        }]
    );
    let persisted = h.persisted();
    let persisted_urls: Vec<&str> = persisted["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["url"].as_str().unwrap())
        .collect();
    assert!(!persisted_urls.contains(&LIST_LINK));
    assert_eq!(persisted_urls.len(), 5);
}

#[tokio::test]
async fn expanding_a_mix_skips_queued_entries_and_reports_unavailable_ones() {
    let h = Harness::new();
    let queued = h.ready(&watch("e2")).await;
    let link = h.ready(MIX_LINK).await;
    h.engine
        .script_playlist(MIX_LINK, Ok(playlist(None, &["e1", "e2", "e3"], 1)));

    h.queue.expand_playlist(link).unwrap();
    h.until("the link to be replaced", |h| {
        h.queue.get_queue().iter().all(|i| i.id != link)
    })
    .await;

    let urls: Vec<String> = h.queue.get_queue().into_iter().map(|i| i.url).collect();
    assert_eq!(urls, vec![watch("e3"), watch("e1"), watch("e2")]);
    assert_eq!(h.item(queued).url, watch("e2"), "the queued entry is kept");
    assert_eq!(
        h.sink.notices(),
        vec![
            Notice {
                level: NoticeLevel::Info,
                text: "Skipped 1 unavailable video from the playlist.".to_string()
            },
            Notice {
                level: NoticeLevel::Success,
                text: "Added 2 videos from the mix.".to_string()
            },
        ]
    );
}

#[tokio::test]
async fn a_playlist_already_in_the_queue_restores_the_item() {
    let h = Harness::new();
    h.ready(&watch("e1")).await;
    h.ready(&watch("e2")).await;
    let link = h.ready(LIST_LINK).await;
    let snapshot = h.item(link);
    h.engine
        .script_playlist(LIST_LINK, Ok(playlist(Some("Road Trip"), &["e1", "e2"], 0)));

    h.queue.expand_playlist(link).unwrap();
    h.until_notice().await;

    assert_eq!(h.item(link), snapshot);
    assert_eq!(h.queue.get_queue().len(), 3);
    assert_eq!(
        h.sink.notices(),
        vec![Notice {
            level: NoticeLevel::Info,
            text: "Every video of this playlist is already in the queue.".to_string()
        }]
    );
    assert_eq!(h.persisted()["items"][2]["status"]["state"], "ready");
}

#[tokio::test]
async fn an_empty_playlist_restores_the_item() {
    let h = Harness::new();
    let link = h.ready(MIX_LINK).await;
    let snapshot = h.item(link);
    h.engine
        .script_playlist(MIX_LINK, Ok(playlist(None, &[], 0)));

    h.queue.expand_playlist(link).unwrap();
    h.until_notice().await;

    assert_eq!(h.item(link), snapshot);
    assert_eq!(
        h.sink.notices(),
        vec![Notice {
            level: NoticeLevel::Info,
            text: "This mix has no downloadable videos.".to_string()
        }]
    );
}

#[tokio::test]
async fn a_single_video_result_restores_status_metadata_and_previous_download() {
    let h = Harness::new();
    let kept = h.out().join("kept.mp3");
    fs::write(&kept, "abc").unwrap();
    h.record_history(history_entry(90, &video_id(LIST_LINK), kept));
    let link = h.ready(LIST_LINK).await;
    let snapshot = h.item(link);
    assert!(snapshot.previous_download.is_some());
    h.engine.script_playlist(
        LIST_LINK,
        Ok(Resolved::Single(meta(&watch("other"), "other"))),
    );

    h.queue.expand_playlist(link).unwrap();
    h.until_notice().await;

    assert_eq!(h.item(link), snapshot);
    assert_eq!(
        h.sink.notices(),
        vec![Notice {
            level: NoticeLevel::Error,
            text: "YouTube returned a single video instead of the playlist.".to_string()
        }]
    );
}

#[tokio::test]
async fn a_failed_playlist_load_restores_a_cancelled_item() {
    let h = Harness::with(|dir| {
        let items = vec![DownloadItem {
            url: LIST_LINK.to_string(),
            ..item(1, DownloadStatus::Cancelled)
        }];
        let file = serde_json::json!({ "version": 1, "next_id": 2, "items": items });
        fs::write(dir.join(QUEUE_FILE), file.to_string()).unwrap();
    });
    let snapshot = h.item(1);
    h.engine
        .script_playlist(LIST_LINK, Err(failure("No internet connection.")));

    h.queue.expand_playlist(1).unwrap();
    h.until_notice().await;

    assert_eq!(h.item(1), snapshot);
    assert_eq!(h.persisted()["items"][0]["status"]["state"], "cancelled");
    assert_eq!(
        h.sink.notices(),
        vec![Notice {
            level: NoticeLevel::Error,
            text: "Loading the playlist failed: No internet connection.".to_string()
        }]
    );
}

#[tokio::test]
async fn expanding_rejects_other_states_and_links_without_a_playlist() {
    let h = Harness::with(|dir| {
        let items = vec![DownloadItem {
            url: LIST_LINK.to_string(),
            file_path: Some(dir.join("done.mp3")),
            ..item(1, DownloadStatus::Finished)
        }];
        let file = serde_json::json!({ "version": 1, "next_id": 2, "items": items });
        fs::write(dir.join(QUEUE_FILE), file.to_string()).unwrap();
    });
    let plain = h.ready("https://www.youtube.com/watch?v=plain").await;

    let finished = h.queue.expand_playlist(1).unwrap_err();
    assert!(
        matches!(&finished, YaydlError::InvalidState { action, state } if action == "add the playlist of" && state == "finished"),
        "{finished:?}"
    );
    let no_list = h.queue.expand_playlist(plain).unwrap_err();
    assert!(
        matches!(&no_list, YaydlError::InvalidState { state, .. } if state == "a link without a playlist"),
        "{no_list:?}"
    );
    assert!(matches!(
        h.queue.expand_playlist(999),
        Err(YaydlError::UnknownDownload(999))
    ));
    assert_eq!(h.status(1), "finished");
    assert_eq!(h.status(plain), "ready");
    assert!(h.sink.notices().is_empty());
}

#[tokio::test]
async fn removing_an_item_while_its_playlist_loads_drops_the_result() {
    let h = Harness::new();
    let link = h.ready(LIST_LINK).await;
    h.engine
        .script_playlist(LIST_LINK, Ok(playlist(Some("Road Trip"), &["e1", "e2"], 0)));
    let open = h.engine.gate_playlist(LIST_LINK);

    h.queue.expand_playlist(link).unwrap();
    assert!(matches!(
        h.queue.expand_playlist(link),
        Err(YaydlError::InvalidState { .. })
    ));
    h.queue.remove_item(link).unwrap();
    open.send(()).unwrap();

    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(h.queue.get_queue().is_empty());
    assert!(h.sink.notices().is_empty());
    assert_eq!(h.persisted()["items"], serde_json::json!([]));
}

#[tokio::test]
async fn the_expansion_notice_follows_the_language() {
    let h = Harness::new();
    let mut settings = h.settings.get();
    settings.language = Language::German;
    h.settings.update(settings).unwrap();
    let link = h.ready(LIST_LINK).await;
    h.engine
        .script_playlist(LIST_LINK, Ok(playlist(Some("Road Trip"), &["e1"], 0)));

    h.queue.expand_playlist(link).unwrap();
    h.until_notice().await;

    assert_eq!(
        h.sink.notices()[0].text,
        "1 Video aus Road Trip hinzugefügt."
    );
}

fn item(id: DownloadId, status: DownloadStatus) -> DownloadItem {
    let url = format!("https://yt.test/{id}");
    DownloadItem {
        id,
        url: url.clone(),
        added_at_ms: 1_750_000_000_000,
        metadata: Some(meta(&url, &id.to_string())),
        format: mp3(),
        custom_name: None,
        status,
        previous_download: None,
        file_path: None,
    }
}

#[tokio::test]
async fn restore_maps_interrupted_states_and_seeds_ids_from_history() {
    let h = Harness::with(|dir| {
        let items = vec![
            item(
                1,
                DownloadStatus::Downloading {
                    progress: Progress::default(),
                },
            ),
            item(2, DownloadStatus::AwaitingDuplicateConfirmation),
            item(3, DownloadStatus::Queued),
            item(4, DownloadStatus::Processing),
            item(5, DownloadStatus::Ready),
            DownloadItem {
                metadata: None,
                ..item(6, DownloadStatus::ResolvingMetadata)
            },
        ];
        let file = serde_json::json!({ "version": 1, "next_id": 7, "items": items });
        fs::write(dir.join(QUEUE_FILE), file.to_string()).unwrap();
        let history = serde_json::json!({
            "version": 1,
            "entries": [history_entry(41, "old", dir.join("old.mp3"))],
        });
        fs::write(dir.join("history.json"), history.to_string()).unwrap();
    });

    for id in [1, 3, 4] {
        let DownloadStatus::Failed { error } = h.item(id).status else {
            panic!("item {id} should be failed, is {}", h.status(id));
        };
        assert_eq!(error.kind, ErrorKind::Interrupted);
        assert_eq!(error.message, INTERRUPTED_MESSAGE);
    }
    assert_eq!(h.status(2), "ready");
    assert_eq!(h.status(5), "ready");
    assert_eq!(h.persisted()["items"][0]["status"]["state"], "failed");
    h.until_status(6, "missing metadata").await;

    let added = h.queue.add_urls("https://yt.test/new").added;
    assert_eq!(added, vec![42]);
}

#[tokio::test]
async fn an_invalid_queue_file_is_quarantined_with_a_notice() {
    let h = Harness::with(|dir| {
        fs::write(
            dir.join(QUEUE_FILE),
            r#"{"version":9,"next_id":1,"items":[]}"#,
        )
        .unwrap();
    });

    assert!(h.queue.get_queue().is_empty());
    let notices = h.sink.notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].level, NoticeLevel::Warning);
    assert!(
        notices[0].text.contains("version is 9"),
        "{}",
        notices[0].text
    );
    let quarantined = fs::read_dir(h.dir.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("queue.json.invalid-")
        })
        .count();
    assert_eq!(quarantined, 1);
}

#[test]
fn url_normalization() {
    assert_eq!(
        normalize_url("HTTPS://Example.COM/watch?v=1").as_deref(),
        Some("https://example.com/watch?v=1")
    );
    assert_eq!(normalize_url("mailto:a@b.c"), None);
    assert_eq!(normalize_url("file:///etc/passwd"), None);
    assert_eq!(normalize_url("not a url"), None);
}
