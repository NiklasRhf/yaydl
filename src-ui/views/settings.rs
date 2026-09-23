use std::path::PathBuf;

use leptos::ev;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{
    Browser, Language, OutputFormat, Settings, SettingsArgs, Theme, YtDlpChannel, YtDlpStatus,
    YtDlpUpdateEvent, MAX_CONCURRENT_DOWNLOADS,
};

use crate::i18n::{format_label, texts_now, use_texts, Text};
use crate::ipc::{call, call0, log_to_backend};
use crate::log_viewer::LogViewer;
use crate::state::{use_app, AppState};
use crate::views::item::capitalize;

/// Sends the whole settings with one field changed. The controls render from
/// the last value the backend confirmed, so on failure a notify puts them
/// back to it.
fn save(state: AppState, change: impl FnOnce(&mut Settings)) {
    let Some(mut next) = state.settings.get_untracked() else {
        state.toasts.error(texts_now(state).settings_not_loaded);
        return;
    };
    change(&mut next);
    if let Err(e) = next.validate() {
        state.toasts.error((texts_now(state).invalid_settings)(&e));
        state.settings.notify();
        return;
    }
    spawn_local(async move {
        match call::<_, Settings>("update_settings", &SettingsArgs { settings: next }).await {
            Ok(saved) => state.settings.set(Some(saved)),
            Err(e) => {
                state
                    .toasts
                    .error((texts_now(state).settings_save_failed)(&e));
                state.settings.notify();
            }
        }
    });
}

/// A reactive read of one field, `None` until the settings loaded.
fn setting<T>(
    state: AppState,
    read: impl Fn(&Settings) -> T + Send + Sync + 'static,
) -> impl Fn() -> Option<T> + Copy + Send + Sync + 'static
where
    T: Send + Sync + 'static,
{
    let read = StoredValue::new(read);
    move || {
        state
            .settings
            .with(|s| s.as_ref().map(|s| read.with_value(|read| read(s))))
    }
}

fn browser_label(browser: Browser) -> String {
    capitalize(browser.as_str())
}

/// Channel names are yt-dlp's, so they are not translated.
fn channel_label(channel: YtDlpChannel) -> &'static str {
    match channel {
        YtDlpChannel::Stable => "Stable",
        YtDlpChannel::Nightly => "Nightly",
    }
}

/// Reports an unknown `<option>` value, which means the markup and the parser
/// disagree, and puts the control back.
fn unknown_option(state: AppState, control: &str, value: &str) {
    let message = format!("the {control} control produced an unknown value {value:?}");
    log_to_backend("error", message.clone());
    state.toasts.error(message);
    state.settings.notify();
}

#[component]
pub fn SettingsPage() -> impl IntoView {
    let state = use_app();
    let t = use_texts();
    // A memo, so saving a setting does not rebuild the form and steal focus.
    let loaded = Memo::new(move |_| state.settings.with(Option::is_some));
    view! {
        <div class="mx-auto flex max-w-3xl flex-col gap-6 p-6">
            <h1 class="text-2xl font-semibold tracking-tight">{move || t().settings_title}</h1>
            {move || {
                if loaded.get() {
                    view! { <SettingsForm /> }.into_any()
                } else if let Some(error) = state.settings_error.get() {
                    view! {
                        <div class="card flex flex-col items-start gap-3 p-4">
                            <p class="text-red-700 dark:text-red-400">
                                {move || (t().settings_load_failed)(&error)}
                            </p>
                            <button class="btn btn-secondary" on:click=move |_| state.reload_settings()>
                                {move || t().try_again}
                            </button>
                        </div>
                    }
                        .into_any()
                } else {
                    view! { <p class="text-zinc-500 dark:text-zinc-400">{move || t().settings_loading}</p> }.into_any()
                }
            }}
            <YtDlpSection />
            <LogsSection />
        </div>
    }
}

