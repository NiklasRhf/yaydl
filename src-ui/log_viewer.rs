use leptos::prelude::*;
use leptos::task::spawn_local;
use yaydl_shared::{ClipboardArgs, LogSnapshot, RecentLogsArgs};

use crate::i18n::{texts_now, use_texts};
use crate::ipc::call;
use crate::state::use_app;

const LOG_LINES: usize = 300;

#[component]
pub fn LogViewer() -> impl IntoView {
    let state = use_app();
    let toasts = state.toasts;
    let t = use_texts();
    let snapshot = RwSignal::new(None::<LogSnapshot>);
    let loading = RwSignal::new(false);

    let load = move || {
        loading.set(true);
        spawn_local(async move {
            match call::<_, LogSnapshot>(
                "get_recent_logs",
                &RecentLogsArgs {
                    max_lines: LOG_LINES,
                },
            )
            .await
            {
                Ok(loaded) => snapshot.set(Some(loaded)),
                Err(e) => toasts.error((texts_now(state).logs_load_failed)(&e)),
            }
            loading.set(false);
        });
    };
    load();

    let copy = move |_| {
        let Some(loaded) = snapshot.get_untracked() else {
            toasts.warning(texts_now(state).logs_not_loaded);
            return;
        };
        spawn_local(async move {
            // Stays English, because it goes into bug reports.
            let text = format!(
                "yaydl {} logs from {}\n{}",
                loaded.app_version,
                loaded.path,
                loaded.lines.join("\n")
            );
            match call::<_, ()>("copy_to_clipboard", &ClipboardArgs { text }).await {
                Ok(()) => toasts.success(texts_now(state).logs_copied),
                Err(e) => toasts.error(e),
            }
        });
    };

    view! {
        <div class="mt-3 rounded-lg border border-zinc-200 bg-zinc-50 p-3 dark:border-zinc-700 dark:bg-zinc-950">
            <div class="flex flex-wrap items-center gap-2">
                <button class="btn btn-secondary" on:click=move |_| load() disabled=move || loading.get()>
                    {move || t().logs_refresh}
                </button>
                <button class="btn btn-secondary" on:click=copy disabled=move || snapshot.with(Option::is_none)>
                    {move || t().logs_copy}
                </button>
                <p class="min-w-0 flex-1 truncate font-mono text-xs text-zinc-500 dark:text-zinc-400">
                    {move || snapshot.with(|s| s.as_ref().map(|s| s.path.clone()))}
                </p>
            </div>
            <Show when=move || snapshot.with(|s| s.as_ref().is_some_and(|s| s.truncated))>
                <p class="mt-2 text-xs text-zinc-500 dark:text-zinc-400">
                    {move || (t().logs_showing_last)(LOG_LINES)}
                </p>
            </Show>
            <pre class="mt-2 max-h-96 overflow-auto whitespace-pre rounded bg-white p-2 font-mono text-xs text-zinc-800 dark:bg-zinc-900 dark:text-zinc-200">
                {move || {
                    snapshot
                        .with(|s| match s {
                            Some(s) => s.lines.join("\n"),
                            None => t().loading.to_string(),
                        })
                }}
            </pre>
        </div>
    }
}
