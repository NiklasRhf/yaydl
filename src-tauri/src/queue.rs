use std::{
    collections::{HashMap, HashSet},
    fs,
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize};
use tokio::runtime::Handle;
use tracing::{debug, error, info, warn};
use yaydl_shared::{
    validate_file_stem, AddUrlsResult, DownloadId, DownloadItem, DownloadStatus, ErrorKind,
    FriendlyError, HistoryEntry, Notice, NoticeLevel, OutputFormat, PreviousDownload, Progress,
    Settings, VideoMetadata, YaydlError,
};

use crate::{
    history::History,
    notices::notice,
    persist::{quarantine, read_json, write_json_atomic, PersistError},
    settings::SettingsStore,
    ytdlp::{
        CancellationToken, DownloadEvent, DownloadOutcome, DownloadRequest, Resolved, RunOptions,
        Runner, YtDlp, YtDlpFailure,
    },
};

pub const QUEUE_FILE: &str = "queue.json";
const QUEUE_VERSION: u64 = 1;

pub const INTERRUPTED_MESSAGE: &str =
    "yaydl was closed during this download. Retry to start it again.";
const EMPTY_PLAYLIST_MESSAGE: &str = "This playlist has no downloadable videos.";

type Result<T> = std::result::Result<T, YaydlError>;

/// What the queue needs from yt-dlp, so the queue logic runs in tests without
/// spawning processes.
pub trait Engine: Send + Sync + 'static {
    fn resolve(
        &self,
        url: &str,
        run: &RunOptions,
    ) -> impl Future<Output = std::result::Result<Resolved, YtDlpFailure>> + Send;

    fn download(
        &self,
        request: &DownloadRequest,
        cancel: CancellationToken,
        on_event: Box<dyn FnMut(DownloadEvent) + Send>,
    ) -> impl Future<Output = std::result::Result<DownloadOutcome, YtDlpFailure>> + Send;
}

impl<R: Runner> Engine for YtDlp<R> {
    async fn resolve(
        &self,
        url: &str,
        run: &RunOptions,
    ) -> std::result::Result<Resolved, YtDlpFailure> {
        YtDlp::resolve(self, url, run).await
    }

    async fn download(
        &self,
        request: &DownloadRequest,
        cancel: CancellationToken,
        on_event: Box<dyn FnMut(DownloadEvent) + Send>,
    ) -> std::result::Result<DownloadOutcome, YtDlpFailure> {
        YtDlp::download(self, request, cancel, on_event).await
    }
}

/// Where queue changes go. The app emits Tauri events, tests record calls.
/// Called while the queue lock is held so events arrive in the order the
/// changes happened, except `batch_finished`, which is called without it.
pub trait Sink: Send + Sync + 'static {
    /// The whole queue, newest first.
    fn queue_replaced(&self, items: &[DownloadItem]);
    fn item_updated(&self, item: &DownloadItem);
    fn history_changed(&self);
    fn notice(&self, notice: Notice);
    fn batch_finished(&self, finished: u32, failed: u32);
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueueFile {
    #[serde(rename = "version")]
    _version: u64,
    next_id: DownloadId,
    items: Vec<DownloadItem>,
}

#[derive(Serialize)]
struct QueueFileRef<'a> {
    version: u64,
    next_id: DownloadId,
    items: &'a [DownloadItem],
}

#[derive(Default)]
struct BatchCounts {
    finished: u32,
    failed: u32,
}

struct State {
    next_id: DownloadId,
    /// Oldest first. Playlist entries take the position of their placeholder,
    /// so this is not strictly sorted by id.
    items: Vec<DownloadItem>,
    /// One per running download task. Its length is the number of used slots.
    tokens: HashMap<DownloadId, CancellationToken>,
    batch: BatchCounts,
}

impl State {
    fn position(&self, id: DownloadId) -> Result<usize> {
        self.items
            .iter()
            .position(|i| i.id == id)
            .ok_or(YaydlError::UnknownDownload(id))
    }

    fn item_mut(&mut self, id: DownloadId) -> Result<&mut DownloadItem> {
        let index = self.position(id)?;
        Ok(&mut self.items[index])
    }

    fn newest_first(&self) -> Vec<DownloadItem> {
        self.items.iter().rev().cloned().collect()
    }

