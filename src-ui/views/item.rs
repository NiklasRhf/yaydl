use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{
    validate_file_stem, ConfirmDuplicateArgs, DownloadId, DownloadItem, DownloadStatus,
    FriendlyError, IdArgs, OutputFormat, RenameArgs, SetFormatArgs,
};

use crate::format;
use crate::ipc::{call, fire, log_to_backend};
use crate::state::use_app;
use crate::update_banner::ProgressBar;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Resolving,
    MetadataFailed,
    Ready,
    AwaitingDuplicate,
    Queued,
    Downloading,
    Processing,
    Finished,
    Failed,
    Cancelled,
}

impl Kind {
    fn of(status: &DownloadStatus) -> Self {
        match status {
            DownloadStatus::ResolvingMetadata => Kind::Resolving,
            DownloadStatus::MetadataFailed { .. } => Kind::MetadataFailed,
            DownloadStatus::Ready => Kind::Ready,
            DownloadStatus::AwaitingDuplicateConfirmation => Kind::AwaitingDuplicate,
            DownloadStatus::Queued => Kind::Queued,
            DownloadStatus::Downloading { .. } => Kind::Downloading,
            DownloadStatus::Processing => Kind::Processing,
            DownloadStatus::Finished => Kind::Finished,
            DownloadStatus::Failed { .. } => Kind::Failed,
            DownloadStatus::Cancelled => Kind::Cancelled,
        }
    }
}

fn display_title(item: &DownloadItem) -> String {
    match &item.metadata {
        Some(meta) => meta.title.clone(),
        None => item.url.clone(),
    }
}

/// The extension a rename keeps, shown next to the input. Only a finished
/// item has a file on disk whose extension is known.
fn finished_extension(item: &DownloadItem) -> Option<String> {
    if !matches!(item.status, DownloadStatus::Finished) {
        return None;
    }
    let Some(path) = item.file_path.as_ref() else {
        log_to_backend(
            "warn",
            format!("download {} is finished but has no file_path", item.id),
        );
        return None;
    };
    let path = path.to_string_lossy();
    format::extension(&path).map(|ext| format!(".{ext}"))
}

fn friendly_error(item: &DownloadItem) -> Option<FriendlyError> {
    match &item.status {
        DownloadStatus::Failed { error } | DownloadStatus::MetadataFailed { error } => {
            Some(error.clone())
        }
        _ => None,
    }
}

#[component]
pub fn ItemRow(item: Signal<DownloadItem>) -> impl IntoView {
    let id = item.with_untracked(|i| i.id);
    let kind = Memo::new(move |_| item.with(|i| Kind::of(&i.status)));
    let editing = RwSignal::new(false);

    let title = Memo::new(move |_| item.with(display_title));
    let secondary = Memo::new(move |_| {
        item.with(|i| {
            let mut parts = Vec::new();
            if let Some(meta) = &i.metadata {
                if let Some(uploader) = &meta.uploader {
                    parts.push(uploader.clone());
                }
                if let Some(secs) = meta.duration_secs {
                    parts.push(format::clock(secs));
                }
            }
            parts.push(i.format.to_string());
            if let Some(name) = &i.custom_name {
                parts.push(format!("saves as \"{name}\""));
            }
            parts.join(" \u{b7} ")
        })
    });

    view! {
        <li class="card flex gap-4 p-3">
            <Thumbnail item=item />
            <div class="flex min-w-0 flex-1 flex-col gap-1.5">
                {move || {
                    if editing.get() {
                        let (initial, suffix) = item
                            .with_untracked(|i| {
                                (
                                    i.custom_name.clone().unwrap_or_else(|| display_title(i)),
                                    finished_extension(i),
                                )
                            });
                        view! {
                            <RenameEditor
                                id=id
                                initial=initial
                                suffix=suffix
                                editing=editing
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <div class="min-w-0">
                                <p class="truncate font-medium" title=move || title.get()>
                                    {move || title.get()}
                                </p>
                                <p class="truncate text-sm text-zinc-500 dark:text-zinc-400">
                                    {move || secondary.get()}
                                </p>
                            </div>
                        }
                            .into_any()
                    }
                }}
                {move || match kind.get() {
                    Kind::Resolving => view! { <ResolvingBody /> }.into_any(),
                    Kind::MetadataFailed | Kind::Failed => view! { <ErrorBody item=item /> }.into_any(),
                    Kind::Ready => view! { <ReadyBody item=item /> }.into_any(),
                    Kind::AwaitingDuplicate => view! { <DuplicateBody item=item /> }.into_any(),
                    Kind::Queued => {
                        view! {
                            <p class="text-sm text-zinc-500 dark:text-zinc-400">"Waiting for a free slot"</p>
                        }
                            .into_any()
                    }
                    Kind::Downloading => view! { <DownloadingBody item=item /> }.into_any(),
                    Kind::Processing => {
                        view! {
                            <div>
                                <ProgressBar percent=Signal::stored(None) />
                                <p class="mt-1 text-sm text-zinc-500 dark:text-zinc-400">"Converting\u{2026}"</p>
                            </div>
                        }
                            .into_any()
                    }
                    Kind::Finished => {
                        view! {
                            <p class="flex items-center gap-1.5 text-sm text-emerald-700 dark:text-emerald-400">
                                <Icon icon=icondata::LuCircleCheck />
                                "Finished"
                            </p>
                        }
                            .into_any()
                    }
                    Kind::Cancelled => {
                        view! { <p class="text-sm text-zinc-500 dark:text-zinc-400">"Cancelled"</p> }.into_any()
                    }
                }}
            </div>
            <div class="flex shrink-0 items-start gap-1">
                {move || actions(id, kind.get(), editing)}
            </div>
        </li>
    }
}

