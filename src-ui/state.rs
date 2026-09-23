use leptos::prelude::*;
use leptos::task::spawn_local;
use yaydl_shared::{Language, Locale, Settings};

use crate::i18n::{self, texts_now};
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
    /// Follows `settings.language`, and `navigator.language` for `System` and
    /// until the settings loaded.
    pub locale: Signal<Locale>,
}

impl AppState {
    pub fn new(toasts: Toasts) -> Self {
        let settings = RwSignal::new(None::<Settings>);
        let system_tag = window().navigator().language();
        let system_tag_missing = system_tag.is_none();
        let locale = Memo::new(move |_| {
            settings
                .with(|s| s.as_ref().map_or(Language::System, |s| s.language))
                .resolve(system_tag.as_deref())
        });
        let state = Self {
            toasts,
            queue: Queue::new(),
            settings,
            settings_error: RwSignal::new(None),
            history_rev: RwSignal::new(0),
            update: RwSignal::new(UpdateState::Hidden),
            page: RwSignal::new(Page::Downloads),
            locale: locale.into(),
        };
        i18n::install(state, system_tag_missing);
        state
    }

    pub async fn load_settings(self) {
        match call0::<Settings>("get_settings").await {
            Ok(settings) => {
                self.settings_error.set(None);
                self.settings.set(Some(settings));
            }
            Err(e) => {
                self.toasts
                    .error((texts_now(self).settings_load_failed)(&e));
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