    fn allocate_id(&mut self) -> DownloadId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

struct Inner<E, S> {
    engine: Arc<E>,
    sink: S,
    settings: Arc<SettingsStore>,
    history: Arc<Mutex<History>>,
    path: PathBuf,
    runtime: Handle,
    state: Mutex<State>,
}

/// The download queue. Cheap to clone, every clone shares the same state.
///
/// Lock order is queue state, then history, then settings. No lock is held
/// across an `.await`.
pub struct Queue<E, S> {
    inner: Arc<Inner<E, S>>,
}

impl<E, S> Clone for Queue<E, S> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

pub struct QueueDeps<E, S> {
    pub engine: Arc<E>,
    pub sink: S,
    pub settings: Arc<SettingsStore>,
    pub history: Arc<Mutex<History>>,
    /// `queue.json`.
    pub path: PathBuf,
    /// Tasks are spawned here rather than with `tokio::spawn`, so the queue can
    /// be restored from Tauri's `setup`, which runs outside a runtime context.
    pub runtime: Handle,
}

impl<E: Engine, S: Sink> Queue<E, S> {
    /// Loads the persisted queue, maps states that cannot survive a restart,
    /// and resumes metadata resolution. An invalid file is quarantined and the
    /// queue starts empty with a warning notice.
    pub fn restore(deps: QueueDeps<E, S>) -> Result<Self> {
        let loaded = match load_file(&deps.path) {
            Ok(loaded) => loaded,
            Err(e) => {
                warn!(error = %e, "the queue file is invalid");
                let moved = quarantine(&deps.path)?;
                deps.sink.notice(notice(
                    NoticeLevel::Warning,
                    format!(
                        "The saved download queue was invalid ({}) and was moved to {}. The queue starts empty.",
                        e.message,
                        moved.display()
                    ),
                ));
                None
            }
        };
        let (persisted_next_id, mut items) = match loaded {
            Some(file) => (file.next_id, file.items),
            None => (1, Vec::new()),
        };

        let history_next = {
            let history = deps.history.lock().expect("history lock poisoned");
            history
                .entries()
                .iter()
                .map(|e| e.download_id + 1)
                .max()
                .unwrap_or(1)
        };
        let next_id = persisted_next_id.max(history_next).max(1);
        info!(
            persisted_next_id,
            history_next,
            next_id,
            items = items.len(),
            "restoring the queue"
        );

        let mut to_resolve = Vec::new();
        for item in &mut items {
            let restored = match &item.status {
                DownloadStatus::ResolvingMetadata => {
                    to_resolve.push(item.id);
                    None
                }
                DownloadStatus::Queued
                | DownloadStatus::Downloading { .. }
                | DownloadStatus::Processing => Some(DownloadStatus::Failed {
                    error: FriendlyError {
                        kind: ErrorKind::Interrupted,
                        message: INTERRUPTED_MESSAGE.to_string(),
                        detail: String::new(),
                    },
                }),
                DownloadStatus::AwaitingDuplicateConfirmation => Some(DownloadStatus::Ready),
                _ => None,
            };
            if let Some(status) = restored {
                info!(
                    id = item.id,
                    from = item.status.name(),
                    to = status.name(),
                    "restored item state"
                );
                item.status = status;
            }
        }

        let queue = Self {
            inner: Arc::new(Inner {
                engine: deps.engine,
                sink: deps.sink,
                settings: deps.settings,
                history: deps.history,
                path: deps.path,
                runtime: deps.runtime,
                state: Mutex::new(State {
                    next_id,
                    items,
                    tokens: HashMap::new(),
                    batch: BatchCounts::default(),
                }),
            }),
        };
        {
            let mut state = queue.inner.lock();
            let inner = &queue.inner;
            for item in &mut state.items {
                if item.status.can_edit_before_download() {
                    item.previous_download = inner.previous_download(item);
                }
            }
            inner.persist(&state);
        }
        for id in to_resolve {
            queue.spawn_resolve(id);
        }
        Ok(queue)
    }

    pub fn get_queue(&self) -> Vec<DownloadItem> {
        self.inner.lock().newest_first()
    }

    pub fn add_urls(&self, text: &str) -> AddUrlsResult {
        let settings = self.inner.settings.get();
        let mut result = AddUrlsResult::default();
        {
            let mut state = self.inner.lock();
            let mut seen: HashSet<String> = state.items.iter().map(|i| i.url.clone()).collect();
            let now = now_ms();
            for token in text.split_whitespace() {
                let Some(url) = normalize_url(token) else {
                    result.invalid.push(token.to_string());
                    continue;
                };
                if !seen.insert(url.clone()) {
                    result.already_queued.push(url);
                    continue;
                }
                let id = state.allocate_id();
                state.items.push(DownloadItem {
                    id,
                    url,
                    added_at_ms: now,
                    metadata: None,
                    format: settings.default_format,
                    custom_name: None,
                    status: DownloadStatus::ResolvingMetadata,
                    previous_download: None,
                    file_path: None,
                });
                result.added.push(id);
            }
            info!(?result, "add_urls");
            if !result.added.is_empty() {
                self.inner.sink.queue_replaced(&state.newest_first());
                self.inner.persist(&state);
            }
        }
        for id in &result.added {
            self.spawn_resolve(*id);
        }
        result
    }

