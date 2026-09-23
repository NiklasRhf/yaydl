use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use yaydl_shared::Theme;

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
                Err(e) => report(
                    format!("Listening for OS theme changes failed: {e:?}"),
                    state,
                ),
            }
        }
        Ok(None) => report(
            "The webview cannot report the OS theme, so System uses the light theme".into(),
            state,
        ),
        Err(e) => report(
            format!("Querying the OS theme failed, so System uses the light theme: {e:?}"),
            state,
        ),
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
            report(
                "Applying the theme failed: the document has no root element".into(),
                state,
            );
            return;
        };
        if let Err(e) = root.class_list().toggle_with_force("dark", dark) {
            report(format!("Applying the theme failed: {e:?}"), state);
        }
    });
}

fn report(message: String, state: AppState) {
    log_to_backend("warn", message.clone());
    state.toasts.warning(message);
}
