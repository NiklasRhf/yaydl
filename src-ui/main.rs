mod app;
mod notification;
mod update_modal;
mod update_context;

use app::App;
use leptos::{mount_to_body, view};

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| {
        view! {
            <App/>
        }
    });
}