    pub fn start_download(&self, id: DownloadId) -> Result<()> {
        {
            let mut state = self.inner.lock();
            self.inner.begin(&mut state, id)?;
            let index = state.position(id)?;
            self.inner.sink.item_updated(&state.items[index]);
            self.inner.persist(&state);
        }
        self.schedule();
        Ok(())
    }

    pub fn start_all(&self) -> u32 {
        let mut started = 0;
        {
            let mut state = self.inner.lock();
            let ready: Vec<DownloadId> = state
                .items
                .iter()
                .filter(|i| i.status.can_start())
                .map(|i| i.id)
                .collect();
            for id in ready {
                match self.inner.begin(&mut state, id) {
                    Ok(()) => started += 1,
                    Err(e) => {
                        error!(id, error = %e, "start_all could not start an item");
                        self.inner.sink.notice(notice(
                            NoticeLevel::Error,
                            format!("Starting download {id} failed: {e}"),
                        ));
                    }
                }
            }
            if started > 0 {
                self.inner.sink.queue_replaced(&state.newest_first());
                self.inner.persist(&state);
            }
        }
        info!(started, "start_all");
        self.schedule();
        started
    }

    pub fn confirm_duplicate(&self, id: DownloadId, proceed: bool) -> Result<()> {
        self.inner.update_item(id, |item| {
            if item.status != DownloadStatus::AwaitingDuplicateConfirmation {
                return Err(invalid_state("confirm", &item.status));
            }
            item.status = if proceed {
                DownloadStatus::Queued
            } else {
                DownloadStatus::Ready
            };
            info!(id, proceed, "confirm_duplicate");
            Ok(())
        })?;
        self.schedule();
        Ok(())
    }

    pub fn retry_download(&self, id: DownloadId) -> Result<()> {
        let mut resolve = false;
        self.inner.update_item(id, |item| {
            match item.status {
                DownloadStatus::Failed { .. } | DownloadStatus::Cancelled => {
                    item.status = DownloadStatus::Queued;
                }
                DownloadStatus::MetadataFailed { .. } => {
                    item.status = DownloadStatus::ResolvingMetadata;
                    resolve = true;
                }
                _ => return Err(invalid_state("retry", &item.status)),
            }
            info!(id, to = item.status.name(), "retry_download");
            Ok(())
        })?;
        if resolve {
            self.spawn_resolve(id);
        }
        self.schedule();
        Ok(())
    }

    pub fn cancel_download(&self, id: DownloadId) -> Result<()> {
        {
            let mut state = self.inner.lock();
            let index = state.position(id)?;
            match state.items[index].status {
                DownloadStatus::Queued => {
                    state.items[index].status = DownloadStatus::Cancelled;
                }
                DownloadStatus::AwaitingDuplicateConfirmation => {
                    state.items[index].status = DownloadStatus::Ready;
                }
                DownloadStatus::Downloading { .. } | DownloadStatus::Processing => {
                    let Some(token) = state.tokens.get(&id) else {
                        error!(
                            id,
                            status = state.items[index].status.name(),
                            "a running download has no cancellation token"
                        );
                        return Err(YaydlError::InvalidState {
                            action: "cancel".to_string(),
                            state: format!(
                                "{} without a running task",
                                state.items[index].status.name()
                            ),
                        });
                    };
                    info!(id, "cancelling a running download");
                    token.cancel();
                    // The task sets `Cancelled` once the engine has stopped.
                    return Ok(());
                }
                ref other => return Err(invalid_state("cancel", other)),
            }
            info!(id, to = state.items[index].status.name(), "cancel_download");
            self.inner.sink.item_updated(&state.items[index]);
            self.inner.persist(&state);
        }
        self.schedule();
        Ok(())
    }

    pub fn remove_item(&self, id: DownloadId) -> Result<()> {
        let mut state = self.inner.lock();
        let index = state.position(id)?;
        if state.items[index].status.is_active() {
            return Err(invalid_state("remove", &state.items[index].status));
        }
        let removed = state.items.remove(index);
        info!(id, status = removed.status.name(), "remove_item");
        self.inner.sink.queue_replaced(&state.newest_first());
        self.inner.persist(&state);
        Ok(())
    }

    pub fn clear_finished(&self) -> u32 {
        let (removed, _) = self
            .inner
            .remove_where(|i| i.status == DownloadStatus::Finished);
        info!(removed, "clear_finished");
        removed
    }

    /// Returns how many active items were kept.
    pub fn clear_all(&self) -> u32 {
        let (removed, kept) = self.inner.remove_where(|i| !i.status.is_active());
        info!(removed, kept, "clear_all");
        kept
    }

