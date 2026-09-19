use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_shell::ShellExt;
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

mod settings;
pub mod ytdlp;
use settings::Setup;
use tauri_plugin_updater::UpdaterExt;
use yaydl_shared::{
    AddLinkError, Download, DownloadEvent, DownloadState, LogSnapshot, Metadata, MetadataError,
    Settings, UpdateError, YaydlError, YtDlpError, YtDlpStatus, YtDlpUpdateEvent,
};
use ytdlp::{ProcessRunner, Runner, YtDlp};

type Result<T> = std::result::Result<T, YaydlError>;

const LOG_FILE_NAME: &str = "yaydl.log";
const LOG_ENV_VAR: &str = "YAYDL_LOG";
const DEFAULT_LOG_FILTER: &str = "info,yaydl_lib=debug";

/// Lines logged from the webview carry this target so they are distinguishable
/// from backend lines in the same file.
const UI_TARGET: &str = "yaydl_ui";

/// The log viewer reads at most this many lines regardless of what the UI asks for.
const MAX_LOG_LINES: usize = 2000;

/// The non-blocking writer flushes on drop, so the guard has to outlive every
/// log call, which means the whole process.
static LOG_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub struct AppData {
    download_list: Vec<Download>,
    settings: Settings,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            download_list: Default::default(),
            settings: Settings::with_defaults(),
        }
    }
}

#[tauri::command]
async fn get_downloads<R: Runtime>(app_handle: tauri::AppHandle<R>) -> Vec<Download> {
    app_handle
        .state::<Mutex<AppData>>()
        .lock()
        .unwrap()
        .download_list
        .clone()
}

#[tauri::command]
async fn clear_downloads<R: Runtime>(app_handle: tauri::AppHandle<R>) {
    app_handle
        .state::<Mutex<AppData>>()
        .lock()
        .unwrap()
        .download_list
        .clear();
}

#[tauri::command]
async fn update_download<R: Runtime>(
    app_handle: tauri::AppHandle<R>,
    id: String,
    state: DownloadState,
) {
    if let Some(download) = app_handle
        .state::<Mutex<AppData>>()
        .lock()
        .unwrap()
        .download_list
        .iter_mut()
        .find(|d| d.metadata.id == id)
    {
        download.download_state = state;
    }
}

