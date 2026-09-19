use crate::app::{command_error_text, invoke, log_to_backend, notify_error};
use crate::notification::{Notification, NotificationContext, NotificationType};
use leptos::*;
use wasm_bindgen_futures::spawn_local;
use yaydl_shared::{ClipboardArgs, LogSnapshot, RecentLogsArgs};

const LOG_LINES: usize = 300;

#[component]
pub fn LogViewer(on_close: Callback<()>) -> impl IntoView {
    let notifications = use_context::<NotificationContext>().unwrap();
    let snapshot = create_rw_signal(None::<LogSnapshot>);

    let load = {
        let notifications = notifications.clone();
        move || {
            let notifications = notifications.clone();
            spawn_local(async move {
                let args = serde_wasm_bindgen::to_value(&RecentLogsArgs {
                    max_lines: LOG_LINES,
                })
                .expect("recent-log arguments to serialize");
                match invoke("get_recent_logs", args).await {
                    Ok(js_val) => {
                        match serde_wasm_bindgen::from_value::<LogSnapshot>(js_val.clone()) {
                            Ok(loaded) => snapshot.set(Some(loaded)),
                            Err(e) => notify_error(
                                &notifications,
                                format!("Decoding the log snapshot failed: {e} ({js_val:?})"),
                            ),
                        }
                    }
                    Err(js_val) => notify_error(&notifications, command_error_text(&js_val)),
                }
            });
        }
    };

    create_effect({
        let load = load.clone();
        move |_| load()
    });

    let refresh = {
        let load = load.clone();
        move |_| load()
    };

    let copy = {
        let notifications = notifications.clone();
        move |_| {
            let Some(loaded) = snapshot.get_untracked() else {
                return;
            };
            let notifications = notifications.clone();
            spawn_local(async move {
                let text = format!(
                    "yaydl {} logs from {}\n{}",
                    loaded.app_version,
                    loaded.path,
                    loaded.lines.join("\n")
                );
                let args = serde_wasm_bindgen::to_value(&ClipboardArgs { text })
                    .expect("clipboard arguments to serialize");
                match invoke("copy_to_clipboard", args).await {
                    Ok(_) => {
                        log_to_backend("info", "logs copied to the clipboard".to_string());
                        notifications.add_notification(Notification {
                            text: "Logs copied to the clipboard".into(),
                            notification_type: NotificationType::Success,
                        });
                    }
                    Err(js_val) => {
                        let text = command_error_text(&js_val);
                        log_to_backend("info", format!("copying logs failed: {text}"));
                        notify_error(&notifications, text);
                    }
                }
            });
        }
    };

    let path = move || {
        snapshot
            .get()
            .map(|s| s.path)
            .unwrap_or_else(|| "loading".to_string())
    };
    let body = move || {
        snapshot
            .get()
            .map(|s| s.lines.join("\n"))
            .unwrap_or_default()
    };
    let truncated = move || snapshot.get().is_some_and(|s| s.truncated);

    view! {
        <div class="mt-2 border-2 border-gray-400 rounded-md p-2 bg-gray-100">
            <div class="flex space-x-1 items-center">
                <button
                    on:click=refresh
                    class="border-2 border-gray-500 h-8 px-2 rounded-md bg-gray-400 hover:bg-gray-500 shadow-md"
                >
                    "Refresh"
                </button>
                <button
                    on:click=copy
                    class="border-2 border-gray-500 h-8 px-2 rounded-md bg-gray-400 hover:bg-gray-500 shadow-md"
                >
                    "Copy to clipboard"
                </button>
                <button
                    on:click=move |_| on_close.call(())
                    class="border-2 border-gray-500 h-8 px-2 rounded-md bg-gray-400 hover:bg-gray-500 shadow-md"
                >
                    "Close"
                </button>
                <p class="text-xs font-mono truncate">{path}</p>
            </div>
            <Show when=truncated>
                <p class="text-xs mt-1">{format!("showing the last {LOG_LINES} lines")}</p>
            </Show>
            <pre class="mt-2 font-mono max-h-96 overflow-auto whitespace-pre text-xs bg-white p-2 rounded">
                {body}
            </pre>
        </div>
    }
}
