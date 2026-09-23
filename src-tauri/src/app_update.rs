use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};
use tracing::{error, info};
use yaydl_shared::{events, AppInfo, AppUpdateInfo, AppUpdateProgress, YaydlError};

async fn check<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, YaydlError> {
    let updater = app.updater_builder().build().map_err(|e| {
        error!(error = %e, "building the app updater failed");
        YaydlError::AppUpdate(format!("building the updater failed: {e}"))
    })?;
    let update = updater.check().await.map_err(|e| {
        error!(error = %e, "checking for an app update failed");
        YaydlError::AppUpdate(format!("checking for an update failed: {e}"))
    })?;
    info!(
        available = update.is_some(),
        version = update.as_ref().map(|u| u.version.as_str()),
        "app update check finished"
    );
    Ok(update)
}

fn emit_progress<R: Runtime>(app: &AppHandle<R>, progress: AppUpdateProgress) {
    if let Err(e) = app.emit(events::APP_UPDATE_PROGRESS, &progress) {
        error!(error = %e, ?progress, "emitting app update progress failed");
    }
}

#[tauri::command]
pub async fn check_app_update<R: Runtime>(
    app: AppHandle<R>,
) -> Result<Option<AppUpdateInfo>, YaydlError> {
    #[cfg(debug_assertions)]
    if simulation::enabled() {
        return Ok(Some(simulation::info(&app)));
    }
    Ok(check(&app).await?.map(|update| AppUpdateInfo {
        current_version: update.current_version.clone(),
        version: update.version.clone(),
        notes: update.body.clone(),
    }))
}

#[tauri::command]
pub async fn install_app_update<R: Runtime>(app: AppHandle<R>) -> Result<(), YaydlError> {
    #[cfg(debug_assertions)]
    if simulation::enabled() {
        simulation::install(&app).await;
        return Ok(());
    }
    let update = check(&app)
        .await?
        .ok_or_else(|| YaydlError::AppUpdate("No update is available anymore".to_string()))?;
    info!(version = %update.version, "installing app update");
    let mut downloaded_bytes = 0u64;
    update
        .download_and_install(
            |chunk, total_bytes| {
                downloaded_bytes += chunk as u64;
                emit_progress(
                    &app,
                    AppUpdateProgress {
                        downloaded_bytes,
                        total_bytes,
                    },
                );
            },
            || info!("app update downloaded, installing"),
        )
        .await
        .map_err(|e| {
            error!(error = %e, version = %update.version, "installing the app update failed");
            YaydlError::AppUpdate(format!("installing {} failed: {e}", update.version))
        })?;
    info!(version = %update.version, "app update installed, restarting");
    app.restart()
}

/// `YAYDL_SIMULATE_UPDATE=1` in a debug build fakes an available update, so the
/// update UI can be exercised without a release.
#[cfg(debug_assertions)]
mod simulation {
    use std::time::Duration;

    use tauri::{AppHandle, Runtime};
    use tracing::info;
    use yaydl_shared::{AppUpdateInfo, AppUpdateProgress};

    const TOTAL_BYTES: u64 = 10 * 1024 * 1024;

    pub fn enabled() -> bool {
        std::env::var("YAYDL_SIMULATE_UPDATE").is_ok_and(|v| v == "1")
    }

    pub fn info<R: Runtime>(app: &AppHandle<R>) -> AppUpdateInfo {
        info!("simulating an available app update");
        AppUpdateInfo {
            current_version: app.package_info().version.to_string(),
            version: "99.0.0".to_string(),
            notes: Some("Simulated update (YAYDL_SIMULATE_UPDATE=1).".to_string()),
        }
    }

    pub async fn install<R: Runtime>(app: &AppHandle<R>) {
        info!("simulating an app update install");
        for step in 0..=10 {
            super::emit_progress(
                app,
                AppUpdateProgress {
                    downloaded_bytes: TOTAL_BYTES / 10 * step,
                    total_bytes: Some(TOTAL_BYTES),
                },
            );
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        info!("simulated app update finished, not restarting");
    }
}

/// The release workflow tags every release as `yaydl-v<version>`.
const RELEASE_TAG_URL: &str = "https://github.com/NiklasRhf/yaydl/releases/tag/yaydl-v";

#[tauri::command]
pub async fn get_app_info<R: Runtime>(app: AppHandle<R>) -> Result<AppInfo, YaydlError> {
    Ok(AppInfo {
        version: app.package_info().version.to_string(),
    })
}

#[tauri::command]
pub async fn open_release_notes<R: Runtime>(app: AppHandle<R>) -> Result<(), YaydlError> {
    let url = format!("{RELEASE_TAG_URL}{}", app.package_info().version);
    app.opener().open_url(&url, None::<&str>).map_err(|e| {
        error!(%url, error = %e, "opening the release notes failed");
        YaydlError::Open(format!("{url}: {e}"))
    })
}