#[component]
fn Thumbnail(item: Signal<DownloadItem>) -> impl IntoView {
    let src =
        Memo::new(move |_| item.with(|i| i.metadata.as_ref().and_then(|m| m.thumbnail.clone())));
    let broken = RwSignal::new(false);
    let placeholder = || {
        view! {
            <div class="flex h-full w-full items-center justify-center text-2xl text-zinc-400 dark:text-zinc-500">
                <Icon icon=icondata::LuImage />
            </div>
        }
    };
    view! {
        <div class="h-[68px] w-[120px] shrink-0 overflow-hidden rounded-md bg-zinc-100 dark:bg-zinc-800">
            {move || match src.get() {
                Some(url) if !broken.get() => {
                    view! {
                        <img
                            src=url
                            alt=""
                            loading="lazy"
                            referrerpolicy="no-referrer"
                            class="h-full w-full object-cover"
                            on:error=move |_| broken.set(true)
                        />
                    }
                        .into_any()
                }
                _ => placeholder().into_any(),
            }}
        </div>
    }
}

fn actions(id: DownloadId, kind: Kind, editing: RwSignal<bool>) -> AnyView {
    let toasts = use_app().toasts;
    let remove = move || {
        view! {
            <IconButton
                label="Remove"
                icon=icondata::LuTrash2
                on_click=Callback::new(move |()| fire(toasts, "remove_item", IdArgs { id }))
            />
        }
    };
    let retry = move || {
        view! {
            <button class="btn btn-secondary" on:click=move |_| fire(toasts, "retry_download", IdArgs { id })>
                <Icon icon=icondata::LuRotateCw />
                "Retry"
            </button>
        }
    };
    let cancel = move || {
        view! {
            <button class="btn btn-secondary" on:click=move |_| fire(toasts, "cancel_download", IdArgs { id })>
                <Icon icon=icondata::LuX />
                "Cancel"
            </button>
        }
    };
    let rename = move || {
        view! {
            <IconButton
                label="Rename"
                icon=icondata::LuPencil
                on_click=Callback::new(move |()| editing.set(true))
            />
        }
    };
    match kind {
        Kind::Resolving => remove().into_any(),
        Kind::MetadataFailed | Kind::Failed | Kind::Cancelled => {
            view! {
                {retry()}
                {remove()}
            }
            .into_any()
        }
        Kind::Ready => view! {
            <button class="btn btn-primary" on:click=move |_| fire(toasts, "start_download", IdArgs { id })>
                <Icon icon=icondata::LuDownload />
                "Download"
            </button>
            {rename()}
            {remove()}
        }
        .into_any(),
        Kind::AwaitingDuplicate => ().into_any(),
        Kind::Queued | Kind::Downloading | Kind::Processing => cancel().into_any(),
        Kind::Finished => view! {
            <IconButton
                label="Open"
                icon=icondata::LuExternalLink
                on_click=Callback::new(move |()| fire(toasts, "open_file", IdArgs { id }))
            />
            <IconButton
                label="Show in folder"
                icon=icondata::LuFolderOpen
                on_click=Callback::new(move |()| fire(toasts, "reveal_file", IdArgs { id }))
            />
            {rename()}
            {remove()}
        }
        .into_any(),
    }
}

#[component]
pub fn IconButton(
    label: &'static str,
    icon: icondata::Icon,
    on_click: Callback<()>,
) -> impl IntoView {
    view! {
        <button class="icon-btn" title=label aria-label=label on:click=move |_| on_click.run(())>
            <Icon icon=icon />
        </button>
    }
}

