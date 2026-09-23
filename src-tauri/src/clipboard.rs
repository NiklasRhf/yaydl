//! On Linux the clipboard plugin (arboard) needs the wlr data-control protocol,
//! which KWin and Mutter do not offer, and its X11 fallback only sees the
//! clipboard while an Xwayland window has focus. GTK uses the regular clipboard
//! of the focused window, like any other app, so Linux goes through GTK.

use tauri::{AppHandle, Runtime};
use tracing::warn;
use yaydl_shared::YaydlError;

#[cfg(target_os = "linux")]
pub async fn read_text<R: Runtime>(app: &AppHandle<R>) -> Result<String, YaydlError> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<String>>();
    app.run_on_main_thread(move || {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.request_text(move |_, text| {
            if tx.send(text.map(str::to_string)).is_err() {
                warn!("the clipboard text arrived after the reader gave up");
            }
        });
    })
    .map_err(|e| read_error(format!("scheduling on the main thread failed: {e}")))?;
    rx.await
        .map_err(|_| read_error("GTK dropped the clipboard request".to_string()))?
        .ok_or_else(|| read_error("the clipboard holds no text".to_string()))
}

#[cfg(not(target_os = "linux"))]
pub async fn read_text<R: Runtime>(app: &AppHandle<R>) -> Result<String, YaydlError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .read_text()
        .map_err(|e| read_error(e.to_string()))
}

#[cfg(target_os = "linux")]
pub async fn write_text<R: Runtime>(app: &AppHandle<R>, text: String) -> Result<(), YaydlError> {
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    app.run_on_main_thread(move || {
        let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
        clipboard.set_text(&text);
        // Lets the text outlive yaydl if a clipboard manager is running.
        clipboard.store();
        if tx.send(()).is_err() {
            warn!("the clipboard write finished after the writer gave up");
        }
    })
    .map_err(|e| write_error(format!("scheduling on the main thread failed: {e}")))?;
    rx.await
        .map_err(|_| write_error("the main thread dropped the clipboard write".to_string()))
}

#[cfg(not(target_os = "linux"))]
pub async fn write_text<R: Runtime>(app: &AppHandle<R>, text: String) -> Result<(), YaydlError> {
    use tauri_plugin_clipboard_manager::ClipboardExt;
    app.clipboard()
        .write_text(text)
        .map_err(|e| write_error(e.to_string()))
}

fn read_error(reason: String) -> YaydlError {
    warn!(%reason, "reading the clipboard failed");
    YaydlError::ClipboardRead(reason)
}

fn write_error(reason: String) -> YaydlError {
    warn!(%reason, "writing the clipboard failed");
    YaydlError::ClipboardWrite(reason)
}
