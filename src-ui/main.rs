mod app;
mod format;
mod i18n;
mod ipc;
mod log_viewer;
mod queue;
mod state;
mod theme;
mod toast;
mod update_banner;
mod views;

use app::App;
use ipc::log_to_backend;

fn main() {
    // A wasm panic otherwise only reaches the devtools console, which the user
    // running the packaged app has no way to open.
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        log_to_backend("error", format!("webview panicked: {info}"));
    }));
    leptos::mount::mount_to_body(App);
}