#[component]
fn ResolvingBody() -> impl IntoView {
    view! {
        <div class="flex items-center gap-3" aria-busy="true">
            <div class="h-2 w-40 animate-pulse rounded bg-zinc-200 dark:bg-zinc-700"></div>
            <div class="h-2 w-16 animate-pulse rounded bg-zinc-200 dark:bg-zinc-700"></div>
            <p class="text-sm text-zinc-500 dark:text-zinc-400">"Fetching info\u{2026}"</p>
        </div>
    }
}

#[component]
fn ErrorBody(item: Signal<DownloadItem>) -> impl IntoView {
    let settings = use_app().settings;
    let show_details = RwSignal::new(false);
    let error = Memo::new(move |_| item.with(friendly_error));
    let cookie_hint = move || {
        let suggests = item.with(|i| {
            matches!(&i.status, DownloadStatus::Failed { error } if error.kind.suggests_cookies())
        });
        let has_cookies =
            settings.with(|s| s.as_ref().is_some_and(|s| s.cookies_from_browser.is_some()));
        suggests && !has_cookies
    };
    view! {
        <div class="text-sm">
            <p class="whitespace-pre-wrap text-red-700 dark:text-red-400">
                {move || error.with(|e| e.as_ref().map(|e| e.message.clone()))}
            </p>
            <Show when=cookie_hint>
                <p class="mt-1 text-zinc-600 dark:text-zinc-300">
                    "Choose a browser under Settings \u{2192} Cookies, then retry."
                </p>
            </Show>
            <button
                class="link mt-1 inline-flex items-center gap-1 text-xs"
                aria-expanded=move || show_details.get().to_string()
                on:click=move |_| show_details.update(|v| *v = !*v)
            >
                <span class=move || if show_details.get() { "rotate-90 transition-transform" } else { "transition-transform" }>
                    <Icon icon=icondata::LuChevronRight />
                </span>
                "Details"
            </button>
            <Show when=move || show_details.get()>
                <pre class="mt-1 max-h-48 overflow-auto whitespace-pre-wrap break-words rounded bg-zinc-100 p-2 font-mono text-xs text-zinc-700 dark:bg-zinc-950 dark:text-zinc-300">
                    {move || error.with(|e| e.as_ref().map(|e| e.detail.clone()))}
                </pre>
            </Show>
        </div>
    }
}

