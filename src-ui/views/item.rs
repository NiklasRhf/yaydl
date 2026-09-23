use leptos::ev;
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{
    playlist_in_video_link, validate_file_stem, ConfirmDuplicateArgs, DownloadId, DownloadItem,
    DownloadStatus, FriendlyError, IdArgs, OutputFormat, RenameArgs, SetFormatArgs,
    MAX_FILE_STEM_CHARS,
};

use crate::format;
use crate::i18n::{error_message, format_label, use_texts, Text, Texts};
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

/// The name the user gave the file wins over the video title, because it is
/// what the file on disk is called.
fn display_title(item: &DownloadItem) -> String {
    match (&item.custom_name, &item.metadata) {
        (Some(name), _) => name.clone(),
        (None, Some(meta)) => meta.title.clone(),
        (None, None) => item.url.clone(),
    }
}

fn title_tooltip(t: &Texts, item: &DownloadItem) -> String {
    match (&item.custom_name, &item.metadata) {
        (Some(_), Some(meta)) => (t.original_title)(&meta.title),
        _ => display_title(item),
    }
}

/// Localized reason why `validate_file_stem` rejects `name`. The shared
/// validator stays the only authority on validity, the checks here only name
/// the rule it applied.
fn rename_error(t: &Texts, name: &str) -> Option<String> {
    let rejection = validate_file_stem(name).err()?;
    if name.trim().is_empty() {
        return Some(t.name_empty.to_string());
    }
    if name != name.trim() {
        return Some(t.name_whitespace.to_string());
    }
    if name.ends_with('.') {
        return Some(t.name_trailing_dot.to_string());
    }
    if name.chars().count() > MAX_FILE_STEM_CHARS {
        return Some((t.name_too_long)(MAX_FILE_STEM_CHARS));
    }
    let forbidden = name
        .chars()
        .find(|c| c.is_control() || validate_file_stem(&format!("a{c}a")).is_err());
    if let Some(c) = forbidden {
        let shown = if c.is_control() {
            c.escape_unicode().to_string()
        } else {
            c.to_string()
        };
        return Some((t.name_forbidden_char)(&shown));
    }
    let base = name.split('.').next().unwrap_or(name);
    if !base.is_empty() && validate_file_stem(base).is_err() {
        return Some((t.name_reserved)(name));
    }
    log_to_backend(
        "warn",
        format!("no localized text for the file name rejection {rejection:?} of {name:?}"),
    );
    Some(capitalize(&rejection))
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

/// Whether the row offers to add the whole playlist its link was opened from,
/// and if so whether that playlist is a YouTube Mix. Once a download started,
/// replacing the item would throw away its progress or file.
fn playlist_offer(item: &DownloadItem) -> Option<bool> {
    if !item.status.can_edit_before_download() {
        return None;
    }
    playlist_in_video_link(&item.url).map(|link| link.is_mix)
}

struct OfferTexts {
    label: &'static str,
    hint: &'static str,
}

fn offer_texts(t: &Texts, is_mix: bool) -> OfferTexts {
    if is_mix {
        OfferTexts {
            label: t.add_whole_mix,
            hint: t.add_whole_mix_hint,
        }
    } else {
        OfferTexts {
            label: t.add_whole_playlist,
            hint: t.add_whole_playlist_hint,
        }
    }
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
    let expanding = RwSignal::new(false);
    let t = use_texts();

    let title = Memo::new(move |_| item.with(display_title));
    let tooltip = Memo::new(move |_| item.with(|i| title_tooltip(t(), i)));
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
            parts.push(format_label(t(), i.format));
            parts.join(" \u{b7} ")
        })
    });
    // A memo so progress and format updates do not rebuild the offer.
    let offer = Memo::new(move |_| item.with(playlist_offer));

    view! {
        <li class="card flex gap-4 p-3">
            <Thumbnail item=item />
            <div class="flex min-w-0 flex-1 flex-col gap-1.5">
                {move || {
                    if editing.get() {
                        let (initial, suffix) = item
                            .with_untracked(|i| {
                                (
                                    display_title(i),
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
                                <p class="truncate font-medium" title=move || tooltip.get()>
                                    {move || title.get()}
                                </p>
                                <p class="truncate text-sm text-zinc-500 dark:text-zinc-400">
                                    {move || secondary.get()}
                                </p>
                                {move || {
                                    offer
                                        .get()
                                        .map(|is_mix| {
                                            view! { <PlaylistOffer id=id is_mix=is_mix expanding=expanding /> }
                                        })
                                }}
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
                            <p class="text-sm text-zinc-500 dark:text-zinc-400">{move || t().item_waiting_slot}</p>
                        }
                            .into_any()
                    }
                    Kind::Downloading => view! { <DownloadingBody item=item /> }.into_any(),
                    Kind::Processing => {
                        view! {
                            <div>
                                <ProgressBar percent=Signal::stored(None) />
                                <p class="mt-1 text-sm text-zinc-500 dark:text-zinc-400">{move || t().item_converting}</p>
                            </div>
                        }
                            .into_any()
                    }
                    Kind::Finished => {
                        view! {
                            <p class="flex items-center gap-1.5 text-sm text-emerald-700 dark:text-emerald-400">
                                <Icon icon=icondata::LuCircleCheck />
                                {move || t().item_finished}
                            </p>
                        }
                            .into_any()
                    }
                    Kind::Cancelled => {
                        view! { <p class="text-sm text-zinc-500 dark:text-zinc-400">{move || t().item_cancelled}</p> }.into_any()
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
    let t = use_texts();
    let remove = move || {
        view! {
            <IconButton
                label=|t| t.action_remove
                icon=icondata::LuTrash2
                on_click=Callback::new(move |()| fire(toasts, "remove_item", IdArgs { id }))
            />
        }
    };
    let retry = move || {
        view! {
            <button class="btn btn-secondary" on:click=move |_| fire(toasts, "retry_download", IdArgs { id })>
                <Icon icon=icondata::LuRotateCw />
                {move || t().action_retry}
            </button>
        }
    };
    let cancel = move || {
        view! {
            <button class="btn btn-secondary" on:click=move |_| fire(toasts, "cancel_download", IdArgs { id })>
                <Icon icon=icondata::LuX />
                {move || t().action_cancel}
            </button>
        }
    };
    let rename = move || {
        view! {
            <IconButton
                label=|t| t.action_rename
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
                {move || t().action_download}
            </button>
            {rename()}
            {remove()}
        }
        .into_any(),
        Kind::AwaitingDuplicate => ().into_any(),
        Kind::Queued | Kind::Downloading | Kind::Processing => cancel().into_any(),
        Kind::Finished => view! {
            <IconButton
                label=|t| t.action_open
                icon=icondata::LuExternalLink
                on_click=Callback::new(move |()| fire(toasts, "open_file", IdArgs { id }))
            />
            <IconButton
                label=|t| t.action_reveal
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
pub fn IconButton(label: Text, icon: icondata::Icon, on_click: Callback<()>) -> impl IntoView {
    let t = use_texts();
    view! {
        <button
            class="icon-btn"
            title=move || label(t())
            aria-label=move || label(t())
            on:click=move |_| on_click.run(())
        >
            <Icon icon=icon />
        </button>
    }
}

#[component]
fn PlaylistOffer(
    id: DownloadId,
    is_mix: bool,
    /// Owned by the row, so rebuilding this view cannot re-enable the button
    /// while the command is still running.
    expanding: RwSignal<bool>,
) -> impl IntoView {
    let toasts = use_app().toasts;
    let t = use_texts();
    let texts = move || offer_texts(t(), is_mix);
    // Success needs no toast, the backend raises its own notices and replaces
    // this row.
    let expand = move |_| {
        if expanding.get_untracked() {
            return;
        }
        expanding.set(true);
        spawn_local(async move {
            if let Err(e) = call::<_, ()>("expand_playlist", &IdArgs { id }).await {
                toasts.error(e);
            }
            expanding.set(false);
        });
    };
    view! {
        <p class="mt-0.5 text-xs">
            <button
                class="link inline-flex items-start gap-1 text-left disabled:cursor-wait disabled:opacity-50 disabled:no-underline"
                title=move || texts().hint
                aria-label=move || texts().label
                aria-busy=move || expanding.get().to_string()
                disabled=move || expanding.get()
                on:click=expand
            >
                <span class="mt-px shrink-0 text-sm leading-none">
                    <Icon icon=icondata::LuListPlus />
                </span>
                <span>{move || texts().label}</span>
            </button>
        </p>
    }
}

#[component]
fn ResolvingBody() -> impl IntoView {
    let t = use_texts();
    view! {
        <div class="flex items-center gap-3" aria-busy="true">
            <div class="h-2 w-40 animate-pulse rounded bg-zinc-200 dark:bg-zinc-700"></div>
            <div class="h-2 w-16 animate-pulse rounded bg-zinc-200 dark:bg-zinc-700"></div>
            <p class="text-sm text-zinc-500 dark:text-zinc-400">{move || t().item_fetching}</p>
        </div>
    }
}

#[component]
fn ErrorBody(item: Signal<DownloadItem>) -> impl IntoView {
    let settings = use_app().settings;
    let t = use_texts();
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
                {move || error.with(|e| e.as_ref().map(|e| error_message(t(), e)))}
            </p>
            <Show when=cookie_hint>
                <p class="mt-1 text-zinc-600 dark:text-zinc-300">
                    {move || t().cookie_hint}
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
                {move || t().details}
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
    let t = use_texts();
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
                {move || t().format_label}
            </label>
            <select id=format!("format-{id}") class="select py-1 text-sm" on:change=on_change>
                {OutputFormat::all()
                    .into_iter()
                    .map(|format| {
                        view! {
                            <option value=format.key() prop:selected=move || current.get() == format>
                                {move || format_label(t(), format)}
                            </option>
                        }
                    })
                    .collect_view()}
            </select>
            {move || {
                previous
                    .get()
                    .map(|ms| {
                        let t = t();
                        view! {
                            <span class="inline-flex items-center gap-1 rounded-full bg-zinc-100 px-2 py-0.5 text-xs text-zinc-600 dark:bg-zinc-800 dark:text-zinc-300">
                                <Icon icon=icondata::LuHistory />
                                {(t.downloaded_before)(&format::date_ms(ms, t.locale))}
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
    let t = use_texts();
    let id = item.with_untracked(|i| i.id);
    let text = move || {
        let t = t();
        item.with(|i| match &i.previous_download {
            Some(previous) => (t.duplicate_prompt)(
                &format::date_ms(previous.finished_at_ms, t.locale),
                &previous.file_path.to_string_lossy(),
            ),
            None => {
                log_to_backend(
                    "warn",
                    format!(
                        "download {id} awaits duplicate confirmation without previous_download"
                    ),
                );
                t.duplicate_prompt_unknown.to_string()
            }
        })
    };
    view! {
        <div class="flex flex-wrap items-center gap-2 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-950 dark:border-amber-700/60 dark:bg-amber-950/50 dark:text-amber-100">
            <p class="min-w-[14rem] flex-1 break-words">{text}</p>
            <button
                class="btn btn-primary py-1"
                on:click=move |_| fire(toasts, "confirm_duplicate", ConfirmDuplicateArgs { id, proceed: true })
            >
                {move || t().download_again}
            </button>
            <button
                class="btn btn-secondary py-1"
                on:click=move |_| fire(toasts, "confirm_duplicate", ConfirmDuplicateArgs { id, proceed: false })
            >
                {move || t().skip}
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
    let t = use_texts();
    let details = move || {
        let t = t();
        let locale = t.locale;
        progress.with(|p| {
            let mut parts = Vec::new();
            if let Some(percent) = p.percent {
                parts.push(format::percent(f64::from(percent), locale));
            }
            match (p.downloaded_bytes, p.total_bytes) {
                (Some(done), Some(total)) => parts.push(format!(
                    "{} / {}",
                    format::bytes(done, locale),
                    format::bytes(total, locale)
                )),
                (Some(done), None) => parts.push(format::bytes(done, locale)),
                (None, Some(total)) => parts.push((t.progress_of)(&format::bytes(total, locale))),
                (None, None) => {}
            }
            if let Some(speed) = p.speed_bytes_per_sec {
                parts.push(format::speed(speed, locale));
            }
            if let Some(eta) = p.eta_secs {
                parts.push((t.eta)(&format::clock(eta as f64)));
            }
            if parts.is_empty() {
                t.item_starting.to_string()
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
    let t = use_texts();
    let name = RwSignal::new(initial);
    let saving = RwSignal::new(false);
    let error = Memo::new(move |_| name.with(|n| rename_error(t(), n)));
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
                {move || t().file_name_label}
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
                    {move || t().save}
                </button>
                <button class="btn btn-ghost py-1" on:click=move |_| editing.set(false)>
                    {move || t().cancel}
                </button>
            </div>
            <p id=format!("rename-error-{id}") class="min-h-[1rem] text-xs text-red-700 dark:text-red-400" aria-live="polite">
                {move || error.get()}
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{DE, EN};
    use yaydl_shared::{AudioCodec, VideoMetadata};

    fn finished(custom_name: Option<&str>) -> DownloadItem {
        DownloadItem {
            id: 7,
            url: "https://youtu.be/abc123".to_string(),
            added_at_ms: 0,
            metadata: Some(VideoMetadata {
                extractor: "Youtube".to_string(),
                video_id: "abc123".to_string(),
                title: "DUNE: Part Three | War Chant Lyrics".to_string(),
                uploader: None,
                duration_secs: None,
                thumbnail: None,
                webpage_url: "https://youtu.be/abc123".to_string(),
            }),
            format: OutputFormat::Audio {
                codec: AudioCodec::Mp3,
            },
            custom_name: custom_name.map(str::to_string),
            status: DownloadStatus::Finished,
            previous_download: None,
            file_path: Some("/m/War Chant.mp3".into()),
        }
    }

    #[test]
    fn a_renamed_item_shows_its_new_name() {
        let renamed = finished(Some("War Chant"));
        assert_eq!(display_title(&renamed), "War Chant");
        assert_eq!(
            title_tooltip(&DE, &renamed),
            "Original: DUNE: Part Three | War Chant Lyrics"
        );

        let plain = finished(None);
        assert_eq!(display_title(&plain), "DUNE: Part Three | War Chant Lyrics");
        assert_eq!(title_tooltip(&EN, &plain), display_title(&plain));

        let unresolved = DownloadItem {
            metadata: None,
            ..finished(None)
        };
        assert_eq!(display_title(&unresolved), "https://youtu.be/abc123");
    }

    #[test]
    fn only_editable_items_from_playlist_links_offer_the_playlist() {
        let with = |url: &str, status: DownloadStatus| DownloadItem {
            url: url.to_string(),
            status,
            ..finished(None)
        };
        let playlist = "https://www.youtube.com/watch?v=abc123&list=PLxyz";
        let mix = "https://youtu.be/abc123?list=RDabc123";

        assert_eq!(
            playlist_offer(&with(playlist, DownloadStatus::Ready)),
            Some(false)
        );
        assert_eq!(
            playlist_offer(&with(mix, DownloadStatus::Cancelled)),
            Some(true)
        );
        assert_eq!(
            playlist_offer(&with(playlist, DownloadStatus::Finished)),
            None
        );
        assert_eq!(
            playlist_offer(&with(playlist, DownloadStatus::Queued)),
            None
        );
        assert_eq!(
            playlist_offer(&with(playlist, DownloadStatus::ResolvingMetadata)),
            None
        );
        assert_eq!(
            playlist_offer(&with("https://youtu.be/abc123", DownloadStatus::Ready)),
            None
        );
    }

    #[test]
    fn a_mix_is_named_a_mix() {
        assert_eq!(offer_texts(&DE, false).label, "Ganze Playlist hinzufügen");
        assert_eq!(offer_texts(&DE, true).label, "Ganzen Mix hinzufügen");
        assert_eq!(offer_texts(&EN, true).hint, EN.add_whole_mix_hint);
        assert_eq!(offer_texts(&EN, false).hint, EN.add_whole_playlist_hint);
    }

    #[test]
    fn rename_errors_name_the_broken_rule() {
        assert_eq!(rename_error(&DE, "War Chant"), None);
        assert_eq!(rename_error(&DE, "  ").as_deref(), Some(DE.name_empty));
        assert_eq!(rename_error(&DE, " a").as_deref(), Some(DE.name_whitespace));
        assert_eq!(
            rename_error(&DE, "a.").as_deref(),
            Some(DE.name_trailing_dot)
        );
        assert_eq!(
            rename_error(&DE, &"x".repeat(MAX_FILE_STEM_CHARS + 1)),
            Some((DE.name_too_long)(MAX_FILE_STEM_CHARS))
        );
        assert_eq!(
            rename_error(&DE, "what?"),
            Some((DE.name_forbidden_char)("?"))
        );
        assert_eq!(
            rename_error(&EN, "a\u{7}b"),
            Some((EN.name_forbidden_char)("\\u{7}"))
        );
        assert_eq!(
            rename_error(&DE, "con.txt"),
            Some((DE.name_reserved)("con.txt"))
        );
    }
}
