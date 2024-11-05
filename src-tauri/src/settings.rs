use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::Mutex,
};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_dialog::DialogExt;

use crate::AppData;
use yaydl_shared::Settings;

pub trait Setup {
    fn setup_settings(config_dir: &PathBuf) -> Self;
    fn with_defaults() -> Self;
}

impl Setup for Settings {
    fn setup_settings(config_dir: &PathBuf) -> Self {
        fs::create_dir_all(config_dir).unwrap();
        let config_file = config_dir.join("settings.toml");
        if let Ok(mut file) = File::open(&config_file) {
            let mut buffer = String::new();
            file.read_to_string(&mut buffer).expect("to read to string");
            let settings: Settings = toml::from_str(&buffer).expect("to read toml file");
            settings
        } else {
            let mut file = File::create(&config_file).expect("to create file");
            let settings = Self::with_defaults();
            file.write_all(toml::to_string(&settings).expect("to serialize").as_bytes())
                .expect("to write to file");
            settings
        }
    }

    fn with_defaults() -> Self {
        Self {
            output_dir: dirs::audio_dir().unwrap(),
            output_format: String::from("mp3"),
            dark_theme: true,
        }
    }
}

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, Mutex<AppData>>) -> Settings {
    state.lock().unwrap().settings.clone()
}

#[tauri::command]
pub async fn choose_output_dir<R: Runtime>(
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> Result<String, String> {
    let Some(path) = app_handle.dialog().file().blocking_pick_folder() else {
        return Err("No folder selected".to_string());
    };
    path.as_path()
        .unwrap()
        .clone_into(&mut state.lock().unwrap().settings.output_dir);
    update_settings(&app_handle, &state);

    Ok(path.to_string())
}

#[tauri::command]
pub fn set_output_format<R: Runtime>(
    value: &str,
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> bool {
    state.lock().unwrap().settings.output_format = value.into();
    update_settings(&app_handle, &state)
}

#[tauri::command]
pub fn set_dark_theme<R: Runtime>(
    value: bool,
    app_handle: AppHandle<R>,
    state: tauri::State<'_, Mutex<AppData>>,
) -> bool {
    state.lock().unwrap().settings.dark_theme = value;
    update_settings(&app_handle, &state)
}

fn update_settings<R: Runtime>(
    app_handle: &AppHandle<R>,
    state: &tauri::State<'_, Mutex<AppData>>,
) -> bool {
    let settings_path = app_handle
        .path()
        .app_config_dir()
        .unwrap()
        .join("settings.toml");
    let mut file = OpenOptions::new().write(true).open(&settings_path).unwrap();
    let settings = &state.inner().lock().unwrap().settings;
    let serialized = toml::to_string(&settings).unwrap();
    file.set_len(0).unwrap();
    file.write_all(serialized.as_bytes()).unwrap();
    true
}
