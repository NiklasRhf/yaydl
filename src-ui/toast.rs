use std::time::Duration;

use leptos::prelude::*;
use leptos_icons::Icon;
use yaydl_shared::NoticeLevel;

use crate::i18n::use_texts;
use crate::ipc::log_to_backend;

const MAX_VISIBLE: usize = 5;

#[derive(Clone, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub level: NoticeLevel,
    pub text: String,
}

#[derive(Clone, Copy)]
pub struct Toasts {
    list: RwSignal<Vec<Toast>>,
    next_id: StoredValue<u64>,
}

impl Toasts {
    pub fn new() -> Self {
        Self {
            list: RwSignal::new(Vec::new()),
            next_id: StoredValue::new(0),
        }
    }

    pub fn push(self, level: NoticeLevel, text: impl Into<String>) {
        let id = self.next_id.get_value();
        self.next_id.set_value(id + 1);
        let text = text.into();
        self.list.update(|list| {
            list.push(Toast { id, level, text });
            let overflow = list.len().saturating_sub(MAX_VISIBLE);
            list.drain(..overflow);
        });
        let lifetime = match level {
            NoticeLevel::Success | NoticeLevel::Info => Some(Duration::from_secs(5)),
            NoticeLevel::Warning => Some(Duration::from_secs(8)),
            NoticeLevel::Error => None,
        };
        if let Some(lifetime) = lifetime {
            if let Err(e) = set_timeout_with_handle(move || self.dismiss(id), lifetime) {
                log_to_backend(
                    "warn",
                    format!("scheduling the dismissal of toast {id} failed: {e:?}"),
                );
            }
        }
    }

    pub fn success(self, text: impl Into<String>) {
        self.push(NoticeLevel::Success, text);
    }

    pub fn info(self, text: impl Into<String>) {
        self.push(NoticeLevel::Info, text);
    }

    pub fn warning(self, text: impl Into<String>) {
        self.push(NoticeLevel::Warning, text);
    }

    pub fn error(self, text: impl Into<String>) {
        self.push(NoticeLevel::Error, text);
    }

    pub fn dismiss(self, id: u64) {
        self.list.update(|list| list.retain(|t| t.id != id));
    }
}

pub fn use_toasts() -> Toasts {
    expect_context::<Toasts>()
}

#[component]
pub fn ToastStack() -> impl IntoView {
    let toasts = use_toasts();
    view! {
        <div class="pointer-events-none fixed bottom-4 right-4 z-50 flex w-96 max-w-[calc(100vw-2rem)] flex-col gap-2">
            <For each=move || toasts.list.get() key=|t| t.id let:toast>
                <ToastCard toast=toast />
            </For>
        </div>
    }
}

#[component]
fn ToastCard(toast: Toast) -> impl IntoView {
    let toasts = use_toasts();
    let t = use_texts();
    let id = toast.id;
    let level = toast.level;
    let (icon, accent) = match level {
        NoticeLevel::Success => (
            icondata::LuCircleCheck,
            "border-l-emerald-500 text-emerald-600 dark:text-emerald-400",
        ),
        NoticeLevel::Info => (
            icondata::LuInfo,
            "border-l-blue-500 text-blue-600 dark:text-blue-400",
        ),
        NoticeLevel::Warning => (
            icondata::LuTriangleAlert,
            "border-l-amber-500 text-amber-600 dark:text-amber-400",
        ),
        NoticeLevel::Error => (
            icondata::LuTriangleAlert,
            "border-l-red-500 text-red-600 dark:text-red-400",
        ),
    };
    let label = move || match level {
        NoticeLevel::Success => t().toast_success,
        NoticeLevel::Info => t().toast_info,
        NoticeLevel::Warning => t().toast_warning,
        NoticeLevel::Error => t().toast_error,
    };
    let role = if toast.level == NoticeLevel::Error {
        "alert"
    } else {
        "status"
    };
    view! {
        <div
            role=role
            class=format!(
                "pointer-events-auto flex items-start gap-3 rounded-lg border border-l-4 border-zinc-200 bg-white p-3 shadow-lg dark:border-zinc-700 dark:bg-zinc-900 {accent}",
            )
        >
            <span class="mt-0.5 shrink-0 text-lg" aria-label=label>
                <Icon icon=icon />
            </span>
            <p class="max-h-40 min-w-0 flex-1 overflow-y-auto whitespace-pre-wrap break-words text-sm text-zinc-800 dark:text-zinc-100">
                {toast.text}
            </p>
            <button
                class="icon-btn -m-1 shrink-0"
                title=move || t().dismiss
                aria-label=move || t().dismiss
                on:click=move |_| toasts.dismiss(id)
            >
                <Icon icon=icondata::LuX />
            </button>
        </div>
    }
}
