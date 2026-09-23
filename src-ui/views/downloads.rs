use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use wasm_bindgen::JsCast;
use yaydl_shared::{AddUrlsArgs, AddUrlsResult};

use crate::format;
use crate::i18n::{texts_now, use_texts, Text, Texts};
use crate::ipc::{call, call0, fire0, log_to_backend};
use crate::state::{use_app, AppState};
use crate::views::item::ItemRow;

const MAX_INVALID_SHOWN: usize = 3;

pub fn report_added(state: AppState, result: &AddUrlsResult) {
    let toasts = state.toasts;
    let t = texts_now(state);
    if !result.added.is_empty() {
        toasts.success((t.added)(result.added.len() as u64));
    }
    if !result.already_queued.is_empty() {
        toasts.info((t.already_queued)(result.already_queued.len() as u64));
    }
    if !result.invalid.is_empty() {
        let shown: Vec<&str> = result
            .invalid
            .iter()
            .take(MAX_INVALID_SHOWN)
            .map(String::as_str)
            .collect();
        let rest = result.invalid.len().saturating_sub(MAX_INVALID_SHOWN);
        toasts.warning((t.not_a_link)(&shown.join(", "), rest as u64));
    }
    if result.added.is_empty() && result.already_queued.is_empty() && result.invalid.is_empty() {
        toasts.info(t.no_links_found);
    }
}

/// Resolves to `true` when the text should be cleared from the input, which
/// is whenever something besides invalid tokens came back.
async fn add_text(state: AppState, text: String) -> bool {
    match call::<_, AddUrlsResult>("add_urls", &AddUrlsArgs { text }).await {
        Ok(result) => {
            report_added(state, &result);
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
fn dropped_text(t: &Texts, data: &web_sys::DataTransfer) -> Result<String, String> {
    let uri_list = data
        .get_data("text/uri-list")
        .map_err(|e| (t.drop_links_read_failed)(&format!("{e:?}")))?;
    let links: Vec<&str> = uri_list
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if !links.is_empty() {
        return Ok(links.join("\n"));
    }
    data.get_data("text/plain")
        .map_err(|e| (t.drop_text_read_failed)(&format!("{e:?}")))
}

#[component]
pub fn DownloadsPage() -> impl IntoView {
    let state = use_app();
    let toasts = state.toasts;
    let queue = state.queue;
    let t = use_texts();
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
            toasts.warning(texts_now(state).paste_unreadable);
            return;
        };
        let text = match data.get_data("text/plain") {
            Ok(text) => text,
            Err(e) => {
                toasts.error((texts_now(state).paste_read_failed)(&format!("{e:?}")));
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
                Ok(result) => report_added(state, &result),
                Err(e) => toasts.error(e),
            }
        });
    };

    let on_drop = move |ev: ev::DragEvent| {
        ev.prevent_default();
        drag_depth.set(0);
        let Some(data) = ev.data_transfer() else {
            log_to_backend("warn", "a drop event carried no data".to_string());
            toasts.warning(texts_now(state).drop_unreadable);
            return;
        };
        match dropped_text(texts_now(state), &data) {
            Ok(text) if text.trim().is_empty() => toasts.info(texts_now(state).drop_nothing),
            Ok(text) => spawn_local(async move {
                add_text(state, text).await;
            }),
            Err(e) => toasts.error(e),
        }
    };

    let summary_text = move || {
        let s = summary.get();
        let t = t();
        let parts: Vec<String> = [
            s.resolving,
            s.awaiting,
            s.ready,
            s.downloading,
            s.queued,
            s.finished,
            s.failed,
            s.cancelled,
        ]
        .into_iter()
        .zip(t.summary)
        .filter(|(n, _)| *n > 0)
        .map(|(n, (one, many))| {
            let label = if n == 1 { one } else { many };
            format!("{} {label}", format::count(u64::from(n), t.locale))
        })
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
            <h1 class="text-2xl font-semibold tracking-tight">{move || t().downloads_title}</h1>
            <div class="card flex flex-wrap items-center gap-2 p-3">
                <label for="add-links" class="sr-only">
                    {move || t().add_links_label}
                </label>
                <input
                    id="add-links"
                    class="input min-w-[16rem] flex-1"
                    type="text"
                    placeholder=move || t().add_links_placeholder
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
                    {move || t().add}
                </button>
                <button class="btn btn-secondary" on:click=from_clipboard>
                    <Icon icon=icondata::LuClipboardPaste />
                    {move || t().paste_from_clipboard}
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
                    {move || t().download_all}
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
                    {move || t().clear_finished}
                </button>
                <ConfirmButton
                    label=|t| t.clear_all
                    confirm_label=|t| t.clear_all_confirm
                    icon=icondata::LuTrash2
                    on_confirm=Callback::new(move |()| {
                        spawn_local(async move {
                            match call0::<u32>("clear_all").await {
                                Ok(0) => {}
                                Ok(kept) => toasts.info((texts_now(state).kept_running)(u64::from(kept))),
                                Err(e) => toasts.error(e),
                            }
                        })
                    })
                />
                <button class="btn btn-secondary" on:click=move |_| fire0(toasts, "open_output_dir")>
                    <Icon icon=icondata::LuFolderOpen />
                    {move || t().open_folder}
                </button>
                <p class="ml-auto text-sm text-zinc-500 dark:text-zinc-400" aria-live="polite">
                    {summary_text}
                </p>
            </div>

            <Show
                when=move || queue.order.with(|o| !o.is_empty())
                fallback=move || {
                    view! {
                        <div class="flex flex-1 flex-col items-center justify-center gap-3 rounded-xl border-2 border-dashed border-zinc-300 p-10 text-center text-zinc-500 dark:border-zinc-700 dark:text-zinc-400">
                            <span class="text-4xl" aria-hidden="true">
                                <Icon icon=icondata::LuLink />
                            </span>
                            <p>{move || t().queue_empty}</p>
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
                        {move || t().drop_overlay}
                    </span>
                </div>
            </Show>
        </div>
    }
}

/// A destructive button that needs a second click within a few seconds.
#[component]
pub fn ConfirmButton(
    label: Text,
    confirm_label: Text,
    icon: icondata::Icon,
    on_confirm: Callback<()>,
) -> impl IntoView {
    let t = use_texts();
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
            {move || if armed.get() { confirm_label(t()) } else { label(t()) }}
        </button>
    }
}
