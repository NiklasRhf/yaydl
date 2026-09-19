mod app;
mod log_viewer;
mod notification;
mod update_context;
mod update_modal;

use app::{log_to_backend, App};
use leptos::{mount_to_body, view};

fn main() {
    // A wasm panic otherwise only reaches the devtools console, which the user
    // running the packaged app has no way to open.
    std::panic::set_hook(Box::new(|info| {
        console_error_panic_hook::hook(info);
        log_to_backend("error", format!("webview panicked: {info}"));
    }));
    mount_to_body(|| {
        view! {
            <App/>
        }
    });
}