#[component]
fn ReadyBody(item: Signal<DownloadItem>) -> impl IntoView {
    let toasts = use_app().toasts;
    let id = item.with_untracked(|i| i.id);
    let current = Memo::new(move |_| item.with(|i| i.format));
    let previous =
        Memo::new(move |_| item.with(|i| i.previous_download.as_ref().map(|p| p.finished_at_ms)));
    let on_change = move |ev: ev::Event| {
        let key = event_target_value(&ev);
        match OutputFormat::parse_key(&key) {
            Some(format) => fire(toasts, "set_item_format", SetFormatArgs { id, format }),
            None => {
                let message = format!("the format select offered an unknown format key {key:?}");
                log_to_backend("error", message.clone());
                toasts.error(message);
            }
        }
    };
    view! {
        <div class="flex flex-wrap items-center gap-2">
            <label class="sr-only" for=format!("format-{id}")>
                "Format"
            </label>
            <select id=format!("format-{id}") class="select py-1 text-sm" on:change=on_change>
                {OutputFormat::all()
                    .into_iter()
                    .map(|format| {
                        view! {
                            <option value=format.key() prop:selected=move || current.get() == format>
                                {format.to_string()}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
            {move || {
                previous
                    .get()
                    .map(|ms| {
                        view! {
                            <span class="inline-flex items-center gap-1 rounded-full bg-zinc-100 px-2 py-0.5 text-xs text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300">
                                <Icon icon=icondata::LuHistory />
                                {format!("Downloaded before on {}", format::date_ms(ms))}
                            </span>
                        }
                    })
            }}
        </div>
    }
}

#[component]
fn DuplicateBody(item: Signal<DownloadItem>) -> impl IntoView {
    let toasts = use_app().toasts;
    let id = item.with_untracked(|i| i.id);
    let text = move || {
        item.with(|i| match &i.previous_download {
            Some(previous) => format!(
                "You already downloaded this on {} to {}. Download it again?",
                format::date_ms(previous.finished_at_ms),
                previous.file_path.to_string_lossy()
            ),
            None => {
                log_to_backend(
                    "warn",
                    format!(
                        "download {id} awaits duplicate confirmation without previous_download"
                    ),
                );
                "You already downloaded this. Download it again?".to_string()
            }
        })
    };
    view! {
        <div class="flex flex-wrap items-center gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-950 dark:border-amber-700/60 dark:bg-amber-950/50 dark:text-amber-100">
            <p class="min-w-0 flex-1 break-words">{text}</p>
            <button
                class="btn btn-primary py-1"
                on:click=move |_| fire(toasts, "confirm_duplicate", ConfirmDuplicateArgs { id, proceed: true })
            >
                "Download again"
            </button>
            <button
                class="btn btn-secondary py-1"
                on:click=move |_| fire(toasts, "confirm_duplicate", ConfirmDuplicateArgs { id, proceed: false })
            >
                "Skip"
            </button>
        </div>
    }
}

#[component]
fn DownloadingBody(item: Signal<DownloadItem>) -> impl IntoView {
    let progress = Memo::new(move |_| {
        item.with(|i| match &i.status {
            DownloadStatus::Downloading { progress } => progress.clone(),
            _ => Default::default(),
        })
    });
    let percent = Signal::derive(move || progress.with(|p| p.percent.map(f64::from)));
    let details = move || {
        progress.with(|p| {
            let mut parts = Vec::new();
            if let Some(percent) = p.percent {
                parts.push(format!("{percent:.0}%"));
            }
            match (p.downloaded_bytes, p.total_bytes) {
                (Some(done), Some(total)) => parts.push(format!(
                    "{} / {}",
                    format::bytes(done),
                    format::bytes(total)
                )),
                (Some(done), None) => parts.push(format::bytes(done)),
                (None, Some(total)) => parts.push(format!("of {}", format::bytes(total))),
                (None, None) => {}
            }
            if let Some(speed) = p.speed_bytes_per_sec {
                parts.push(format::speed(speed));
            }
            if let Some(eta) = p.eta_secs {
                parts.push(format::eta(eta));
            }
            if parts.is_empty() {
                "Starting\u{2026}".to_string()
            } else {
                parts.join(" \u{b7} ")
            }
        })
    };
    view! {
        <div>
            <ProgressBar percent=percent />
            <p class="mt-1 text-sm tabular-nums text-zinc-500 dark:text-zinc-400">{details}</p>
        </div>
    }
}

#[component]
fn RenameEditor(
    id: DownloadId,
    initial: String,
    suffix: Option<String>,
    /// A signal rather than a callback, because the save can finish after the
    /// row was removed, and setting a disposed signal is a no-op.
    editing: RwSignal<bool>,
) -> impl IntoView {
    let toasts = use_app().toasts;
    let name = RwSignal::new(initial);
    let saving = RwSignal::new(false);
    let error = Memo::new(move |_| name.with(|n| validate_file_stem(n).err()));
    let input_ref = NodeRef::<html::Input>::new();

    Effect::new(move |_| {
        if let Some(input) = input_ref.get() {
            if let Err(e) = input.focus() {
                log_to_backend("warn", format!("focusing the rename field failed: {e:?}"));
            }
            input.select();
        }
    });

    let save = move || {
        if error.with_untracked(Option::is_some) || saving.get_untracked() {
            return;
        }
        saving.set(true);
        let name = name.get_untracked();
        spawn_local(async move {
            match call::<_, ()>("rename_item", &RenameArgs { id, name }).await {
                Ok(()) => editing.set(false),
                Err(e) => toasts.error(e),
            }
            saving.set(false);
        });
    };
    let on_keydown = move |ev: ev::KeyboardEvent| match ev.key().as_str() {
        "Enter" => {
            ev.prevent_default();
            save();
        }
        "Escape" => {
            ev.prevent_default();
            editing.set(false);
        }
        _ => {}
    };
    let input_id = format!("rename-{id}");
    view! {
        <div class="flex flex-col gap-1">
            <label class="sr-only" for=input_id.clone()>
                "File name"
            </label>
            <div class="flex items-center gap-2">
                <div class="flex min-w-0 flex-1 items-center">
                    <input
                        id=input_id
                        node_ref=input_ref
                        class=move || {
                            format!(
                                "input min-w-0 flex-1 py-1 text-sm {}",
                                if error.with(Option::is_some) { "input-invalid" } else { "" },
                            )
                        }
                        type="text"
                        spellcheck="false"
                        aria-invalid=move || error.with(Option::is_some).to_string()
                        aria-describedby=format!("rename-error-{id}")
                        bind:value=name
                        on:keydown=on_keydown
                    />
                    {suffix
                        .map(|s| {
                            view! { <span class="ml-1 shrink-0 font-mono text-sm text-zinc-500 dark:text-zinc-400">{s}</span> }
                        })}
                </div>
                <button
                    class="btn btn-primary py-1"
                    disabled=move || error.with(Option::is_some) || saving.get()
                    on:click=move |_| save()
                >
                    "Save"
                </button>
                <button class="btn btn-ghost py-1" on:click=move |_| editing.set(false)>
                    "Cancel"
                </button>
            </div>
            <p id=format!("rename-error-{id}") class="min-h-[1rem] text-xs text-red-700 dark:text-red-400" aria-live="polite">
                {move || error.get().map(|e| capitalize(&e))}
            </p>
        </div>
    }
}

pub fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
