use leptos::task::spawn_local;
use serde::{de::DeserializeOwned, Serialize};
use wasm_bindgen::prelude::*;
use yaydl_shared::{LogArgs, YaydlError};

use crate::toast::Toasts;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke, catch)]
    async fn tauri_invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"], js_name = listen, catch)]
    async fn tauri_listen(event: &str, handler: &js_sys::Function) -> Result<JsValue, JsValue>;
}

fn to_js<A: Serialize + ?Sized>(value: &A) -> Result<JsValue, String> {
    // json_compatible so `None` becomes null and maps become plain objects,
    // which is what Tauri's JSON deserializer on the other side expects.
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| e.to_string())
}

fn rejection_text(value: &JsValue) -> String {
    if let Ok(error) = serde_wasm_bindgen::from_value::<YaydlError>(value.clone()) {
        return error.to_string();
    }
    match value.as_string() {
        Some(text) => text,
        None => format!("{value:?}"),
    }
}

async fn invoke_raw<R: DeserializeOwned>(cmd: &str, args: JsValue) -> Result<R, String> {
    let value = tauri_invoke(cmd, args)
        .await
        .map_err(|e| rejection_text(&e))?;
    serde_wasm_bindgen::from_value::<R>(value.clone()).map_err(|e| {
        let message = format!("{cmd} returned a value the UI cannot read: {e} (got {value:?})");
        log_to_backend("error", message.clone());
        message
    })
}

pub async fn call<A: Serialize, R: DeserializeOwned>(cmd: &str, args: &A) -> Result<R, String> {
    let args = to_js(args).map_err(|e| {
        let message = format!("encoding the arguments of {cmd} failed: {e}");
        log_to_backend("error", message.clone());
        message
    })?;
    invoke_raw(cmd, args).await
}

pub async fn call0<R: DeserializeOwned>(cmd: &str) -> Result<R, String> {
    invoke_raw(cmd, js_sys::Object::new().into()).await
}

/// Runs a command whose only interesting outcome is failure, which becomes an
/// error toast.
pub fn fire<A: Serialize + 'static>(toasts: Toasts, cmd: &'static str, args: A) {
    spawn_local(async move {
        if let Err(e) = call::<A, ()>(cmd, &args).await {
            toasts.error(e);
        }
    });
}

pub fn fire0(toasts: Toasts, cmd: &'static str) {
    spawn_local(async move {
        if let Err(e) = call0::<()>(cmd).await {
            toasts.error(e);
        }
    });
}

/// Registers an app-lifetime listener. The handler closure is leaked on
/// purpose, because the listener must live as long as the webview.
pub async fn listen<T, F>(event: &'static str, toasts: Toasts, handler: F) -> Result<(), String>
where
    T: DeserializeOwned + 'static,
    F: Fn(T) + 'static,
{
    let closure = Closure::<dyn Fn(JsValue)>::new(move |envelope: JsValue| {
        let payload = match js_sys::Reflect::get(&envelope, &JsValue::from_str("payload")) {
            Ok(payload) => payload,
            Err(e) => {
                let message = format!("the {event} event has no payload: {e:?}");
                log_to_backend("error", message.clone());
                toasts.error(message);
                return;
            }
        };
        match serde_wasm_bindgen::from_value::<T>(payload.clone()) {
            Ok(value) => handler(value),
            Err(e) => {
                let message = format!(
                    "the {event} event carried data the UI cannot read: {e} (got {payload:?})"
                );
                log_to_backend("error", message.clone());
                toasts.error(message);
            }
        }
    });
    tauri_listen(event, closure.as_ref().unchecked_ref())
        .await
        .map_err(|e| format!("listening for {event} failed: {}", rejection_text(&e)))?;
    closure.forget();
    Ok(())
}

/// Mirrors webview activity into the backend log file. A failure can only go to
/// the devtools console, because reporting it through the backend is what failed.
pub fn log_to_backend(level: &'static str, message: String) {
    match level {
        "error" => web_sys::console::error_1(&JsValue::from_str(&message)),
        "warn" => web_sys::console::warn_1(&JsValue::from_str(&message)),
        _ => web_sys::console::log_1(&JsValue::from_str(&message)),
    }
    // Goes through the raw binding, because `call` reports its own failures here
    // and would recurse.
    spawn_local(async move {
        let outcome = match to_js(&LogArgs { level, message }) {
            Ok(args) => tauri_invoke("ui_log", args)
                .await
                .map(|_| ())
                .map_err(|e| rejection_text(&e)),
            Err(e) => Err(format!("encoding failed: {e}")),
        };
        if let Err(e) = outcome {
            web_sys::console::error_1(&JsValue::from_str(&format!(
                "forwarding a log line to the backend failed: {e}"
            )));
        }
    });
}
