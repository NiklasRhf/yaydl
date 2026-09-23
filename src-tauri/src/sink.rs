use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;
use tracing::{debug, error, info, warn};
use yaydl_shared::{events, DownloadItem, Notice};

use crate::{notices::Notices, queue::Sink, settings::SettingsStore};

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
        if !self.settings.get().notify_on_finish {
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
        let body = batch_summary(finished, failed);
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

pub fn batch_summary(finished: u32, failed: u32) -> String {
    let downloads = |n: u32| if n == 1 { "download" } else { "downloads" };
    match (finished, failed) {
        (f, 0) => format!("{f} {} finished", downloads(f)),
        (0, x) => format!("{x} {} failed", downloads(x)),
        (f, x) => format!("{f} finished, {x} failed"),
    }
}

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_summaries() {
        assert_eq!(batch_summary(1, 0), "1 download finished");
        assert_eq!(batch_summary(3, 0), "3 downloads finished");
        assert_eq!(batch_summary(0, 2), "2 downloads failed");
        assert_eq!(batch_summary(2, 1), "2 finished, 1 failed");
    }
}
