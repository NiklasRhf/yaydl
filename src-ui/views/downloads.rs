use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;
use yaydl_shared::{AddUrlsArgs, AddUrlsResult};

use crate::format;
use crate::ipc::{call, call0, fire0, log_to_backend};
use crate::state::{use_app, AppState};
use crate::toast::Toasts;
use crate::views::item::ItemRow;

const MAX_INVALID_SHOWN: usize = 3;

pub fn report_added(toasts: Toasts, result: &AddUrlsResult) {
    if !result.added.is_empty() {
        toasts.success(format!("Added {}", result.added.len()));
    }
    if !result.already_queued.is_empty() {
        toasts.info(format!(
            "{} already in the queue",
            result.already_queued.len()
        ));
    }
    if !result.invalid.is_empty() {
        let shown: Vec<&str> = result
            .invalid
            .iter()
            .take(MAX_INVALID_SHOWN)
            .map(String::as_str)
            .collect();
        let rest = result.invalid.len().saturating_sub(MAX_INVALID_SHOWN);
        let more = if rest > 0 {
            format!(" and {rest} more")
        } else {
            String::new()
        };
        toasts.warning(format!("Not a link: {}{more}", shown.join(", ")));
    }
    if result.added.is_empty() && result.already_queued.is_empty() && result.invalid.is_empty() {
        toasts.info("No links found");
    }
}

/// Resolves to `true` when the text should be cleared from the input, which
/// is whenever something besides invalid tokens came back.
async fn add_text(state: AppState, text: String) -> bool {
    match call::<_, AddUrlsResult>("add_urls", &AddUrlsArgs { text }).await {
        Ok(result) => {
            report_added(state.toasts, &result);
            !result.added.is_empty() || !result.already_queued.is_empty()
        }
        Err(e) => {
            state.toasts.error(e);
            false
        }
    }
}

fn focus_in_field() -> bool {
    document().active_element().is_some_and(|el| {
        matches!(
            el.tag_name().to_ascii_lowercase().as_str(),
            "input" | "textarea" | "select"
        ) || el
            .dyn_ref::<web_sys::HtmlElement>()
            .is_some_and(|el| el.is_content_editable())
    })
}

/// Prefers `text/uri-list`, whose `#` lines are comments, over `text/plain`.
fn dropped_text(data: &web_sys::DataTransfer) -> Result<String, String> {
    let uri_list = data
        .get_data("text/uri-list")
        .map_err(|e| format!("reading the dropped links failed: {e:?}"))?;
    let links: Vec<&str> = uri_list
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if !links.is_empty() {
        return Ok(links.join("\n"));
    }
    data.get_data("text/plain")
        .map_err(|e| format!("reading the dropped text failed: {e:?}"))
}

