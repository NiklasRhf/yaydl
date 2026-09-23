use leptos::prelude::*;
use leptos::task::spawn_local;
use yaydl_shared::Settings;

use crate::ipc::call0;
use crate::queue::Queue;
use crate::toast::Toasts;
use crate::update_banner::UpdateState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Downloads,
    Statistics,
    Settings,
}

#[derive(Clone, Copy)]
pub struct AppState {
    pub toasts: Toasts,
    pub queue: Queue,
    /// `None` until `get_settings` succeeded.
    pub settings: RwSignal<Option<Settings>>,
    pub settings_error: RwSignal<Option<String>>,
    /// Bumped on every `history-changed` event.
    pub history_rev: RwSignal<u64>,
    pub update: RwSignal<UpdateState>,
    pub page: RwSignal<Page>,
}

impl AppState {
    pub fn new(toasts: Toasts) -> Self {
        Self {
            toasts,
            queue: Queue::new(),
            settings: RwSignal::new(None),
            settings_error: RwSignal::new(None),
            history_rev: RwSignal::new(0),
            update: RwSignal::new(UpdateState::Hidden),
            page: RwSignal::new(Page::Downloads),
        }
    }

    pub async fn load_settings(self) {
        match call0::<Settings>("get_settings").await {
            Ok(settings) => {
                self.settings_error.set(None);
                self.settings.set(Some(settings));
            }
            Err(e) => {
                self.toasts
                    .error(format!("Loading the settings failed: {e}"));
                self.settings_error.set(Some(e));
            }
        }
    }

    pub fn reload_settings(self) {
        spawn_local(self.load_settings());
    }
}

pub fn use_app() -> AppState {
    expect_context::<AppState>()
}
