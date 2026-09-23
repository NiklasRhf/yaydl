use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{events, AppUpdateProgress, DownloadItem, Notice};

use crate::i18n::{texts_now, use_texts, Text};
use crate::ipc::{call0, listen, log_to_backend};
use crate::state::{use_app, AppState, Page};
use crate::theme;
use crate::toast::{ToastStack, Toasts};
use crate::update_banner::{self, UpdateBanner};
use crate::views::downloads::DownloadsPage;
use crate::views::settings::SettingsPage;
use crate::views::stats::StatisticsPage;

#[component]
pub fn App() -> impl IntoView {
    let toasts = Toasts::new();
    let state = AppState::new(toasts);
    provide_context(toasts);
    provide_context(state);
    theme::install(state);
    spawn_local(startup(state));

    view! {
        <div class="flex h-screen overflow-hidden">
            <Sidebar />
            <div class="flex min-w-0 flex-1 flex-col">
                <UpdateBanner />
                <main class="min-h-0 flex-1 overflow-y-auto">
                    {move || match state.page.get() {
                        Page::Downloads => view! { <DownloadsPage /> }.into_any(),
                        Page::Statistics => view! { <StatisticsPage /> }.into_any(),
                        Page::Settings => view! { <SettingsPage /> }.into_any(),
                    }}
                </main>
            </div>
        </div>
        <ToastStack />
    }
}

async fn startup(state: AppState) {
    let toasts = state.toasts;
    let queue = state.queue;
    let registrations = [
        listen(
            events::QUEUE_REPLACED,
            toasts,
            move |items: Vec<DownloadItem>| queue.replace(items),
        )
        .await,
        listen(
            events::QUEUE_ITEM_UPDATED,
            toasts,
            move |item: DownloadItem| queue.update(item, state),
        )
        .await,
        listen(events::NOTICE, toasts, move |notice: Notice| {
            toasts.push(notice.level, notice.text)
        })
        .await,
        listen(
            events::APP_UPDATE_PROGRESS,
            toasts,
            move |progress: AppUpdateProgress| update_banner::on_progress(state, progress),
        )
        .await,
        listen(events::HISTORY_CHANGED, toasts, move |(): ()| {
            state.history_rev.update(|rev| *rev += 1)
        })
        .await,
    ];
    let failures: Vec<String> = registrations.into_iter().filter_map(Result::err).collect();
    if !failures.is_empty() {
        let message = (texts_now(state).listen_failed)(&failures.join("\n"));
        log_to_backend("error", message.clone());
        toasts.error(message);
    }

    match call0::<Vec<DownloadItem>>("get_queue").await {
        Ok(items) => queue.replace(items),
        Err(e) => toasts.error((texts_now(state).queue_load_failed)(&e)),
    }
    match call0::<Vec<Notice>>("take_startup_notices").await {
        Ok(notices) => {
            for notice in notices {
                toasts.push(notice.level, notice.text);
            }
        }
        Err(e) => toasts.error((texts_now(state).startup_notices_failed)(&e)),
    }
    state.load_settings().await;
    update_banner::check(state).await;
}

#[component]
fn Sidebar() -> impl IntoView {
    let t = use_texts();
    view! {
        <nav
            aria-label=move || t().nav_label
            class="flex w-52 shrink-0 flex-col gap-1 border-r border-zinc-200 bg-zinc-100/70 p-3 dark:border-zinc-800 dark:bg-zinc-900/60"
        >
            <p class="px-3 pb-4 pt-2 text-xl font-semibold tracking-tight">"yaydl"</p>
            <NavItem page=Page::Downloads label=|t| t.nav_downloads icon=icondata::LuDownload />
            <NavItem page=Page::Statistics label=|t| t.nav_statistics icon=icondata::LuChartColumn />
            <NavItem page=Page::Settings label=|t| t.nav_settings icon=icondata::LuSettings />
        </nav>
    }
}

#[component]
fn NavItem(page: Page, label: Text, icon: icondata::Icon) -> impl IntoView {
    let app = use_app();
    let t = use_texts();
    let active = move || app.page.get() == page;
    view! {
        <button
            class=move || {
                format!(
                    "focus-ring flex items-center gap-3 rounded-md px-3 py-2 text-left text-sm font-medium transition-colors {}",
                    if active() {
                        "bg-white text-zinc-900 shadow-sm dark:bg-zinc-800 dark:text-zinc-50"
                    } else {
                        "text-zinc-600 hover:bg-zinc-200/70 hover:text-zinc-900 dark:text-zinc-400 dark:hover:bg-zinc-800/70 dark:hover:text-zinc-100"
                    },
                )
            }
            aria-current=move || active().then_some("page")
            on:click=move |_| app.page.set(page)
        >
            <span class="text-lg" aria-hidden="true">
                <Icon icon=icon />
            </span>
            {move || label(t())}
        </button>
    }
}
