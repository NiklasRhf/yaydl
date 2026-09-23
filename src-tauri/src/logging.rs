use std::{
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use tauri::{AppHandle, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tracing::{debug, error, info, warn};
use tracing_appender::{
    non_blocking::WorkerGuard,
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use yaydl_shared::{LogSnapshot, YaydlError};

const LOG_FILE_PREFIX: &str = "yaydl";
const LOG_FILE_SUFFIX: &str = "log";
const KEPT_LOG_FILES: usize = 7;
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

static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Resolves the log directory without an `AppHandle`, because logging has to be
/// up before `tauri::Builder` runs. Matches Tauri's `app_log_dir` on Linux
/// (`$XDG_DATA_HOME/<identifier>/logs`) and on Windows
/// (`%LOCALAPPDATA%\<identifier>\logs`).
pub fn log_dir(identifier: &str) -> PathBuf {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| panic!("no local data directory for the current platform"));
    base.join(identifier).join("logs")
}

/// Panics on any failure, because the app must not run without its log.
pub fn init(log_dir: &Path) {
    fs::create_dir_all(log_dir).unwrap_or_else(|e| {
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

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(KEPT_LOG_FILES)
        .build(log_dir)
        .unwrap_or_else(|e| {
            panic!(
                "creating the log file appender in {} failed: {e}",
                log_dir.display()
            )
        });
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

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

    if LOG_GUARD.set(guard).is_err() {
        panic!("tracing was initialised twice");
    }
    if LOG_DIR.set(log_dir.to_path_buf()).is_err() {
        panic!("the log directory was set twice");
    }
}

/// The file the appender currently writes to. Daily files are named
/// `yaydl.<YYYY-MM-DD>.log`, so the lexicographically largest is the newest.
pub fn newest_log_file(dir: &Path) -> Result<Option<PathBuf>, std::io::Error> {
    let prefix = format!("{LOG_FILE_PREFIX}.");
    let suffix = format!(".{LOG_FILE_SUFFIX}");
    let mut newest: Option<(String, PathBuf)> = None;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let is_log = name
            .strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(&suffix))
            .is_some_and(|date| !date.is_empty());
        if is_log && newest.as_ref().is_none_or(|(n, _)| name > n.as_str()) {
            newest = Some((name.to_string(), entry.path()));
        }
    }
    Ok(newest.map(|(_, path)| path))
}

/// Bridges webview logs into the same subscriber, so a wasm panic ends up in the
/// log file next to the backend line that triggered it.
#[tauri::command]
pub async fn ui_log(level: String, message: String) -> Result<(), YaydlError> {
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
pub async fn get_recent_logs<R: Runtime>(
    app: AppHandle<R>,
    max_lines: usize,
) -> Result<LogSnapshot, YaydlError> {
    let dir = LOG_DIR
        .get()
        .ok_or_else(|| YaydlError::LogsUnavailable("logging was never initialised".to_string()))?;
    let path = newest_log_file(dir)
        .map_err(|e| YaydlError::LogsUnavailable(format!("listing {} failed: {e}", dir.display())))?
        .ok_or_else(|| YaydlError::LogsUnavailable(format!("no log file in {}", dir.display())))?;
    let contents = fs::read_to_string(&path).map_err(|e| {
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
pub async fn copy_to_clipboard<R: Runtime>(
    app: AppHandle<R>,
    text: String,
) -> Result<(), YaydlError> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| YaydlError::ClipboardWrite(e.to_string()))
}

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_daily_file_is_picked_and_other_files_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(newest_log_file(dir.path()).unwrap(), None);

        for name in [
            "yaydl.2026-09-21.log",
            "yaydl.2026-09-23.log",
            "yaydl.2026-09-22.log",
            "yaydl.log",
            "yaydl.2026-09-30.txt",
            "other.2026-10-01.log",
        ] {
            fs::write(dir.path().join(name), "").unwrap();
        }

        assert_eq!(
            newest_log_file(dir.path()).unwrap(),
            Some(dir.path().join("yaydl.2026-09-23.log"))
        );
    }
}
