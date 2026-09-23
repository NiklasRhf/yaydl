use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tracing::{debug, error, info, warn};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

pub mod history;
pub mod persist;
pub mod ytdlp;
use yaydl_shared::{LogSnapshot, YaydlError, YtDlpChannel, YtDlpStatus, YtDlpUpdateEvent};
use ytdlp::{ProcessRunner, YtDlp};

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

#[tauri::command]
#[tracing::instrument(skip_all)]
async fn get_yt_dlp_status(ytdlp: tauri::State<'_, YtDlp<ProcessRunner>>) -> Result<YtDlpStatus> {
    let channel = YtDlpChannel::default();
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
async fn update_yt_dlp(ytdlp: tauri::State<'_, YtDlp<ProcessRunner>>) -> Result<YtDlpUpdateEvent> {
    let channel = YtDlpChannel::default();
    let event = ytdlp
        .update(channel)
        .await
        .inspect_err(|e| info!(%channel, error = %e, "update_yt_dlp failed"))?;
    info!(%channel, ?event, "update_yt_dlp ok");
    Ok(event)
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
            let config_dir = app.path().app_config_dir()?;
            let channel = YtDlpChannel::default();

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

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let ytdlp = handle.state::<YtDlp<ProcessRunner>>();
                match ytdlp.maybe_update_on_startup(channel).await {
                    Ok(event) => info!(?event, "yt-dlp startup update check done"),
                    Err(e) => warn!(error = %e, %channel, "yt-dlp startup update failed"),
                }
            });
            Ok(())
        })
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            get_yt_dlp_status,
            update_yt_dlp,
            ui_log,
            get_recent_logs,
            copy_to_clipboard,
        ])
        .run(context)
        .expect("error while running tauri application");
}
