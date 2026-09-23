use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_icons::Icon;
use yaydl_shared::{AppUpdateInfo, AppUpdateProgress};

use crate::format;
use crate::ipc::{call0, log_to_backend};
use crate::state::{use_app, AppState};

#[derive(Clone, PartialEq)]
pub enum UpdateState {
    Hidden,
    Available(AppUpdateInfo),
    Installing {
        info: AppUpdateInfo,
        progress: Option<AppUpdateProgress>,
    },
    /// `install_app_update` returned Ok but the app is still running. The
    /// backend restarts it on success, so this only shows if that restart is
    /// slow or did not happen.
    Restarting(AppUpdateInfo),
    Failed {
        info: AppUpdateInfo,
        message: String,
    },
}

pub async fn check(state: AppState) {
    match call0::<Option<AppUpdateInfo>>("check_app_update").await {
        Ok(Some(info)) => state.update.set(UpdateState::Available(info)),
        Ok(None) => {}
        Err(e) => state
            .toasts
            .warning(format!("Checking for a yaydl update failed: {e}")),
    }
}

pub fn on_progress(state: AppState, progress: AppUpdateProgress) {
    state.update.update(|current| match current {
        UpdateState::Installing { progress: slot, .. } => *slot = Some(progress),
        _ => log_to_backend(
            "warn",
            format!(
                "app-update-progress arrived while no update is installing ({} of {:?} bytes)",
                progress.downloaded_bytes, progress.total_bytes
            ),
        ),
    });
}

fn install(state: AppState, info: AppUpdateInfo) {
    state.update.set(UpdateState::Installing {
        info: info.clone(),
        progress: None,
    });
    spawn_local(async move {
        match call0::<()>("install_app_update").await {
            Ok(()) => state.update.set(UpdateState::Restarting(info)),
            Err(message) => state.update.set(UpdateState::Failed { info, message }),
        }
    });
}

fn percent_of(progress: &AppUpdateProgress) -> Option<f64> {
    progress
        .total_bytes
        .filter(|total| *total > 0)
        .map(|total| (progress.downloaded_bytes as f64 / total as f64 * 100.0).min(100.0))
}

#[component]
pub fn UpdateBanner() -> impl IntoView {
    let state = use_app();
    move || banner(state, state.update.get())
}

fn banner(state: AppState, update: UpdateState) -> AnyView {
    let hide = move |_| state.update.set(UpdateState::Hidden);
    match update {
        UpdateState::Hidden => ().into_any(),
        UpdateState::Available(info) => {
            let version = info.version.clone();
            let notes = info.notes.clone();
            view! {
                <Banner tone=Tone::Info>
                    <div class="min-w-0 flex-1">
                        <p class="font-medium">{format!("yaydl {version} is available")}</p>
                        {notes
                            .map(|n| {
                                view! { <p class="mt-0.5 line-clamp-2 text-sm opacity-80">{n}</p> }
                            })}
                    </div>
                    <button class="btn btn-primary" on:click=move |_| install(state, info.clone())>
                        "Update and restart"
                    </button>
                    <button class="btn btn-ghost" on:click=hide>
                        "Later"
                    </button>
                </Banner>
            }
            .into_any()
        }
        UpdateState::Installing { info, progress } => {
            let detail = match &progress {
                None => "Starting the download\u{2026}".to_string(),
                Some(p) => match (p.total_bytes, percent_of(p)) {
                    (Some(total), Some(percent)) => format!(
                        "{} of {} ({percent:.0}%)",
                        format::bytes(p.downloaded_bytes),
                        format::bytes(total),
                    ),
                    _ => format!("{} downloaded", format::bytes(p.downloaded_bytes)),
                },
            };
            let percent = progress.as_ref().and_then(percent_of);
            view! {
                <Banner tone=Tone::Info>
                    <div class="min-w-0 flex-1">
                        <p class="font-medium">{format!("Installing yaydl {}", info.version)}</p>
                        <p class="text-sm opacity-80">{detail}</p>
                        <ProgressBar percent=Signal::stored(percent) />
                    </div>
                    <button class="btn btn-ghost" on:click=hide>
                        "Hide"
                    </button>
                </Banner>
            }
            .into_any()
        }
        UpdateState::Restarting(info) => view! {
            <Banner tone=Tone::Info>
                <p class="min-w-0 flex-1">
                    {format!(
                        "yaydl {} is installed and restarts now. If it does not, restart it yourself.",
                        info.version,
                    )}
                </p>
                <button class="btn btn-ghost" on:click=hide>
                    "Dismiss"
                </button>
            </Banner>
        }
        .into_any(),
        UpdateState::Failed { info, message } => view! {
            <Banner tone=Tone::Error>
                <div class="min-w-0 flex-1">
                    <p class="font-medium">{format!("Updating to yaydl {} failed", info.version)}</p>
                    <p class="max-h-24 overflow-y-auto whitespace-pre-wrap break-words text-sm">
                        {message}
                    </p>
                </div>
                <button class="btn btn-primary" on:click=move |_| install(state, info.clone())>
                    "Retry"
                </button>
                <button class="btn btn-ghost" on:click=hide>
                    "Dismiss"
                </button>
            </Banner>
        }
        .into_any(),
    }
}

#[derive(Clone, Copy)]
enum Tone {
    Info,
    Error,
}

#[component]
fn Banner(tone: Tone, children: Children) -> impl IntoView {
    let (classes, icon) = match tone {
        Tone::Error => (
            "border-red-300 bg-red-50 text-red-900 dark:border-red-800 dark:bg-red-950 dark:text-red-100",
            icondata::LuTriangleAlert,
        ),
        Tone::Info => (
            "border-blue-200 bg-blue-50 text-blue-950 dark:border-blue-900 dark:bg-blue-950 dark:text-blue-50",
            icondata::LuDownload,
        ),
    };
    view! {
        <div role="status" class=format!("flex items-center gap-3 border-b px-4 py-2.5 {classes}")>
            <span class="shrink-0 text-lg" aria-hidden="true">
                <Icon icon=icon />
            </span>
            {children()}
        </div>
    }
}

/// `None` renders an indeterminate bar.
#[component]
pub fn ProgressBar(percent: Signal<Option<f64>>) -> impl IntoView {
    view! {
        <div
            class="relative mt-1.5 h-1.5 w-full overflow-hidden rounded-full bg-zinc-200 dark:bg-zinc-700"
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow=move || percent.get().map(|p| format!("{p:.0}"))
        >
            {move || match percent.get() {
                Some(p) => {
                    view! {
                        <div
                            class="h-full rounded-full bg-blue-600 transition-[width] duration-300 dark:bg-blue-500"
                            style=format!("width: {p:.1}%")
                        ></div>
                    }
                        .into_any()
                }
                None => {
                    view! {
                        <div class="indeterminate h-full w-1/3 rounded-full bg-blue-600 dark:bg-blue-500"></div>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}