#[tauri::command]
#[tracing::instrument(skip_all)]
async fn try_add<R: Runtime>(app_handle: AppHandle<R>) -> Result<(String, Vec<Download>)> {
    info!("reading a link from the clipboard");
    let content = app_handle.clipboard().read_text();
    let state = app_handle.state::<Mutex<AppData>>();
    let result = match content {
        Ok(url) if url.contains("https://www.youtube.com/") => {
            let download = Download {
                metadata: Metadata {
                    url: url.clone(),
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut state = state.lock().unwrap();
            let contains = state.download_list.contains(&download);
            if !contains {
                state.download_list.insert(0, download);
                Ok((url, state.download_list.clone()))
            } else {
                Err(YaydlError::AddLinkError(AddLinkError::AlreadyAdded))
            }
        }
        Ok(_) => Err(YaydlError::AddLinkError(AddLinkError::NoValidLink)),
        Err(_) => Err(YaydlError::AddLinkError(AddLinkError::ClipboardRead)),
    };
    match &result {
        Ok((url, list)) => info!(%url, downloads = list.len(), "added a link"),
        Err(e) => info!(error = %e, "adding a link failed"),
    }
    result
}

#[tauri::command]
fn open_explorer<R: Runtime>(
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<()> {
    let shell = app_handle.shell();
    let explorer = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return Err(YaydlError::UnsupportedOs);
    };
    let output_dir = state
        .lock()
        .unwrap()
        .settings
        .output_dir
        .display()
        .to_string();
    shell.command(explorer).arg(output_dir).spawn().unwrap();
    Ok(())
}

#[tauri::command]
#[tracing::instrument(skip_all, fields(url = %url))]
async fn retreive_metadata<R: Runtime>(
    url: String,
    app_handle: AppHandle<R>,
    ytdlp: tauri::State<'_, YtDlp<ProcessRunner>>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<Metadata> {
    let channel = state.lock().unwrap().settings.yt_dlp_channel;
    let args = ytdlp.metadata_args(&url);

    let output = ytdlp
        .run_with_self_heal(channel, &args, |runner, binary, args| {
            Box::pin(async move {
                let output = runner
                    .output(binary, args)
                    .await
                    .map_err(|e| YtDlpError::Io(e.to_string()))?;
                ytdlp::check(output, args)
            })
        })
        .await
        .inspect_err(|e| info!(error = %e, "retreive_metadata failed"))?;

    let fields: Vec<&str> = output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    if fields.len() < 4 {
        info!(
            fields = fields.len(),
            "retreive_metadata failed: yt-dlp returned too few fields"
        );
        return Err(YaydlError::MetadataError(MetadataError::MissingFields));
    }

    let metadata = Metadata {
        title: fields[0].to_string(),
        id: fields[1].to_string(),
        thumbnail: fields[2].to_string(),
        duration: fields[3].to_string(),
        url,
        loading: false,
    };

    if let Some(d) = app_handle
        .state::<Mutex<AppData>>()
        .lock()
        .unwrap()
        .download_list
        .iter_mut()
        .find(|d| d.metadata.url == metadata.url)
    {
        d.metadata = metadata.clone();
    }

    info!(id = %metadata.id, title = %metadata.title, "retreive_metadata ok");
    Ok(metadata)
}

#[tauri::command]
#[tracing::instrument(skip_all, fields(url = %url, id = %id))]
async fn execute_yt_dl<R: Runtime>(
    url: String,
    id: String,
    app_handle: AppHandle<R>,
    ytdlp: tauri::State<'_, YtDlp<ProcessRunner>>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<()> {
    let (output_dir, output_format, channel) = {
        let settings = &state.lock().unwrap().settings;
        (
            settings.output_dir.display().to_string(),
            settings.output_format.clone(),
            settings.yt_dlp_channel,
        )
    };
    let args = ytdlp.download_args(&url, &output_dir, &output_format);
    info!(%output_dir, %output_format, %channel, "download starting");

    ytdlp
        .run_with_self_heal(channel, &args, |runner, binary, args| {
            let app_handle = app_handle.clone();
            let id = id.clone();
            Box::pin(async move {
                let output = runner
                    .stream(
                        binary,
                        args,
                        Box::new(move |line| emit_progress(&app_handle, &id, line)),
                    )
                    .await
                    .map_err(|e| YtDlpError::Io(e.to_string()))?;
                ytdlp::check(output, args).map(|_| ())
            })
        })
        .await
        .inspect_err(|e| info!(error = %e, "download failed"))?;

    info!("download finished");
    Ok(())
}

fn emit_progress<R: Runtime>(app_handle: &AppHandle<R>, id: &str, line: &str) {
    let Some(remainder) = line.strip_prefix("[download]") else {
        return;
    };
    let Some(percent) = remainder.trim_start().split(' ').next() else {
        return;
    };
    let Some(Ok(progress)) = percent.strip_suffix('%').map(str::parse::<f32>) else {
        return;
    };
    // Every parsed percentage would be hundreds of lines per download.
    if progress == 0.0 {
        debug!(%id, "yt-dlp download progress started");
    }
    if let Err(e) = app_handle.emit(
        "download-progress",
        DownloadEvent {
            id: id.to_string(),
            progress: progress as u8,
        },
    ) {
        error!(%id, error = %e, "emitting download-progress failed");
    }
}

#[tauri::command]
#[tracing::instrument(skip_all)]
async fn get_yt_dlp_status(
    ytdlp: tauri::State<'_, YtDlp<ProcessRunner>>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<YtDlpStatus> {
    let channel = state.lock().unwrap().settings.yt_dlp_channel;
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
#[tracing::instrument(skip_all)]
async fn update_yt_dlp(
    ytdlp: tauri::State<'_, YtDlp<ProcessRunner>>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<YtDlpUpdateEvent> {
    let channel = state.lock().unwrap().settings.yt_dlp_channel;
    let event = ytdlp
        .update(channel)
        .await
        .inspect_err(|e| info!(%channel, error = %e, "update_yt_dlp failed"))?;
    info!(%channel, ?event, "update_yt_dlp ok");
    Ok(event)
}

#[tauri::command]
fn quit_app<R: Runtime>(app: AppHandle<R>) {
    info!("quitting on user request");
    app.exit(0);
}

/// Bridges webview logs into the same subscriber, so a wasm panic ends up in the
/// log file next to the backend line that triggered it.
#[tauri::command]
fn ui_log(level: String, message: String) -> Result<()> {
    match level.as_str() {
        "error" => error!(target: UI_TARGET, "{message}"),
        "warn" => warn!(target: UI_TARGET, "{message}"),
        "info" => info!(target: UI_TARGET, "{message}"),
        "debug" => debug!(target: UI_TARGET, "{message}"),
        other => {
            return Err(YaydlError::UnknownLogLevel(other.to_string()));
        }
    }
    Ok(())
}

#[tauri::command]
fn get_recent_logs<R: Runtime>(app: AppHandle<R>, max_lines: usize) -> Result<LogSnapshot> {
    let path = LOG_PATH
        .get()
        .ok_or_else(|| YaydlError::LogsUnavailable("logging was never initialised".to_string()))?;
    let contents = std::fs::read_to_string(path).map_err(|e| {
        YaydlError::LogsUnavailable(format!("reading {} failed: {e}", path.display()))
    })?;

    let all: Vec<&str> = contents.lines().collect();
    let wanted = max_lines.min(MAX_LOG_LINES);
    let start = all.len().saturating_sub(wanted);
    Ok(LogSnapshot {
        path: path.display().to_string(),
        app_version: app.package_info().version.to_string(),
        lines: all[start..].iter().map(|l| l.to_string()).collect(),
        truncated: start > 0,
    })
}

#[tauri::command]
fn copy_to_clipboard<R: Runtime>(app: AppHandle<R>, text: String) -> Result<()> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| YaydlError::ClipboardWrite(e.to_string()))
}

#[tauri::command]
async fn check_update<R: Runtime>(app: tauri::AppHandle<R>) -> Result<bool> {
    // Simulate update if env var is set
    if std::env::var("YAYDL_SIMULATE_UPDATE").ok().as_deref() == Some("1") {
        return Ok(true);
    }
    let update = app
        .updater_builder()
        .build()
        .map_err(|e| {
            error!(error = %e, "building the app updater failed");
            YaydlError::UpdateError(UpdateError::BuildFailed)
        })?
        .check()
        .await
        .map_err(|e| {
            error!(error = %e, "checking for an app update failed");
            YaydlError::UpdateError(UpdateError::CheckFailed)
        })?;
    info!(
        available = update.is_some(),
        version = update.as_ref().map(|u| u.version.as_str()),
        "app update check finished"
    );
    Ok(update.is_some())
}

#[tauri::command]
async fn start_update<R: Runtime>(app: tauri::AppHandle<R>) -> Result<()> {
    // Simulate update if env var is set
    if std::env::var("YAYDL_SIMULATE_UPDATE").ok().as_deref() == Some("1") {
        for percent in (0..=100).step_by(10) {
            let _ = app.emit("update-progress", percent);
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let _ = app.emit("update-finished", ());
        return Ok(());
    }
    if let Some(update) = app
        .updater_builder()
        .build()
        .map_err(|_| YaydlError::UpdateError(UpdateError::BuildFailed))?
        .check()
        .await
        .map_err(|_| YaydlError::UpdateError(UpdateError::CheckFailed))?
    {
        let mut downloaded = 0u64;
        let mut total = 0u64;
        update
            .download_and_install(
                |chunk_length, content_length| {
                    downloaded += chunk_length as u64;
                    total = content_length.unwrap_or(0);
                    let percent = (downloaded * 100).checked_div(total).unwrap_or(0) as u8;
                    let _ = app.emit("update-progress", percent);
                },
                || {
                    let _ = app.emit("update-finished", ());
                },
            )
            .await
            .map_err(|_| YaydlError::UpdateError(UpdateError::DownloadAndInstallFailed))?;
        app.restart();
    }
    Ok(())
}

/// Resolves the log directory without an `AppHandle`, because logging has to be
/// up before `tauri::Builder` runs. Matches Tauri's `app_log_dir` on Linux
/// (`$XDG_DATA_HOME/<identifier>/logs`) and on Windows
/// (`%LOCALAPPDATA%\<identifier>\logs`).
fn log_dir(identifier: &str) -> PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| panic!("no local data directory for the current platform"));
    base.join(identifier).join("logs")
}

