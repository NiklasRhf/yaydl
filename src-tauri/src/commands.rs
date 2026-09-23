//! Thin Tauri wrappers. Every command is async so none runs on the main
//! thread, where a blocking call would freeze the window.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use tauri::{AppHandle, Emitter, State, Wry};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::oneshot;
use tracing::{info, instrument};
use yaydl_shared::{
    events, AddUrlsResult, DownloadId, DownloadItem, HistoryEntry, Notice, OutputFormat, Settings,
    Statistics, StatsGranularity, YaydlError, YtDlpStatus, YtDlpUpdateEvent,
};

use crate::{
    history::{compute_statistics, History},
    notices::Notices,
    queue::Queue,
    settings::SettingsStore,
    sink::TauriSink,
    ytdlp::{ProcessRunner, YtDlp},
};

pub type AppEngine = YtDlp<ProcessRunner>;
pub type AppQueue = Queue<AppEngine, TauriSink<Wry>>;

pub struct HistoryState(pub Arc<Mutex<History>>);

type Result<T> = std::result::Result<T, YaydlError>;

fn open_path(app: &AppHandle, path: &Path) -> Result<()> {
    let utf8 = path
        .to_str()
        .ok_or_else(|| YaydlError::Open(format!("{} is not a valid UTF-8 path", path.display())))?;
    app.opener()
        .open_path(utf8, None::<&str>)
        .map_err(|e| YaydlError::Open(format!("{}: {e}", path.display())))
}

#[tauri::command]
pub async fn get_queue(queue: State<'_, AppQueue>) -> Result<Vec<DownloadItem>> {
    Ok(queue.get_queue())
}

#[tauri::command]
pub async fn add_urls(queue: State<'_, AppQueue>, text: String) -> Result<AddUrlsResult> {
    Ok(queue.add_urls(&text))
}

#[tauri::command]
pub async fn add_from_clipboard(
    app: AppHandle,
    queue: State<'_, AppQueue>,
) -> Result<AddUrlsResult> {
    let text = crate::clipboard::read_text(&app).await?;
    Ok(queue.add_urls(&text))
}

#[tauri::command]
pub async fn start_download(queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    queue.start_download(id)
}

#[tauri::command]
pub async fn start_all(queue: State<'_, AppQueue>) -> Result<u32> {
    Ok(queue.start_all())
}

#[tauri::command]
pub async fn retry_download(queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    queue.retry_download(id)
}

#[tauri::command]
pub async fn expand_playlist(queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    queue.expand_playlist(id)
}

#[tauri::command]
pub async fn cancel_download(queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    queue.cancel_download(id)
}

#[tauri::command]
pub async fn remove_item(queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    queue.remove_item(id)
}

#[tauri::command]
pub async fn clear_finished(queue: State<'_, AppQueue>) -> Result<u32> {
    Ok(queue.clear_finished())
}

#[tauri::command]
pub async fn clear_all(queue: State<'_, AppQueue>) -> Result<u32> {
    Ok(queue.clear_all())
}

#[tauri::command]
pub async fn confirm_duplicate(
    queue: State<'_, AppQueue>,
    id: DownloadId,
    proceed: bool,
) -> Result<()> {
    queue.confirm_duplicate(id, proceed)
}

#[tauri::command]
pub async fn set_item_format(
    queue: State<'_, AppQueue>,
    id: DownloadId,
    format: OutputFormat,
) -> Result<()> {
    queue.set_item_format(id, format)
}

#[tauri::command]
pub async fn rename_item(queue: State<'_, AppQueue>, id: DownloadId, name: String) -> Result<()> {
    queue.rename_item(id, &name)
}

#[tauri::command]
pub async fn open_file(app: AppHandle, queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    let path = queue.finished_file(id)?;
    info!(id, path = %path.display(), "opening a downloaded file");
    open_path(&app, &path)
}

#[tauri::command]
pub async fn reveal_file(app: AppHandle, queue: State<'_, AppQueue>, id: DownloadId) -> Result<()> {
    let path = queue.finished_file(id)?;
    info!(id, path = %path.display(), "revealing a downloaded file");
    app.opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| YaydlError::Open(format!("{}: {e}", path.display())))
}