    pub fn set_item_format(&self, id: DownloadId, format: OutputFormat) -> Result<()> {
        let inner = &self.inner;
        inner.update_item(id, |item| {
            if !item.status.can_edit_before_download() {
                return Err(invalid_state("change the format of", &item.status));
            }
            info!(id, from = %item.format, to = %format, "set_item_format");
            item.format = format;
            item.previous_download = inner.previous_download(item);
            Ok(())
        })
    }

    pub fn rename_item(&self, id: DownloadId, name: &str) -> Result<()> {
        validate_file_stem(name).map_err(YaydlError::InvalidFileName)?;
        let mut state = self.inner.lock();
        let index = state.position(id)?;
        let item = &mut state.items[index];
        if item.status.can_edit_before_download()
            || item.status == DownloadStatus::AwaitingDuplicateConfirmation
        {
            info!(id, name, "rename_item before download");
            item.custom_name = Some(name.to_string());
        } else if item.status == DownloadStatus::Finished {
            let new_path = self.inner.rename_on_disk(item, name)?;
            let mut history = self.inner.history.lock().expect("history lock poisoned");
            match history.update_file_path(id, new_path.clone()) {
                Ok(true) => self.inner.sink.history_changed(),
                Ok(false) => debug!(id, "renamed a download that is not in the history"),
                Err(e) => {
                    error!(id, error = %e, new_path = %new_path.display(), "updating the history after a rename failed");
                    drop(history);
                    self.inner.sink.item_updated(&state.items[index]);
                    self.inner.persist(&state);
                    return Err(e.into());
                }
            }
        } else {
            return Err(invalid_state("rename", &item.status));
        }
        self.inner.sink.item_updated(&state.items[index]);
        self.inner.persist(&state);
        Ok(())
    }

    /// The file of a finished download, for opening or revealing it.
    pub fn finished_file(&self, id: DownloadId) -> Result<PathBuf> {
        let state = self.inner.lock();
        let item = &state.items[state.position(id)?];
        if item.status != DownloadStatus::Finished {
            return Err(YaydlError::Open(format!(
                "download {id} is {}, only finished downloads have a file",
                item.status.name()
            )));
        }
        let Some(path) = item.file_path.clone() else {
            error!(id, "a finished download has no file path");
            return Err(YaydlError::Open(format!(
                "download {id} is finished but has no file"
            )));
        };
        if !file_exists(&path) {
            return Err(YaydlError::Open(format!(
                "{} no longer exists",
                path.display()
            )));
        }
        Ok(path)
    }

    /// Starts queued downloads up to the concurrency limit, and reports the end
    /// of a batch once nothing is running or queued anymore.
    pub fn schedule(&self) {
        let settings = self.inner.settings.get();
        let max = usize::from(settings.max_concurrent_downloads);
        let mut spawn = Vec::new();
        let batch = {
            let mut state = self.inner.lock();
            while state.tokens.len() < max {
                let Some(index) = state
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.status == DownloadStatus::Queued)
                    .min_by_key(|(_, i)| i.id)
                    .map(|(index, _)| index)
                else {
                    break;
                };
                let item = &mut state.items[index];
                item.status = DownloadStatus::Downloading {
                    progress: Progress::default(),
                };
                let request = download_request(item, &settings);
                let id = item.id;
                let token = CancellationToken::new();
                info!(id, url = %request.url, format = %request.format, "starting download");
                self.inner.sink.item_updated(&state.items[index]);
                state.tokens.insert(id, token.clone());
                spawn.push((id, request, token));
            }
            if !spawn.is_empty() {
                self.inner.persist(&state);
            }
            let idle = state.tokens.is_empty()
                && !state
                    .items
                    .iter()
                    .any(|i| i.status == DownloadStatus::Queued);
            if idle && (state.batch.finished > 0 || state.batch.failed > 0) {
                Some(std::mem::take(&mut state.batch))
            } else {
                None
            }
        };
        for (id, request, token) in spawn {
            let queue = self.clone();
            self.inner.runtime.spawn(async move {
                queue.run_download(id, request, token).await;
            });
        }
        if let Some(batch) = batch {
            info!(
                finished = batch.finished,
                failed = batch.failed,
                "batch finished"
            );
            self.inner.sink.batch_finished(batch.finished, batch.failed);
        }
    }

    fn spawn_resolve(&self, id: DownloadId) {
        let queue = self.clone();
        self.inner.runtime.spawn(async move {
            queue.run_resolve(id).await;
        });
    }

