use std::sync::{Arc, Mutex};

use tauri::{App, Manager};
use tracing::{info, warn};
use yaydl_shared::NoticeLevel;

pub mod app_update;
mod clipboard;
pub mod commands;
pub mod history;
pub mod i18n;
pub mod logging;
pub mod notices;
pub mod persist;
pub mod queue;
pub mod settings;
pub mod sink;
pub mod ytdlp;

use commands::{AppEngine, AppQueue, HistoryState};
use history::{History, HISTORY_FILE};
use notices::{notice, startup_update_notice, Notices};
use persist::quarantine;
use queue::{QueueDeps, QUEUE_FILE};
use settings::{SettingsStore, SETTINGS_FILE};
use sink::TauriSink;
use ytdlp::{ProcessRunner, YtDlp};

type SetupResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn setup(app: &mut App) -> SetupResult {
    let handle = app.handle().clone();
    let config_dir = app.path().app_config_dir()?;
    let data_dir = app.path().app_data_dir()?;

    let notices = Arc::new(Notices::new());
    app.manage(Arc::clone(&notices));

    let loaded = settings::load(
        &config_dir.join(SETTINGS_FILE),
        settings::default_output_dir,
    )?;
    i18n::log_startup_locale(&loaded.settings);
    for n in loaded.notices {
        notices.notify(&handle, n);
    }
    let settings = Arc::new(SettingsStore::new(
        config_dir.join(SETTINGS_FILE),
        loaded.settings,
    ));
    app.manage(Arc::clone(&settings));

    let exe = std::env::current_exe()?;
    let exe_dir = exe
        .parent()
        .ok_or("the executable has no parent directory")?
        .to_path_buf();
    let ytdlp_dir = data_dir.join("yt-dlp");
    let bootstrap = exe_dir.join(format!("yt-dlp{}", std::env::consts::EXE_SUFFIX));
    let ytdlp: Arc<AppEngine> = Arc::new(YtDlp::new(
        ProcessRunner,
        ytdlp_dir,
        bootstrap.clone(),
        exe_dir.clone(),
    ));
    info!(
        version = %app.package_info().version,
        exe = %exe.display(),
        exe_dir = %exe_dir.display(),
        app_data_dir = %data_dir.display(),
        app_config_dir = %config_dir.display(),
        bootstrap_yt_dlp = %bootstrap.display(),
        managed_yt_dlp = %ytdlp.binary().display(),
        "yaydl starting"
    );
    ytdlp.ensure_installed()?;
    app.manage(Arc::clone(&ytdlp));

    let history_path = data_dir.join(HISTORY_FILE);
    let history = match History::load(history_path.clone()) {
        Ok(history) => history,
        Err(e) => {
            warn!(error = %e, "the history file is invalid");
            let moved = quarantine(&history_path)?;
            notices.notify(
                &handle,
                notice(
                    NoticeLevel::Warning,
                    (i18n::texts_for(&settings.get()).history_file_invalid)(&e.message, &moved),
                ),
            );
            History::empty(history_path)
        }
    };
    let history = Arc::new(Mutex::new(history));
    app.manage(HistoryState(Arc::clone(&history)));

    let queue: AppQueue = queue::Queue::restore(QueueDeps {
        engine: Arc::clone(&ytdlp),
        sink: TauriSink {
            app: handle.clone(),
            notices: Arc::clone(&notices),
            settings: Arc::clone(&settings),
        },
        settings: Arc::clone(&settings),
        history,
        path: data_dir.join(QUEUE_FILE),
        runtime: tauri::async_runtime::handle().inner().clone(),
    })?;
    app.manage(queue);

    tauri::async_runtime::spawn(async move {
        let channel = settings.get().yt_dlp_channel;
        let result = ytdlp.maybe_update_on_startup(channel).await;
        info!(%channel, ?result, "yt-dlp startup update check done");
        if let Some(n) = startup_update_notice(&result, i18n::texts_for(&settings.get())) {
            notices.notify(&handle, n);
        }
    });
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let context = tauri::generate_context!();
    let log_dir = logging::log_dir(&context.config().identifier);
    logging::init(&log_dir);
    info!(log_dir = %log_dir.display(), "logging initialised");

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(setup)
        .invoke_handler(tauri::generate_handler![
            commands::get_queue,
            commands::add_urls,
            commands::add_from_clipboard,
            commands::start_download,
            commands::retry_download,
            commands::expand_playlist,
            commands::cancel_download,
            commands::remove_item,
            commands::open_file,
            commands::reveal_file,
            commands::start_all,
            commands::clear_finished,
            commands::clear_all,
            commands::confirm_duplicate,
            commands::set_item_format,
            commands::rename_item,
            commands::open_output_dir,
            commands::get_statistics,
            commands::get_history,
            commands::clear_history,
            commands::get_settings,
            commands::update_settings,
            commands::choose_output_dir,
            commands::get_yt_dlp_status,
            commands::update_yt_dlp,
            commands::take_startup_notices,
            app_update::check_app_update,
            app_update::get_app_info,
            app_update::open_release_notes,
            app_update::install_app_update,
            logging::ui_log,
            logging::get_recent_logs,
            logging::copy_to_clipboard,
        ])
        .run(context)
        .expect("error while running tauri application");
}
