use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;
use tracing::{debug, error, info, warn};
use yaydl_shared::{events, DownloadItem, Notice};

use crate::{i18n, notices::Notices, queue::Sink, settings::SettingsStore};

const MAIN_WINDOW: &str = "main";

pub struct TauriSink<R: Runtime> {
    pub app: AppHandle<R>,
    pub notices: Arc<Notices>,
    pub settings: Arc<SettingsStore>,
}

impl<R: Runtime> TauriSink<R> {
    fn emit<T: serde::Serialize + Clone>(&self, event: &str, payload: T) {
        if let Err(e) = self.app.emit(event, payload) {
            error!(event, error = %e, "emitting an event failed");
        }
    }

    fn main_window_focused(&self) -> bool {
        let Some(window) = self.app.get_webview_window(MAIN_WINDOW) else {
            warn!(
                label = MAIN_WINDOW,
                "the main window does not exist, treating it as unfocused"
            );
            return false;
        };
        match window.is_focused() {
            Ok(focused) => focused,
            Err(e) => {
                error!(error = %e, "reading the main window focus failed, treating it as unfocused");
                false
            }
        }
    }
}

impl<R: Runtime> Sink for TauriSink<R> {
    fn queue_replaced(&self, items: &[DownloadItem]) {
        self.emit(events::QUEUE_REPLACED, items);
    }

    fn item_updated(&self, item: &DownloadItem) {
        self.emit(events::QUEUE_ITEM_UPDATED, item);
    }

    fn history_changed(&self) {
        self.emit(events::HISTORY_CHANGED, ());
    }

    fn notice(&self, notice: Notice) {
        self.notices.notify(&self.app, notice);
    }

    fn batch_finished(&self, finished: u32, failed: u32) {
        let settings = self.settings.get();
        if !settings.notify_on_finish {
            debug!(finished, failed, "batch finished, notifications are off");
            return;
        }
        if self.main_window_focused() {
            debug!(
                finished,
                failed, "batch finished while the window is focused, no notification"
            );
            return;
        }
        let body = (i18n::texts_for(&settings).batch_summary)(finished, failed);
        info!(%body, "sending the batch notification");
        if let Err(e) = self
            .app
            .notification()
            .builder()
            .title("yaydl")
            .body(body)
            .show()
        {
            error!(error = %e, finished, failed, "showing the batch notification failed");
        }
    }
}