    async fn run_resolve(&self, id: DownloadId) {
        let url = {
            let state = self.inner.lock();
            match state.items.iter().find(|i| i.id == id) {
                Some(item) if item.status == DownloadStatus::ResolvingMetadata => item.url.clone(),
                Some(item) => {
                    warn!(
                        id,
                        expected = "resolving metadata",
                        actual = item.status.name(),
                        "not resolving an item in an unexpected state"
                    );
                    return;
                }
                None => {
                    debug!(id, "item was removed before resolving started");
                    return;
                }
            }
        };
        let run = run_options(&self.inner.settings.get());
        info!(id, %url, "resolving metadata");
        let engine = Arc::clone(&self.inner.engine);
        let result = self
            .guarded(
                id,
                "resolve",
                async move { engine.resolve(&url, &run).await },
            )
            .await;
        self.inner.apply_resolved(id, result);
    }

    async fn run_download(
        &self,
        id: DownloadId,
        request: DownloadRequest,
        token: CancellationToken,
    ) {
        let result = if dir_exists(&request.output_dir) {
            let queue = self.clone();
            let on_event: Box<dyn FnMut(DownloadEvent) + Send> =
                Box::new(move |event: DownloadEvent| queue.inner.on_download_event(id, event));
            let engine = Arc::clone(&self.inner.engine);
            let owned = request.clone();
            self.guarded(id, "download", async move {
                engine.download(&owned, token, on_event).await
            })
            .await
        } else {
            warn!(id, output_dir = %request.output_dir.display(), "the output folder does not exist, not starting the download");
            Err(YtDlpFailure::Failed(FriendlyError {
                kind: ErrorKind::Other,
                message: format!(
                    "The output folder {} does not exist. Choose another folder in Settings.",
                    request.output_dir.display()
                ),
                detail: String::new(),
            }))
        };
        self.inner.finish_download(id, &request, result);
        self.schedule();
    }

    /// Runs engine work in its own task, so a panic in it becomes a visible
    /// failure of this item instead of leaving it stuck in a running state
    /// with its concurrency slot taken forever.
    async fn guarded<T: Send + 'static>(
        &self,
        id: DownloadId,
        what: &'static str,
        work: impl Future<Output = std::result::Result<T, YtDlpFailure>> + Send + 'static,
    ) -> std::result::Result<T, YtDlpFailure> {
        match self.inner.runtime.spawn(work).await {
            Ok(result) => result,
            Err(e) => {
                error!(id, what, error = %e, "the engine task panicked or was aborted");
                Err(YtDlpFailure::Failed(FriendlyError {
                    kind: ErrorKind::Other,
                    message: "yaydl hit an internal error. Retry, and if it happens again, send the log from Settings."
                        .to_string(),
                    detail: format!("engine {what} task failed: {e}"),
                }))
            }
        }
    }
}