#[component]
fn Section(title: Text, children: Children) -> impl IntoView {
    let t = use_texts();
    view! {
        <section class="card flex flex-col divide-y divide-zinc-200 dark:divide-zinc-800">
            <h2 class="px-4 py-3 text-sm font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">
                {move || title(t())}
            </h2>
            {children()}
        </section>
    }
}

#[component]
fn Row(
    label: Text,
    #[prop(optional)] help: Option<Text>,
    #[prop(optional)] label_for: Option<&'static str>,
    children: Children,
) -> impl IntoView {
    let t = use_texts();
    view! {
        <div class="flex flex-wrap items-center gap-x-6 gap-y-2 px-4 py-3">
            <div class="min-w-[12rem] flex-1">
                <label class="font-medium" for=label_for>
                    {move || label(t())}
                </label>
                {help.map(|h| view! { <p class="mt-0.5 text-sm text-zinc-500 dark:text-zinc-400">{move || h(t())}</p> })}
            </div>
            <div class="flex items-center gap-2">{children()}</div>
        </div>
    }
}

#[component]
fn SettingsForm() -> impl IntoView {
    let state = use_app();
    let output_dir = setting(state, |s| s.output_dir.to_string_lossy().into_owned());
    let default_format = setting(state, |s| s.default_format);
    let max_parallel = setting(state, |s| s.max_concurrent_downloads);
    let embed = setting(state, |s| s.embed_metadata);
    let notify = setting(state, |s| s.notify_on_finish);
    let cookies = setting(state, |s| s.cookies_from_browser);
    let theme = setting(state, |s| s.theme);
    let language = setting(state, |s| s.language);
    let t = use_texts();
    let choosing = RwSignal::new(false);

    let choose_dir = move |_| {
        choosing.set(true);
        spawn_local(async move {
            match call0::<Option<String>>("choose_output_dir").await {
                Ok(Some(path)) => save(state, |s| s.output_dir = PathBuf::from(path)),
                Ok(None) => {}
                Err(e) => state.toasts.error(e),
            }
            choosing.set(false);
        });
    };

    view! {
        <Section title=|t| t.section_downloads>
            <Row label=|t| t.output_folder>
                <p class="max-w-xs truncate font-mono text-sm text-zinc-600 dark:text-zinc-300" title=output_dir>
                    {output_dir}
                </p>
                <button class="btn btn-secondary" disabled=move || choosing.get() on:click=choose_dir>
                    <Icon icon=icondata::LuFolder />
                    {move || t().change_folder}
                </button>
            </Row>
            <Row label=|t| t.default_format label_for="default-format">
                <select
                    id="default-format"
                    class="select"
                    on:change=move |ev: ev::Event| {
                        let key = event_target_value(&ev);
                        match OutputFormat::parse_key(&key) {
                            Some(format) => save(state, |s| s.default_format = format),
                            None => unknown_option(state, "default format", &key),
                        }
                    }
                >
                    {OutputFormat::all()
                        .into_iter()
                        .map(|format| {
                            view! {
                                <option value=format.key() prop:selected=move || default_format() == Some(format)>
                                    {move || format_label(t(), format)}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </Row>
            <Row label=|t| t.parallel_downloads label_for="max-parallel" help=|t| t.parallel_help>
                <select
                    id="max-parallel"
                    class="select"
                    on:change=move |ev: ev::Event| {
                        let value = event_target_value(&ev);
                        match value.parse::<u8>() {
                            Ok(n) => save(state, |s| s.max_concurrent_downloads = n),
                            Err(_) => unknown_option(state, "parallel downloads", &value),
                        }
                    }
                >
                    {(1..=MAX_CONCURRENT_DOWNLOADS)
                        .map(|n| {
                            view! {
                                <option value=n.to_string() prop:selected=move || max_parallel() == Some(n)>
                                    {n.to_string()}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </Row>
            <Row label=|t| t.embed_metadata label_for="embed-metadata" help=|t| t.embed_help>
                <Toggle
                    id="embed-metadata"
                    checked=Signal::derive(move || embed().unwrap_or(false))
                    on_toggle=Callback::new(move |on| save(state, |s| s.embed_metadata = on))
                />
            </Row>
            <Row label=|t| t.notify_finish label_for="notify-finish">
                <Toggle
                    id="notify-finish"
                    checked=Signal::derive(move || notify().unwrap_or(false))
                    on_toggle=Callback::new(move |on| save(state, |s| s.notify_on_finish = on))
                />
            </Row>
        </Section>

        <Section title=|t| t.section_cookies>
            <Row label=|t| t.browser_cookies label_for="cookies-browser" help=|t| t.cookies_help>
                <select
                    id="cookies-browser"
                    class="select"
                    on:change=move |ev: ev::Event| {
                        let value = event_target_value(&ev);
                        if value.is_empty() {
                            save(state, |s| s.cookies_from_browser = None);
                        } else {
                            match Browser::parse(&value) {
                                Some(browser) => save(state, |s| s.cookies_from_browser = Some(browser)),
                                None => unknown_option(state, "cookies browser", &value),
                            }
                        }
                    }
                >
                    <option value="" prop:selected=move || cookies() == Some(None)>
                        {move || t().cookies_none}
                    </option>
                    {Browser::ALL
                        .into_iter()
                        .map(|browser| {
                            view! {
                                <option value=browser.as_str() prop:selected=move || cookies() == Some(Some(browser))>
                                    {browser_label(browser)}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </Row>
        </Section>

        <Section title=|t| t.section_appearance>
            <Row label=|t| t.theme>
                <div class="segmented" role="radiogroup" aria-label=move || t().theme>
                    {Theme::ALL
                        .into_iter()
                        .map(|choice| {
                            let label = move || match choice {
                                Theme::System => t().theme_system,
                                Theme::Light => t().theme_light,
                                Theme::Dark => t().theme_dark,
                            };
                            let icon = match choice {
                                Theme::System => icondata::LuMonitor,
                                Theme::Light => icondata::LuSun,
                                Theme::Dark => icondata::LuMoon,
                            };
                            view! {
                                <button
                                    role="radio"
                                    aria-checked=move || (theme() == Some(choice)).to_string()
                                    class="segmented-item"
                                    on:click=move |_| save(state, |s| s.theme = choice)
                                >
                                    <Icon icon=icon />
                                    {label}
                                </button>
                            }
                        })
                        .collect_view()}
                </div>
            </Row>
            <Row label=|t| t.language label_for="language">
                <select
                    id="language"
                    class="select"
                    on:change=move |ev: ev::Event| {
                        let value = event_target_value(&ev);
                        match Language::parse(&value) {
                            Some(l) => save(state, |s| s.language = l),
                            None => unknown_option(state, "language", &value),
                        }
                    }
                >
                    {Language::ALL
                        .into_iter()
                        .map(|l| {
                            let label = move || match l {
                                Language::System => t().language_system,
                                // Each language is named in itself, so it can be found
                                // by someone who cannot read the current one.
                                Language::English => "English",
                                Language::German => "Deutsch",
                            };
                            view! {
                                <option value=l.as_str() prop:selected=move || language() == Some(l)>
                                    {label}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </Row>
        </Section>
    }
}

#[component]
fn Toggle(id: &'static str, checked: Signal<bool>, on_toggle: Callback<bool>) -> impl IntoView {
    view! {
        <label class="toggle">
            <input
                id=id
                type="checkbox"
                role="switch"
                class="peer sr-only"
                prop:checked=move || checked.get()
                on:change=move |ev: ev::Event| on_toggle.run(event_target_checked(&ev))
            />
            <span class="toggle-track" aria-hidden="true"></span>
        </label>
    }
}

#[component]
fn YtDlpSection() -> impl IntoView {
    let state = use_app();
    let t = use_texts();
    let status = RwSignal::new(None::<Result<YtDlpStatus, String>>);
    let updating = RwSignal::new(false);
    let channel = setting(state, |s| s.yt_dlp_channel);

    let load_status = move || {
        spawn_local(async move {
            status.set(Some(call0::<YtDlpStatus>("get_yt_dlp_status").await));
        });
    };
    load_status();

    // The backend may switch binaries when the channel changes, so the shown
    // version follows the confirmed channel setting.
    Effect::new(move |previous: Option<Option<YtDlpChannel>>| {
        let current = channel();
        if previous.is_some_and(|p| p.is_some() && p != current) {
            load_status();
        }
        current
    });

    let update_now =
        move |_| {
            updating.set(true);
            spawn_local(async move {
                let result = call0::<YtDlpUpdateEvent>("update_yt_dlp").await;
                let t = texts_now(state);
                match result {
                    Ok(YtDlpUpdateEvent::Updated { from, to, channel }) => state
                        .toasts
                        .success((t.ytdlp_updated)(&from, &to, channel_label(channel))),
                    Ok(YtDlpUpdateEvent::AlreadyCurrent { version, channel }) => state
                        .toasts
                        .info((t.ytdlp_current)(&version, channel_label(channel))),
                    Ok(YtDlpUpdateEvent::Failed { message }) => {
                        state.toasts.error((t.ytdlp_update_failed)(&message))
                    }
                    Err(e) => state.toasts.error((t.ytdlp_update_failed)(&e)),
                }
                updating.set(false);
                load_status();
            });
        };

    view! {
        <Section title=|_| "yt-dlp">
            <Row label=|t| t.installed_version>
                <p class="font-mono text-sm text-zinc-600 dark:text-zinc-300">
                    {move || match status.get() {
                        None => t().loading.to_string(),
                        Some(Ok(s)) => format!("{} ({})", s.version, channel_label(s.channel)),
                        Some(Err(e)) => (t().version_unknown)(&e),
                    }}
                </p>
            </Row>
            <Row label=|t| t.release_channel label_for="ytdlp-channel" help=|t| t.channel_help>
                <select
                    id="ytdlp-channel"
                    class="select"
                    disabled=move || channel().is_none()
                    on:change=move |ev: ev::Event| {
                        let value = event_target_value(&ev);
                        match YtDlpChannel::parse(&value) {
                            Some(c) => save(state, |s| s.yt_dlp_channel = c),
                            None => unknown_option(state, "yt-dlp channel", &value),
                        }
                    }
                >
                    {YtDlpChannel::ALL
                        .into_iter()
                        .map(|c| {
                            view! {
                                <option value=c.as_str() prop:selected=move || channel() == Some(c)>
                                    {channel_label(c)}
                                </option>
                            }
                        })
                        .collect_view()}
                </select>
            </Row>
            <Row label=|t| t.update_label>
                <button class="btn btn-secondary" disabled=move || updating.get() on:click=update_now>
                    <span class=move || if updating.get() { "animate-spin" } else { "" }>
                        <Icon icon=icondata::LuRefreshCw />
                    </span>
                    {move || if updating.get() { t().updating } else { t().update_now }}
                </button>
            </Row>
        </Section>
    }
}

#[component]
fn LogsSection() -> impl IntoView {
    let open = RwSignal::new(false);
    let t = use_texts();
    view! {
        <Section title=|t| t.section_logs>
            <div class="px-4 py-3">
                <button
                    class="btn btn-secondary"
                    aria-expanded=move || open.get().to_string()
                    on:click=move |_| open.update(|o| *o = !*o)
                >
                    <Icon icon=icondata::LuFileText />
                    {move || if open.get() { t().hide_logs } else { t().show_logs }}
                </button>
                <Show when=move || open.get()>
                    <LogViewer />
                </Show>
            </div>
        </Section>
    }
}