fn init_tracing(log_dir: &Path) -> WorkerGuard {
    std::fs::create_dir_all(log_dir).unwrap_or_else(|e| {
        panic!(
            "creating the log directory {} failed: {e}",
            log_dir.display()
        )
    });

    let filter = match std::env::var(LOG_ENV_VAR) {
        Ok(directives) => EnvFilter::builder().parse(&directives).unwrap_or_else(|e| {
            panic!("{LOG_ENV_VAR}=\"{directives}\" is not a valid tracing filter: {e}")
        }),
        Err(std::env::VarError::NotPresent) => EnvFilter::builder()
            .parse(DEFAULT_LOG_FILTER)
            .expect("the built-in default log filter to be valid"),
        Err(e) => panic!("reading {LOG_ENV_VAR} failed: {e}"),
    };

    let (file_writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::never(log_dir, LOG_FILE_NAME));

    // Span fields are formatted once by the first fmt layer and shared with the
    // others, so the file layer goes first to keep ANSI codes out of the log file.
    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_level(true)
                .with_timer(fmt::time::SystemTime)
                .with_writer(file_writer),
        )
        .with(
            fmt::layer()
                .with_ansi(true)
                .with_target(true)
                .with_level(true)
                .with_timer(fmt::time::SystemTime)
                .with_writer(std::io::stderr),
        )
        .init();

    guard
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    let log_dir = log_dir(&context.config().identifier);
    if LOG_GUARD.set(init_tracing(&log_dir)).is_err() {
        panic!("tracing was initialised twice");
    }
    let log_file = log_dir.join(LOG_FILE_NAME);
    if LOG_PATH.set(log_file.clone()).is_err() {
        panic!("the log path was set twice");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = check_update(handle).await {
                    error!(error = %e, "app update check failed");
                }
            });
            let config_dir = app.path().app_config_dir()?;
            let settings = Settings::setup_settings(&config_dir);
            let channel = settings.yt_dlp_channel;

            let exe = std::env::current_exe()?;
            let exe_dir = exe
                .parent()
                .ok_or("the executable has no parent directory")?
                .to_path_buf();
            let data_dir = app.path().app_data_dir()?.join("yt-dlp");
            let bootstrap = exe_dir.join(format!("yt-dlp{}", std::env::consts::EXE_SUFFIX));
            let ytdlp = YtDlp::new(
                ProcessRunner,
                data_dir.clone(),
                bootstrap.clone(),
                exe_dir.clone(),
            );
            info!(
                version = %app.package_info().version,
                exe = %exe.display(),
                exe_dir = %exe_dir.display(),
                app_data_dir = %data_dir.display(),
                app_config_dir = %config_dir.display(),
                log_file = %log_file.display(),
                bootstrap_yt_dlp = %bootstrap.display(),
                managed_yt_dlp = %ytdlp.binary().display(),
                "yaydl starting"
            );
            ytdlp.ensure_installed()?;
            app.manage(ytdlp);

            app.manage(Mutex::new(AppData {
                download_list: Vec::new(),
                settings,
            }));

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let ytdlp = handle.state::<YtDlp<ProcessRunner>>();
                let event = match ytdlp.maybe_update_on_startup(channel).await {
                    Ok(None) => return,
                    Ok(Some(event)) => event,
                    Err(e) => {
                        warn!(error = %e, %channel, "yt-dlp startup update failed");
                        YtDlpUpdateEvent::Failed {
                            message: e.to_string(),
                        }
                    }
                };
                if let Err(e) = handle.emit("ytdlp-update", event) {
                    error!(error = %e, "emitting ytdlp-update failed");
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            execute_yt_dl,
            try_add,
            retreive_metadata,
            open_explorer,
            get_downloads,
            clear_downloads,
            update_download,
            check_update,
            start_update,
            get_yt_dlp_status,
            update_yt_dlp,
            quit_app,
            ui_log,
            get_recent_logs,
            copy_to_clipboard,
            settings::choose_output_dir,
            settings::set_output_format,
            settings::set_yt_dlp_channel,
            settings::set_dark_theme,
            settings::get_settings,
        ])
        .run(context)
        .expect("error while running tauri application");
}