impl<E: Engine, S: Sink> Inner<E, S> {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("queue lock poisoned")
    }

    /// Failing to save does not undo the change, which already happened in
    /// memory and is what the user sees. The notice tells them it will not
    /// survive a restart.
    fn persist(&self, state: &State) {
        let file = QueueFileRef {
            version: QUEUE_VERSION,
            next_id: state.next_id,
            items: &state.items,
        };
        if let Err(e) = write_json_atomic(&self.path, &file) {
            error!(error = %e, "saving the queue failed");
            self.sink.notice(notice(
                NoticeLevel::Error,
                format!(
                    "Saving the download queue failed, changes will be lost when yaydl closes: {e}"
                ),
            ));
        }
    }

    fn update_item(
        &self,
        id: DownloadId,
        change: impl FnOnce(&mut DownloadItem) -> Result<()>,
    ) -> Result<()> {
        let mut state = self.lock();
        let item = state.item_mut(id)?;
        change(item)?;
        self.sink.item_updated(item);
        self.persist(&state);
        Ok(())
    }

    /// Returns how many items were removed and how many active items remain.
    fn remove_where(&self, remove: impl Fn(&DownloadItem) -> bool) -> (u32, u32) {
        let mut state = self.lock();
        let before = state.items.len();
        state.items.retain(|i| !remove(i));
        let removed = before - state.items.len();
        if removed > 0 {
            self.sink.queue_replaced(&state.newest_first());
            self.persist(&state);
        }
        let active = state.items.iter().filter(|i| i.status.is_active()).count();
        (count_u32(removed), count_u32(active))
    }

    /// Moves a `Ready` item to `Queued`, or to `AwaitingDuplicateConfirmation`
    /// when an earlier download of it still exists on disk.
    fn begin(&self, state: &mut State, id: DownloadId) -> Result<()> {
        let item = state.item_mut(id)?;
        if !item.status.can_start() {
            return Err(invalid_state("start", &item.status));
        }
        if item.metadata.is_none() {
            error!(
                id,
                status = item.status.name(),
                "a startable item has no metadata"
            );
            return Err(YaydlError::InvalidState {
                action: "start".to_string(),
                state: format!("{} without metadata", item.status.name()),
            });
        }
        item.previous_download = self.previous_download(item);
        item.status = if item.previous_download.is_some() {
            DownloadStatus::AwaitingDuplicateConfirmation
        } else {
            DownloadStatus::Queued
        };
        info!(id, to = item.status.name(), "start_download");
        Ok(())
    }

    fn previous_download(&self, item: &DownloadItem) -> Option<PreviousDownload> {
        let metadata = item.metadata.as_ref()?;
        let history = self.history.lock().expect("history lock poisoned");
        let entry = history.find_previous(&metadata.extractor, &metadata.video_id, &item.format)?;
        if file_exists(&entry.file_path) {
            Some(PreviousDownload {
                finished_at_ms: entry.finished_at_ms,
                file_path: entry.file_path.clone(),
            })
        } else {
            debug!(id = item.id, path = %entry.file_path.display(), "the earlier download's file is gone");
            None
        }
    }

    fn apply_resolved(&self, id: DownloadId, result: std::result::Result<Resolved, YtDlpFailure>) {
        let mut state = self.lock();
        let Ok(index) = state.position(id) else {
            debug!(id, "item was removed while resolving, dropping the result");
            return;
        };
        if state.items[index].status != DownloadStatus::ResolvingMetadata {
            warn!(
                id,
                expected = "resolving metadata",
                actual = state.items[index].status.name(),
                "dropping a resolve result"
            );
            return;
        }
        match result {
            Ok(Resolved::Single(metadata)) => {
                info!(id, title = %metadata.title, "resolved a single video");
                let item = &mut state.items[index];
                item.metadata = Some(metadata);
                item.previous_download = self.previous_download(item);
                item.status = DownloadStatus::Ready;
                self.sink.item_updated(&state.items[index]);
            }
            Ok(Resolved::Playlist {
                title,
                entries,
                unavailable,
            }) => {
                self.expand_playlist(&mut state, index, title, entries, unavailable);
                self.sink.queue_replaced(&state.newest_first());
            }
            Err(YtDlpFailure::Failed(error)) => {
                info!(id, kind = ?error.kind, message = %error.message, "resolving failed");
                state.items[index].status = DownloadStatus::MetadataFailed { error };
                self.sink.item_updated(&state.items[index]);
            }
            Err(YtDlpFailure::Cancelled) => {
                error!(
                    id,
                    "the engine reported a cancelled resolve, which is never requested"
                );
                state.items[index].status = DownloadStatus::MetadataFailed {
                    error: FriendlyError {
                        kind: ErrorKind::Other,
                        message: "Looking up this link stopped unexpectedly. Retry to try again."
                            .to_string(),
                        detail: "the engine returned Cancelled for a resolve".to_string(),
                    },
                };
                self.sink.item_updated(&state.items[index]);
            }
        }
        self.persist(&state);
    }

    fn expand_playlist(
        &self,
        state: &mut State,
        index: usize,
        title: Option<String>,
        entries: Vec<VideoMetadata>,
        unavailable: u32,
    ) {
        let placeholder = state.items[index].clone();
        let mut queued: HashSet<String> = state
            .items
            .iter()
            .filter(|i| i.id != placeholder.id)
            .map(|i| i.url.clone())
            .collect();
        let total = entries.len();
        let mut already_queued = 0;
        let mut invalid_urls = 0;
        let mut expanded = Vec::new();
        for entry in entries {
            let Some(url) = normalize_url(&entry.webpage_url) else {
                warn!(id = placeholder.id, url = %entry.webpage_url, video_id = %entry.video_id, "skipping a playlist entry whose URL is not http(s)");
                invalid_urls += 1;
                continue;
            };
            if !queued.insert(url.clone()) {
                already_queued += 1;
                continue;
            }
            let mut item = DownloadItem {
                id: state.allocate_id(),
                url,
                added_at_ms: placeholder.added_at_ms,
                metadata: Some(entry),
                format: placeholder.format,
                custom_name: None,
                status: DownloadStatus::Ready,
                previous_download: None,
                file_path: None,
            };
            item.previous_download = self.previous_download(&item);
            expanded.push(item);
        }
        info!(
            id = placeholder.id,
            title = ?title,
            total,
            added = expanded.len(),
            already_queued,
            invalid_urls,
            unavailable,
            "resolved a playlist"
        );
        let skipped = unavailable + count_u32(invalid_urls);
        if skipped > 0 {
            let videos = if skipped == 1 { "video" } else { "videos" };
            self.sink.notice(notice(
                NoticeLevel::Info,
                format!(
                    "Skipped {skipped} unavailable {videos} from {}",
                    title.as_deref().unwrap_or("the playlist")
                ),
            ));
        }
        if expanded.is_empty() {
            state.items[index].status = DownloadStatus::MetadataFailed {
                error: FriendlyError {
                    kind: ErrorKind::Other,
                    message: EMPTY_PLAYLIST_MESSAGE.to_string(),
                    detail: format!(
                        "{total} entries listed, {already_queued} already in the queue, {skipped} unavailable"
                    ),
                },
            };
        } else {
            state.items.splice(index..=index, expanded);
        }
    }

    fn on_download_event(&self, id: DownloadId, event: DownloadEvent) {
        let mut state = self.lock();
        let Ok(index) = state.position(id) else {
            warn!(
                id,
                ?event,
                "download event for an item that is no longer in the queue"
            );
            return;
        };
        let item = &mut state.items[index];
        if !matches!(
            item.status,
            DownloadStatus::Downloading { .. } | DownloadStatus::Processing
        ) {
            warn!(
                id,
                expected = "downloading or processing",
                actual = item.status.name(),
                ?event,
                "ignoring a download event"
            );
            return;
        }
        match event {
            DownloadEvent::Progress(progress) => {
                item.status = DownloadStatus::Downloading { progress };
                self.sink.item_updated(item);
            }
            DownloadEvent::Processing => {
                debug!(id, "post-processing");
                item.status = DownloadStatus::Processing;
                self.sink.item_updated(&state.items[index]);
                self.persist(&state);
            }
        }
    }

    fn finish_download(
        &self,
        id: DownloadId,
        request: &DownloadRequest,
        result: std::result::Result<DownloadOutcome, YtDlpFailure>,
    ) {
        let mut state = self.lock();
        if state.tokens.remove(&id).is_none() {
            error!(id, "a finished download task had no registered token");
        }
        let Ok(index) = state.position(id) else {
            error!(id, "a running download disappeared from the queue");
            return;
        };
        match result {
            Ok(outcome) => {
                info!(id, file = %outcome.file_path.display(), "download finished");
                let item = &mut state.items[index];
                item.status = DownloadStatus::Finished;
                item.file_path = Some(outcome.file_path.clone());
                let entry = history_entry(item, request, &outcome.file_path);
                state.batch.finished += 1;
                match entry {
                    Some(entry) => {
                        let recorded = self
                            .history
                            .lock()
                            .expect("history lock poisoned")
                            .record(entry);
                        match recorded {
                            Ok(()) => self.sink.history_changed(),
                            Err(e) => {
                                error!(id, error = %e, "recording the download in the history failed");
                                self.sink.notice(notice(
                                    NoticeLevel::Error,
                                    format!("The download finished, but saving it to the history failed: {e}"),
                                ));
                            }
                        }
                    }
                    None => {
                        error!(
                            id,
                            "a finished download has no metadata, not recording it in the history"
                        );
                        self.sink.notice(notice(
                            NoticeLevel::Error,
                            format!("Download {id} finished, but it could not be added to the history because its details are missing."),
                        ));
                    }
                }
            }
            Err(YtDlpFailure::Cancelled) => {
                info!(id, "download cancelled");
                state.items[index].status = DownloadStatus::Cancelled;
            }
            Err(YtDlpFailure::Failed(error)) => {
                info!(id, kind = ?error.kind, message = %error.message, "download failed");
                state.items[index].status = DownloadStatus::Failed { error };
                state.batch.failed += 1;
            }
        }
        self.sink.item_updated(&state.items[index]);
        self.persist(&state);
    }

    fn rename_on_disk(&self, item: &mut DownloadItem, name: &str) -> Result<PathBuf> {
        let Some(old) = item.file_path.clone() else {
            error!(id = item.id, "a finished download has no file path");
            return Err(YaydlError::Io(format!(
                "Download {} is finished but has no file to rename",
                item.id
            )));
        };
        let file_name = match old.extension() {
            Some(ext) => format!("{name}.{}", ext.to_string_lossy()),
            None => name.to_string(),
        };
        let new = old.with_file_name(file_name);
        if new == old {
            item.custom_name = Some(name.to_string());
            return Ok(new);
        }
        if !file_exists(&old) {
            return Err(YaydlError::Io(format!(
                "The file {} no longer exists",
                old.display()
            )));
        }
        if file_exists(&new) {
            return Err(YaydlError::Io(format!(
                "A file named {} already exists in {}",
                new.file_name()
                    .map(|n| n.to_string_lossy())
                    .unwrap_or_default(),
                new.parent()
                    .map(Path::display)
                    .map(|d| d.to_string())
                    .unwrap_or_default()
            )));
        }
        fs::rename(&old, &new).map_err(|e| {
            YaydlError::Io(format!(
                "Renaming {} to {} failed: {e}",
                old.display(),
                new.display()
            ))
        })?;
        info!(id = item.id, from = %old.display(), to = %new.display(), "renamed a finished download");
        item.file_path = Some(new.clone());
        item.custom_name = Some(name.to_string());
        Ok(new)
    }
}

