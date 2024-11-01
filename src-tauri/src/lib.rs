use std::sync::Mutex;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_shell::ShellExt;

mod settings;
use settings::Settings;
use tauri_plugin_updater::UpdaterExt;

#[derive(Default)]
pub struct AppData {
    download_list: Vec<String>,
    settings: Settings,
}

#[tauri::command]
async fn try_add<R: Runtime>(app_handle: AppHandle<R>) -> Result<String, ()> {
    let content = app_handle.clipboard().read_text();
    let state = app_handle.state::<Mutex<AppData>>();
    if let Ok(text) = content {
        if text.contains("https://www.youtube.com/") {
            let mut state = state.lock().unwrap();
            let contains = state.download_list.contains(&text);
            if !contains {
                state.download_list.push(text.clone());
            }
            return Ok(text);
        }
    }
    return Err(());
}

#[tauri::command]
fn open_explorer<R: Runtime>(
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<(), String> {
    let shell = app_handle.shell();
    let explorer = if cfg!(target_os = "windows") {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "linux") {
        "xdg-open"
    } else {
        return Err("Unsupported operating system".into());
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
async fn retreive_metadata<R: Runtime>(url: &str, app_handle: AppHandle<R>) -> Result<String, ()> {
    let shell = app_handle.shell();
    let output = shell
        .sidecar("yt-dlp")
        .unwrap()
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
        .unwrap();

    let output = std::str::from_utf8(&output.stdout).unwrap();
    Ok(output.to_string())
}

#[tauri::command]
async fn execute_yt_dl<R: Runtime>(
    url: String,
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<(), String> {
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
        .unwrap()
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
        .unwrap();

    while let Some(event) = rx.recv().await {
        if let tauri_plugin_shell::process::CommandEvent::Stdout(line) = event {
            let line = std::str::from_utf8(&line).unwrap();
            if line.starts_with("[download]") {
                let (_, remainder) = line.split_at("[download]".len());
                let remainder = remainder.trim_start();
                let percent = remainder.split(' ').collect::<Vec<_>>()[0];
                let _percent = &percent[..percent.len() - 1];
            }
        }
    }

    Ok(())
}

async fn update(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(update) = app
        .updater_builder()
        .on_before_exit(|| {
            println!("app is about to exit on Windows!");
        })
        .build()
        .map_err(|_| String::from("Couldn't build updater"))?
        .check()
        .await
        .map_err(|_| String::from("Couldn't check updater"))?
    {
        let mut downloaded = 0;

        update
            .download_and_install(
                |chunk_length, content_length| {
                    downloaded += chunk_length;
                    println!("downloaded {downloaded} from {content_length:?}");
                },
                || {
                    println!("download finished");
                },
            )
            .await
            .map_err(|_| String::from("Updater couldn't download and install"))?;

        println!("update installed");
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
                if let Err(e) = update(handle).await {
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
            settings::choose_output_dir,
            settings::set_output_format,
            settings::set_dark_theme,
            settings::get_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
