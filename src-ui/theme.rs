use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use yaydl_shared::Theme;

use crate::i18n::{texts_now, Texts};
use crate::ipc::log_to_backend;
use crate::state::AppState;

/// Keeps the `dark` class on the root element in sync with the theme setting
/// and, for `System`, with the OS preference.
pub fn install(state: AppState) {
    let system_dark = RwSignal::new(false);
    match window().match_media("(prefers-color-scheme: dark)") {
        Ok(Some(query)) => {
            system_dark.set(query.matches());
            let on_change = Closure::<dyn Fn(web_sys::MediaQueryListEvent)>::new(
                move |event: web_sys::MediaQueryListEvent| system_dark.set(event.matches()),
            );
            match query
                .add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref())
            {
                // The listener lives as long as the webview.
                Ok(()) => on_change.forget(),
                Err(e) => report(state, |t| (t.theme_listen_failed)(&format!("{e:?}"))),
            }
        }
        Ok(None) => report(state, |t| t.theme_unsupported.to_string()),
        Err(e) => report(state, |t| (t.theme_query_failed)(&format!("{e:?}"))),
    }

    Effect::new(move |_| {
        let theme = state
            .settings
            .with(|s| s.as_ref().map_or(Theme::System, |s| s.theme));
        let dark = match theme {
            Theme::System => system_dark.get(),
            Theme::Light => false,
            Theme::Dark => true,
        };
        let Some(root) = document().document_element() else {
            report(state, |t| {
                (t.theme_apply_failed)("the document has no root element")
            });
            return;
        };
        if let Err(e) = root.class_list().toggle_with_force("dark", dark) {
            report(state, |t| (t.theme_apply_failed)(&format!("{e:?}")));
        }
    });
}

fn report(state: AppState, message: impl FnOnce(&Texts) -> String) {
    let message = message(texts_now(state));
    log_to_backend("warn", message.clone());
    state.toasts.warning(message);
}