#[tauri::command]
pub async fn open_output_dir(
    app: AppHandle,
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<()> {
    let dir = settings.get().output_dir;
    if !dir.is_dir() {
        return Err(YaydlError::Open(format!(
            "the output folder {} does not exist",
            dir.display()
        )));
    }
    info!(dir = %dir.display(), "opening the output folder");
    open_path(&app, &dir)
}

#[tauri::command]
pub async fn get_statistics(
    history: State<'_, HistoryState>,
    granularity: StatsGranularity,
) -> Result<Statistics> {
    let history = history.0.lock().expect("history lock poisoned");
    Ok(compute_statistics(
        history.entries(),
        granularity,
        &jiff::Zoned::now(),
    ))
}

#[tauri::command]
pub async fn get_history(history: State<'_, HistoryState>) -> Result<Vec<HistoryEntry>> {
    Ok(history
        .0
        .lock()
        .expect("history lock poisoned")
        .newest_first())
}

#[tauri::command]
pub async fn clear_history(app: AppHandle, history: State<'_, HistoryState>) -> Result<()> {
    history.0.lock().expect("history lock poisoned").clear()?;
    info!("history cleared");
    if let Err(e) = app.emit(events::HISTORY_CHANGED, ()) {
        tracing::error!(error = %e, "emitting history-changed failed");
    }
    Ok(())
}

#[tauri::command]
pub async fn get_settings(settings: State<'_, Arc<SettingsStore>>) -> Result<Settings> {
    Ok(settings.get())
}

#[tauri::command]
#[instrument(skip_all)]
pub async fn update_settings(
    store: State<'_, Arc<SettingsStore>>,
    queue: State<'_, AppQueue>,
    settings: Settings,
) -> Result<Settings> {
    let updated = store.update(settings)?;
    queue.schedule();
    Ok(updated)
}

/// Only picks. The UI persists the choice with `update_settings`.
#[tauri::command]
pub async fn choose_output_dir(app: AppHandle) -> Result<Option<String>> {
    let (tx, rx) = oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        if tx.send(picked).is_err() {
            tracing::error!("the folder dialog answered after choose_output_dir stopped waiting");
        }
    });
    let picked = rx
        .await
        .map_err(|e| YaydlError::Io(format!("the folder dialog closed without an answer: {e}")))?;
    let Some(picked) = picked else {
        info!("folder selection cancelled");
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| YaydlError::Io(format!("the chosen folder is not a local path: {e}")))?;
    let path = path.into_os_string().into_string().map_err(|raw| {
        YaydlError::Io(format!(
            "the chosen folder {} is not valid UTF-8",
            raw.to_string_lossy()
        ))
    })?;
    info!(%path, "folder chosen");
    Ok(Some(path))
}

#[tauri::command]
#[instrument(skip_all)]
pub async fn get_yt_dlp_status(
    ytdlp: State<'_, Arc<AppEngine>>,
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<YtDlpStatus> {
    let channel = settings.get().yt_dlp_channel;
    let version = ytdlp
        .version()
        .await
        .inspect_err(|e| info!(%channel, error = %e, "get_yt_dlp_status failed"))?;
    info!(%version, %channel, "get_yt_dlp_status ok");
    Ok(YtDlpStatus {
        version,
        channel,
        binary_path: ytdlp.binary().display().to_string(),
    })
}

#[tauri::command]
#[instrument(skip_all)]
pub async fn update_yt_dlp(
    ytdlp: State<'_, Arc<AppEngine>>,
    settings: State<'_, Arc<SettingsStore>>,
) -> Result<YtDlpUpdateEvent> {
    let channel = settings.get().yt_dlp_channel;
    let event = ytdlp
        .update(channel)
        .await
        .inspect_err(|e| info!(%channel, error = %e, "update_yt_dlp failed"))?;
    info!(%channel, ?event, "update_yt_dlp ok");
    Ok(event)
}

#[tauri::command]
pub async fn take_startup_notices(notices: State<'_, Arc<Notices>>) -> Result<Vec<Notice>> {
    Ok(notices.take_startup())
}