fn load_file(path: &Path) -> std::result::Result<Option<QueueFile>, PersistError> {
    let Some(raw) = read_json::<serde_json::Value>(path)? else {
        return Ok(None);
    };
    let version = raw.get("version").and_then(serde_json::Value::as_u64);
    if version != Some(QUEUE_VERSION) {
        return Err(PersistError::new(
            path,
            format!(
                "queue version is {}, expected {QUEUE_VERSION}",
                raw.get("version")
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "missing".to_string())
            ),
        ));
    }
    let file: QueueFile = serde_json::from_value(raw)
        .map_err(|e| PersistError::new(path, format!("invalid queue: {e}")))?;
    let mut ids = HashSet::new();
    for item in &file.items {
        if !ids.insert(item.id) {
            return Err(PersistError::new(
                path,
                format!("duplicate item id {}", item.id),
            ));
        }
        if item.id >= file.next_id {
            return Err(PersistError::new(
                path,
                format!("item id {} is not below next_id {}", item.id, file.next_id),
            ));
        }
        let needs_metadata = !matches!(
            item.status,
            DownloadStatus::ResolvingMetadata | DownloadStatus::MetadataFailed { .. }
        );
        if needs_metadata && item.metadata.is_none() {
            return Err(PersistError::new(
                path,
                format!(
                    "item {} is {} but has no metadata",
                    item.id,
                    item.status.name()
                ),
            ));
        }
        if item.status == DownloadStatus::Finished && item.file_path.is_none() {
            return Err(PersistError::new(
                path,
                format!("item {} is finished but has no file path", item.id),
            ));
        }
    }
    Ok(Some(file))
}