#[component]
pub fn DownloadsPage() -> impl IntoView {
    let state = use_app();
    let toasts = state.toasts;
    let queue = state.queue;
    let input = RwSignal::new(String::new());
    let adding = RwSignal::new(false);
    let drag_depth = RwSignal::new(0i32);
    let summary = Memo::new(move |_| queue.summary());

    let paste_listener = window_event_listener(ev::paste, move |ev| {
        if focus_in_field() {
            return;
        }
        let Some(data) = ev.clipboard_data() else {
            log_to_backend(
                "warn",
                "a paste event carried no clipboard data".to_string(),
            );
            toasts.warning("The pasted content could not be read");
            return;
        };
        let text = match data.get_data("text/plain") {
            Ok(text) => text,
            Err(e) => {
                toasts.error(format!("Reading the pasted text failed: {e:?}"));
                return;
            }
        };
        if text.trim().is_empty() {
            return;
        }
        ev.prevent_default();
        spawn_local(async move {
            add_text(state, text).await;
        });
    });
    on_cleanup(move || paste_listener.remove());

    let submit = move || {
        let text = input.get_untracked();
        if text.trim().is_empty() || adding.get_untracked() {
            return;
        }
        adding.set(true);
        spawn_local(async move {
            if add_text(state, text).await {
                input.set(String::new());
            }
            adding.set(false);
        });
    };
    let from_clipboard = move |_| {
        spawn_local(async move {
            match call0::<AddUrlsResult>("add_from_clipboard").await {
                Ok(result) => report_added(toasts, &result),
                Err(e) => toasts.error(e),
            }
        });
    };

    let on_drop = move |ev: ev::DragEvent| {
        ev.prevent_default();
        drag_depth.set(0);
        let Some(data) = ev.data_transfer() else {
            log_to_backend("warn", "a drop event carried no data".to_string());
            toasts.warning("The dropped content could not be read");
            return;
        };
        match dropped_text(&data) {
            Ok(text) if text.trim().is_empty() => toasts
                .info("Nothing to add. Drop a link from your browser's address bar or a page."),
            Ok(text) => spawn_local(async move {
                add_text(state, text).await;
            }),
            Err(e) => toasts.error(e),
        }
    };

    let summary_text = move || {
        let s = summary.get();
        let parts: Vec<String> = [
            (s.resolving, "fetching info"),
            (s.awaiting, "awaiting confirmation"),
            (s.ready, "ready"),
            (s.downloading, "downloading"),
            (s.queued, "queued"),
            (s.finished, "finished"),
            (s.failed, "failed"),
            (s.cancelled, "cancelled"),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{} {label}", format::count(u64::from(n))))
        .collect();
        parts.join(" \u{b7} ")
    };

    view! {
        <div
            class="relative flex min-h-full flex-col gap-4 p-6"
            on:dragenter=move |ev: ev::DragEvent| {
                ev.prevent_default();
                drag_depth.update(|d| *d += 1);
            }
            on:dragover=move |ev: ev::DragEvent| {
                ev.prevent_default();
                if let Some(data) = ev.data_transfer() {
                    data.set_drop_effect("copy");
                }
            }
            on:dragleave=move |_| drag_depth.update(|d| *d = (*d - 1).max(0))
            on:drop=on_drop
        >
            <h1 class="text-2xl font-semibold tracking-tight">"Downloads"</h1>
            <div class="card flex flex-wrap items-center gap-2 p-3">
                <label for="add-links" class="sr-only">
                    "Links to add"
                </label>
                <input
                    id="add-links"
                    class="input min-w-[16rem] flex-1"
                    type="text"
                    placeholder="Paste links from YouTube or 1000+ other sites"
                    autocomplete="off"
                    spellcheck="false"
                    bind:value=input
                    on:keydown=move |ev: ev::KeyboardEvent| {
                        if ev.key() == "Enter" {
                            ev.prevent_default();
                            submit();
                        }
                    }
                />
                <button
                    class="btn btn-primary"
                    disabled=move || adding.get() || input.with(|t| t.trim().is_empty())
                    on:click=move |_| submit()
                >
                    <Icon icon=icondata::LuPlus />
                    "Add"
                </button>
                <button class="btn btn-secondary" on:click=from_clipboard>
                    <Icon icon=icondata::LuClipboardPaste />
                    "Paste from clipboard"
                </button>
            </div>

            <div class="flex flex-wrap items-center gap-2">
                <button
                    class="btn btn-secondary"
                    disabled=move || summary.with(|s| s.ready == 0)
                    on:click=move |_| {
                        spawn_local(async move {
                            if let Err(e) = call0::<u32>("start_all").await {
                                toasts.error(e);
                            }
                        })
                    }
                >
                    <Icon icon=icondata::LuDownload />
                    "Download all"
                </button>
                <button
                    class="btn btn-secondary"
                    on:click=move |_| {
                        spawn_local(async move {
                            if let Err(e) = call0::<u32>("clear_finished").await {
                                toasts.error(e);
                            }
                        })
                    }
                >
                    <Icon icon=icondata::LuListX />
                    "Clear finished"
                </button>
                <ConfirmButton
                    label="Clear all"
                    confirm_label="Click again to clear all"
                    icon=icondata::LuTrash2
                    on_confirm=Callback::new(move |()| {
                        spawn_local(async move {
                            match call0::<u32>("clear_all").await {
                                Ok(0) => {}
                                Ok(kept) => {
                                    toasts
                                        .info(
                                            format!(
                                                "Kept {} that {} still running",
                                                format::plural(u64::from(kept), "download", "downloads"),
                                                if kept == 1 { "is" } else { "are" },
                                            ),
                                        )
                                }
                                Err(e) => toasts.error(e),
                            }
                        })
                    })
                />
                <button class="btn btn-secondary" on:click=move |_| fire0(toasts, "open_output_dir")>
                    <Icon icon=icondata::LuFolderOpen />
                    "Open folder"
                </button>
                <p class="ml-auto text-sm text-zinc-500 dark:text-zinc-400" aria-live="polite">
                    {summary_text}
                </p>
            </div>

            <Show
                when=move || queue.order.with(|o| !o.is_empty())
                fallback=|| {
                    view! {
                        <div class="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border-2 border-dashed border-zinc-300 p-10 text-center text-zinc-500 dark:border-zinc-700 dark:text-zinc-400">
                            <span class="text-4xl" aria-hidden="true">
                                <Icon icon=icondata::LuLink />
                            </span>
                            <p>"Paste a link with Ctrl+V, drop it here, or type it above."</p>
                        </div>
                    }
                }
            >
                <ul class="flex flex-col gap-2">
                    <For
                        each=move || queue.order.get()
                        key=|id| *id
                        children=move |id| match queue.item(id) {
                            Some(item) => view! { <ItemRow item=Signal::from(item) /> }.into_any(),
                            None => {
                                let message = format!("download {id} is in the queue order but has no data");
                                log_to_backend("error", message.clone());
                                view! { <li class="card p-3 text-sm text-red-700 dark:text-red-400">{message}</li> }
                                    .into_any()
                            }
                        }
                    />
                </ul>
            </Show>

            <Show when=move || { drag_depth.get() > 0 }>
                <div class="pointer-events-none absolute inset-2 z-40 flex items-center justify-center rounded-xl border-2 border-dashed border-blue-500 bg-blue-50/90 text-lg font-medium text-blue-800 dark:bg-blue-950/90 dark:text-blue-100">
                    <span class="flex items-center gap-2">
                        <Icon icon=icondata::LuLink />
                        "Drop links to add them"
                    </span>
                </div>
            </Show>
        </div>
    }
}

/// A destructive button that needs a second click within a few seconds.
#[component]
pub fn ConfirmButton(
    label: &'static str,
    confirm_label: &'static str,
    icon: icondata::Icon,
    on_confirm: Callback<()>,
) -> impl IntoView {
    let armed = RwSignal::new(false);
    let generation = StoredValue::new(0u64);
    let click = move |_| {
        if armed.get_untracked() {
            armed.set(false);
            on_confirm.run(());
            return;
        }
        armed.set(true);
        let armed_at = generation.get_value() + 1;
        generation.set_value(armed_at);
        // Only the timer of the latest arming may disarm, and `try_get_value`
        // because the button can be gone by the time it fires.
        if let Err(e) = set_timeout_with_handle(
            move || {
                if generation.try_get_value() == Some(armed_at) {
                    armed.set(false);
                }
            },
            std::time::Duration::from_secs(4),
        ) {
            log_to_backend(
                "warn",
                format!("scheduling the confirm reset failed: {e:?}"),
            );
        }
    };
    view! {
        <button
            class=move || if armed.get() { "btn btn-danger" } else { "btn btn-secondary" }
            on:click=click
            on:blur=move |_| armed.set(false)
        >
            <Icon icon=icon />
            {move || if armed.get() { confirm_label } else { label }}
        </button>
    }
}
