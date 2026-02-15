use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_shell::ShellExt;

mod settings;
use settings::Setup;
use tauri_plugin_updater::UpdaterExt;
use yaydl_shared::{
    AddLinkError, Download, DownloadEvent, DownloadState, Metadata, MetadataError, Settings, UpdateError, YaydlError
};

type Result<T> = std::result::Result<T, YaydlError>;

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
        .download_list.clear();
}

#[tauri::command]
async fn update_download<R: Runtime>(app_handle: tauri::AppHandle<R>, id: String, state: DownloadState) {
    if let Some(download) = app_handle
        .state::<Mutex<AppData>>()
        .lock()
        .unwrap()
        .download_list.iter_mut().find(|d| d.metadata.id == id) {
        download.download_state = state;
    }
}

#[tauri::command]
async fn try_add<R: Runtime>(app_handle: AppHandle<R>) -> Result<(String, Vec<Download>)> {
    let content = app_handle.clipboard().read_text();
    let state = app_handle.state::<Mutex<AppData>>();
    match content {
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
    }
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
async fn retreive_metadata<R: Runtime>(url: &str, app_handle: AppHandle<R>) -> Result<Metadata> {
    let shell = app_handle.shell();
    let output = shell
        .sidecar("yt-dlp")
        .map_err(|e| YaydlError::TauriShellError(e.to_string()))?
        .args([
            "--verbose",
            "--get-id",
            "--get-title",
            "--get-duration",
            "--get-thumbnail",
            url,
        ])
        .output()
        .await
        .map_err(|e| YaydlError::TauriShellError(e.to_string()))?;

    if !output.status.success() {
        return Err(YaydlError::MetadataError(MetadataError::RetreivalFailed));
    }

    let output_str = std::str::from_utf8(&output.stdout).map_err(|_| YaydlError::Utf8Conversion)?;
    let metadata: Vec<&str> = output_str.split("\n\n").collect();

    if metadata.len() < 4 {
        return Err(YaydlError::MetadataError(MetadataError::MissingFields));
    }

    let metadata = Metadata {
        title: metadata[0].to_string(),
        id: metadata[1].to_string(),
        thumbnail: metadata[2].to_string(),
        duration: metadata[3].to_string(),
        url: url.to_string(),
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

    Ok(metadata)
}

#[tauri::command]
async fn execute_yt_dl<R: Runtime>(
    url: String,
    id: String,
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<()> {
    let shell = app_handle.shell();
    let output_dir = state
        .lock()
        .unwrap()
        .settings
        .output_dir
        .display()
        .to_string();
    let output_format = state.lock().unwrap().settings.output_format.to_string();

    let (mut rx, _) = shell
        .sidecar("yt-dlp")
        .map_err(|e| YaydlError::TauriShellError(e.to_string()))?
        .args([
            "--newline",
            "-x",
            "--audio-format",
            &output_format,
            "-o",
            &format!("{output_dir}/%(title)s.%(ext)s"),
            &url,
        ])
        .spawn()
        .map_err(|e| YaydlError::TauriShellError(e.to_string()))?;

    while let Some(event) = rx.recv().await {
        if let tauri_plugin_shell::process::CommandEvent::Stdout(line) = event {
            let line = std::str::from_utf8(&line).map_err(|_| YaydlError::Utf8Conversion)?;
            if line.starts_with("[download]") {
                let (_, remainder) = line.split_at("[download]".len());
                let remainder = remainder.trim_start();
                let percent = remainder.split(' ').collect::<Vec<_>>()[0];
                let percent = &percent[..percent.len() - 1];
                if let Ok(progress) = percent.parse::<f32>() {
                    let progress = progress as u8;
                    app_handle
                        .emit(
                            "download-progress",
                            DownloadEvent {
                                id: id.clone(),
                                progress,
                            },
                        )
                        .unwrap();
                }
            }
        }
    }

    Ok(())
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
        .map_err(|_| YaydlError::UpdateError(UpdateError::BuildFailed))?
        .check()
        .await
        .map_err(|_| YaydlError::UpdateError(UpdateError::CheckFailed))?;
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
                    total = content_length.unwrap_or(0) as u64;
                    let percent = if total > 0 { (downloaded * 100 / total) as u8 } else { 0 };
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = check_update(handle).await {
                    println!("{}", e);
                }
            });
            let config_dir = app.path().app_config_dir().unwrap();
            let app_data = AppData {
                settings: Settings::setup_settings(&config_dir),
                ..Default::default()
            };
            app.manage(Mutex::new(app_data));
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
            settings::choose_output_dir,
            settings::set_output_format,
            settings::set_dark_theme,
            settings::get_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