fn download_request(item: &DownloadItem, settings: &Settings) -> DownloadRequest {
    DownloadRequest {
        url: item.url.clone(),
        output_dir: settings.output_dir.clone(),
        format: item.format,
        file_stem: item.custom_name.clone(),
        embed_metadata: settings.embed_metadata,
        run: run_options(settings),
    }
}

fn run_options(settings: &Settings) -> RunOptions {
    RunOptions {
        channel: settings.yt_dlp_channel,
        cookies_from_browser: settings.cookies_from_browser,
    }
}

fn history_entry(
    item: &DownloadItem,
    request: &DownloadRequest,
    file: &Path,
) -> Option<HistoryEntry> {
    let metadata = item.metadata.as_ref()?;
    let file_size_bytes = match fs::metadata(file) {
        Ok(m) => Some(m.len()),
        Err(e) => {
            warn!(id = item.id, file = %file.display(), error = %e, "reading the size of a finished download failed");
            None
        }
    };
    Some(HistoryEntry {
        download_id: item.id,
        extractor: metadata.extractor.clone(),
        video_id: metadata.video_id.clone(),
        title: metadata.title.clone(),
        uploader: metadata.uploader.clone(),
        url: item.url.clone(),
        format: request.format,
        file_path: file.to_path_buf(),
        file_size_bytes,
        duration_secs: metadata.duration_secs,
        finished_at_ms: now_ms(),
    })
}

/// `None` for anything but an http(s) URL with a host. The normalized form is
/// what duplicate detection compares.
pub fn normalize_url(token: &str) -> Option<String> {
    let url = url::Url::parse(token).ok()?;
    let web = matches!(url.scheme(), "http" | "https");
    let has_host = url.host_str().is_some_and(|h| !h.is_empty());
    (web && has_host).then(|| url.as_str().to_string())
}

/// Like [`file_exists`], but a file where a folder is expected also counts as
/// missing.
fn dir_exists(path: &Path) -> bool {
    match fs::metadata(path) {
        Ok(metadata) => metadata.is_dir(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "checking whether a folder exists failed");
            false
        }
    }
}

/// A failing existence check (permissions, I/O) is logged and treated as
/// missing, because every caller then shows the user a "does not exist" state
/// that names the path.
fn file_exists(path: &Path) -> bool {
    match path.try_exists() {
        Ok(exists) => exists,
        Err(e) => {
            warn!(path = %path.display(), error = %e, "checking whether a path exists failed");
            false
        }
    }
}

fn invalid_state(action: &str, status: &DownloadStatus) -> YaydlError {
    YaydlError::InvalidState {
        action: action.to_string(),
        state: status.name().to_string(),
    }
}

fn count_u32(n: usize) -> u32 {
    u32::try_from(n).expect("queue sizes fit in u32")
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests;
