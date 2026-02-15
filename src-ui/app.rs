use crate::notification::{
    provide_notification_context, Notification, NotificationContext, NotificationList,
    NotificationType,
};
use crate::update_modal::UpdateModal;
use crate::update_context::provide_update_context;
use leptos::*;
use leptos_icons::Icon;
use serde::Deserialize;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use yaydl_shared::{
    AddLinkError, Download, DownloadEvent, DownloadState, DownloadStateArgs, Metadata, MetadataArgs, Settings, YaydlError
};
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    async fn invoke_without_args(cmd: &str) -> JsValue;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], js_name = invoke)]
    async fn invoke_with_args(cmd: &str, args: JsValue) -> JsValue;
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "event"])]
    async fn listen(event: &str, handler: &js_sys::Function) -> JsValue;
}
#[derive(Debug, Deserialize)]
struct Event {
    #[allow(dead_code)]
    id: u8,
    #[allow(dead_code)]
    event: String,
    payload: EventType,
}
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum EventType {
    Download(DownloadEvent),
    #[allow(dead_code)]
    SomethingOtherEvent,
}
#[component]
pub fn SideBar<F>(set_main_state: F) -> impl IntoView
where
    F: Fn(MainState) + Copy + 'static,
{
    view! {
        <div class="bg-blue-400 w-20 h-full flex flex-col items-center py-2">
            <div class="">
                <div
                    class="h-14 w-14 hover:bg-blue-200 flex flex-col items-center justify-center"
                    on:click=move |_| set_main_state(MainState::Download)
                >
                    <a href="#">
                        <Icon icon=icondata::FaDownloadSolid style="color: black" class="h-8 w-8"/>
                    </a>
                </div>
                // <div
                //     class="h-14 w-14 hover:bg-blue-200 flex flex-col items-center justify-center"
                //     on:click=move |_| set_main_state(MainState::Statistics)
                // >
                //     <a href="#">
                //         <Icon icon=icondata::BsFileBarGraphFill style="color: black" class="h-8 w-8"/>
                //     </a>
                // </div>
            </div>
            <div class="flex-grow"></div>
            <div
                class="h-14 w-14 hover:bg-blue-200 flex flex-col items-center justify-center"
                on:click=move |_| set_main_state(MainState::Settings)
            >
                <a href="#">
                    <Icon icon=icondata::IoSettingsSharp style="color: black" class="h-8 w-8"/>
                </a>
            </div>
            <div class="h-14 w-14 hover:bg-blue-200 flex flex-col items-center justify-center">
                <a href="https://github.com/NiklasRhf/yaydl" target="_blank">
                    <Icon icon=icondata::AiGithubFilled class="h-8 w-8 text-black-800"/>
                </a>
            </div>
        </div>
    }
}
#[derive(Clone)]
pub enum MainState {
    Settings,
    Download,
    #[allow(dead_code)]
    Statistics,
}
#[component()]
pub fn MainContent<F>(downloads: RwSignal<Vec<Download>>, update_download_state: F) -> impl IntoView
where
    F: Fn(String, DownloadState) + Copy + 'static,
{
    let add = move |_| {
        spawn_local(async move {
            match invoke("try_add", JsValue::NULL).await {
                Ok(url) => {
                    let (url, dls): (String, Vec<Download>) =
                        serde_wasm_bindgen::from_value(url.clone()).unwrap();
                    downloads.set(dls);
                    let args =
                        serde_wasm_bindgen::to_value(&MetadataArgs { url: &url, id: "" }).unwrap();
                    update_download_state(url.to_string(), DownloadState::MetadataLoading);
                    match invoke("retreive_metadata", args).await {
                        Ok(js_val) => {
                            let metadata: Metadata = serde_wasm_bindgen::from_value(js_val).unwrap();
                            let mut updated_downloads = downloads.get_untracked().clone();
                            let latest_download = &mut updated_downloads[0];
                            latest_download.metadata = metadata;
                            downloads.set(updated_downloads);
                        }
                        Err(js_val) => {
                            let err: YaydlError = serde_wasm_bindgen::from_value(js_val).unwrap();
                            let notification_context =
                                use_context::<NotificationContext>().unwrap();
                            notification_context.add_notification(Notification {
                                text: err.to_string(),
                                notification_type: NotificationType::Error,
                            });
                        }
                    }
                }
                Err(err) => {
                    let err: YaydlError = serde_wasm_bindgen::from_value(err.clone()).unwrap();
                    let notification_context = use_context::<NotificationContext>().unwrap();
                    let notification_type = if let YaydlError::AddLinkError(ref add_err) = err {
                        Some(match add_err {
                            AddLinkError::AlreadyAdded => NotificationType::Info,
                            AddLinkError::NoValidLink => NotificationType::Warning,
                            AddLinkError::ClipboardRead => NotificationType::Error,
                        })
                    } else {
                        None
                    };
                    if let Some(notification) = notification_type {
                        notification_context.add_notification(Notification {
                            text: err.to_string(),
                            notification_type: notification,
                        });
                    }
                }
            }
        });
    };
    let clear = move |_| {
        spawn_local(async move {
            invoke_without_args("clear_downloads").await;
            downloads.set(vec![]);
        });
    };
    let download_all = move |_| {
        let downloads = downloads.get_untracked();
        spawn_local(async move {
            for download in downloads {
                update_download_state(download.metadata.id.clone(), DownloadState::Loading(0));
                let args = serde_wasm_bindgen::to_value(&MetadataArgs {
                    url: &download.metadata.url,
                    id: &download.metadata.id,
                })
                .unwrap();
                match invoke("execute_yt_dl", args).await {
                    Ok(_) => {
                        update_download_state(
                            download.metadata.id.clone(),
                            DownloadState::Finished,
                        );
                    }
                    Err(js_val) => {
                        let err: YaydlError = serde_wasm_bindgen::from_value(js_val).unwrap();
                        let notification_context = use_context::<NotificationContext>().unwrap();
                        notification_context.add_notification(Notification {
                            text: err.to_string(),
                            notification_type: NotificationType::Error,
                        });
                        update_download_state(download.metadata.id.clone(), DownloadState::Failure);
                    }
                }
            }
        });
    };
    let open_explorer = move |_| {
        spawn_local(async move {
            if let Err(err) = invoke("open_explorer", JsValue::NULL).await {
                let err: YaydlError = serde_wasm_bindgen::from_value(err).unwrap();
                let notification_context = use_context::<NotificationContext>().unwrap();
                notification_context.add_notification(Notification {
                    text: err.to_string(),
                    notification_type: NotificationType::Error,
                });
            }
        });
    };
    view! {
        <div class="flex items-center h-12 p-2 bg-gray-300 space-x-1">
            // <div class="relative inline-block border-b border-dotted border-black tooltip">
            //   Hover over me
            //   <span class="invisible absolute z-10 p-2 text-white bg-gray-700 rounded-md transition-opacity duration-300 opacity-0 w-28 bottom-full left-1/2 transform -translate-x-1/2 mb-2 tooltiptext">
            //     Tooltip text
            //     <span class="absolute top-full left-1/2 transform -translate-x-1/2 -mt-px border-[5px] border-transparent border-t-gray-700"></span>
            //   </span>
            // </div>
            <button>
                <Icon on:click=add icon=icondata::AiPlusSquareOutlined class="h-8 w-8 fill-gray-500 hover:fill-gray-600"/>
            </button>
            <button>
                <Icon on:click=clear icon=icondata::AiClearOutlined class="h-8 w-8 fill-gray-500 hover:fill-gray-600"/>
            </button>
            <div class="flex-grow"></div>
            <button on:click=open_explorer class="h-8 w-8">
                <Icon icon=icondata::AiFolderOpenFilled class="h-full w-full text-gray-500 hover:text-gray-600" />
            </button>
            <button>
                <Icon on:click=download_all icon=icondata::LuDownload class="h-8 w-8 text-gray-500 hover:text-gray-600" />
            </button>
        </div>
        <div class="flex-1 p-[5px] overflow-auto">
            <ul>
                {move || downloads.get().into_iter()
                    .map(|d| view! {
                        <li>
                            <Download d update_download_state />
                        </li>
                    })
                    .collect_view()
                }
            </ul>
        </div>
    }
}
#[component]
pub fn Download<F>(d: Download, update_download_state: F) -> impl IntoView
where
    F: Fn(String, DownloadState) + Clone + Copy + 'static,
{
    let (download, _set_download) = create_signal(d);
    let download_f = move |_| {
        let download_tmp = download.get().clone();
        update_download_state(download_tmp.metadata.id.clone(), DownloadState::Loading(0));
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&MetadataArgs {
                url: &download_tmp.metadata.url,
                id: &download_tmp.metadata.id,
            })
            .unwrap();
            match invoke("execute_yt_dl", args).await {
                Ok(_) => {
                    update_download_state(
                        download_tmp.metadata.id.clone(),
                        DownloadState::Finished,
                    );
                }
                Err(js_val) => {
                    let err: YaydlError = serde_wasm_bindgen::from_value(js_val).unwrap();
                    let notification_context = use_context::<NotificationContext>().unwrap();
                    notification_context.add_notification(Notification {
                        text: err.to_string(),
                        notification_type: NotificationType::Error,
                    });
                    update_download_state(download_tmp.metadata.id.clone(), DownloadState::Failure);
                }
            }
        });
    };
    view! {
        <div class="flex h-16 rounded border-2 border-gray-400 mb-2 space-x-4 items-center px-1 shadow-md">
            { if download.get_untracked().metadata.loading {
                view! {
                    <div class="animate-pulse space-x-4 rtl:space-x-reverse md:flex w-full">
                        <div class="flex items-center justify-center w-24 h-12 bg-gray-400 rounded">
                            <Icon icon=icondata::BiImageRegular class="h-8 w-8 text-gray-500" />
                        </div>
                        <div class="w-full flex flex-col justify-center">
                            <div class="h-2.5 bg-gray-400 rounded-full w-10/12 mb-2.5"></div>
                            <div class="h-2.5 bg-gray-400 rounded-full w-32 mb-2.5"></div>
                        </div>
                    </div>
                }.into_view()
            } else {
                view! {
                    <img
                        src={&download.get_untracked().metadata.thumbnail}
                        alt={&download.get_untracked().metadata.thumbnail}
                        class="h-12 w-20 rounded shadow-sm"
                    />
                    <div class="w-full">
                        <p class="line-clamp-1">{&download.get_untracked().metadata.title}</p>
                        <p class="text-sm">{&download.get_untracked().metadata.duration}</p>
                    </div>
                    {move || {
                        match download.get_untracked().download_state {
                            DownloadState::Idle => {
                                view! {
                                    <button on:click=download_f class="h-10 w-10">
                                        <Icon icon=icondata::BiDownloadSolid class="h-full w-full text-gray-600 hover:text-gray-800"/>
                                    </button>
                                }.into_view()
                            }
                            DownloadState::Loading(progress) => {
                                { if progress == 0 {
                                    view! {
                                        <Icon icon=icondata::CgSpinner class="w-10 h-10 animate-spin text-gray-600" />
                                    }.into_view()
                                } else {
                                    view! {
                                        <div class="relative w-10 h-10">
                                          <svg class="w-full h-full" viewBox="0 0 100 100">
                                            <circle
                                              class="text-gray-400 stroke-current"
                                              stroke-width="15"
                                              cx="50"
                                              cy="50"
                                              r="40"
                                              fill="transparent"
                                            ></circle>
                                            <circle
                                              class="text-blue-500  progress-ring__circle stroke-current"
                                              stroke-width="15"
                                              stroke-linecap="round"
                                              cx="50"
                                              cy="50"
                                              r="40"
                                              fill="transparent"
                                              stroke-dasharray="251.2"
                                              stroke-dashoffset=format!("calc(251.2px - (251.2px * {progress}) / 100)")
                                            ></circle>
                                            <text x="50" y="50" font-size="26" text-anchor="middle" alignment-baseline="middle">{progress}%</text>
                                          </svg>
                                        </div>
                                   }.into_view()
                                }}
                            }
                            DownloadState::Finished => {
                                view! {
                                    <Icon icon=icondata::AiCheckCircleTwotone class="h-10 w-10 fill-green-600 stroke-green-600" style="stroke-width: 2%" />
                               }.into_view()
                            }
                            DownloadState::Failure => {
                                view! {
                                    <Icon icon=icondata::BiErrorCircleRegular class="h-10 w-10 fill-red-600 stroke-red-600 stroke-[0.5px]" />
                               }.into_view()
                            }
                            _ => {}.into_view()
                        }
                    }}
                }.into_view()
            }}
        </div>
    }
}
#[component]
pub fn Settings() -> impl IntoView {
    let (output_dir, set_output_dir) = create_signal(String::new());
    create_effect(move |_| {
        spawn_local(async move {
            if let Ok(js_val) = invoke("get_settings", JsValue::NULL).await {
                let settings: Settings = serde_wasm_bindgen::from_value(js_val).unwrap();
                set_output_dir.set(settings.output_dir.display().to_string());
            }
        });
    });
    let get_output_dir = move |_| {
        spawn_local(async move {
            if let Ok(js_val) = invoke("choose_output_dir", JsValue::NULL).await {
                let notification_context = use_context::<NotificationContext>().unwrap();
                let path = serde_wasm_bindgen::from_value(js_val).unwrap();
                set_output_dir.set(path);
                notification_context.add_notification(Notification {
                    text: "Output directory updated successfully".into(),
                    notification_type: NotificationType::Success,
                });
            }
        });
    };
    view! {
        <div class="flex items-center justify-center h-12 p-2 bg-gray-300">
        </div>
        <div class="flex flex-col h-full p-2">
            <h2 class="text-xl">Settings</h2>
            <span class="h-2 w-full border-2"></span>
            <br />
            <div class="flex space-x-1 items-center">
                <button on:click=get_output_dir class="border-2 border-gray-500 h-8 w-52 rounded-md bg-gray-400 hover:bg-gray-500 shadow-md">
                    "Set output directory"
                </button>
                <p class="bg-blue-300 p-1 rounded-md w-full">{output_dir}</p>
            </div>
        </div>
    }
}
#[component]
pub fn Statistics() -> impl IntoView {
    view! {
        <div class="flex items-center justify-center h-12 p-2 bg-gray-300">
            <p>Statistics buttons here</p>
        </div>
        <div class="flex w-full h-full p-2">
            <p>Statistics here</p>
        </div>
    }
}
#[component]
pub fn App() -> impl IntoView {
    let (state, set_state) = create_signal(MainState::Download);
    let downloads = create_rw_signal(vec![]);
    let _ = provide_notification_context();
    let update_context = provide_update_context();
    let update_context2 = update_context.clone();
    let update_context3 = update_context.clone();
    let (show_update, set_show_update) = create_signal(update_context.state.get().show);
    let (update_progress, set_update_progress) = create_signal(update_context.state.get().progress);
    create_effect(move |_| {
        set_show_update.set(update_context.state.get().show);
        set_update_progress.set(update_context.state.get().progress);
    });
    // Listen for update events from backend
    create_effect(move |_| {
        let update_context = update_context2.clone();
        let closure = Closure::<dyn FnMut(_)>::new(move |percent: JsValue| {
            let percent: u8 = serde_wasm_bindgen::from_value(percent).unwrap_or(0);
            update_context.state.update(|s| {
                s.progress = Some(percent);
            });
        });
        let finished_closure = Closure::<dyn FnMut(JsValue)>::new(move |_| {
            update_context.state.update(|s| {
                s.progress = Some(100);
            });
        });
        spawn_local(async move {
            listen("update-progress", closure.as_ref().unchecked_ref()).await;
            listen("update-finished", finished_closure.as_ref().unchecked_ref()).await;
            closure.forget();
            finished_closure.forget();
        });
    });
    // On mount, check for update
    create_effect(move |_| {
        let update_context = update_context3.clone();
        spawn_local(async move {
            let has_update = invoke_without_args("check_update").await;
            let has_update: bool = serde_wasm_bindgen::from_value(has_update).unwrap_or(false);
            if has_update {
                update_context.state.update(|s| s.show = true);
            }
        });
    });
    let set_main_state = move |state: MainState| {
        set_state.set(state);
    };
    let update_download_state = move |id: String, state: DownloadState| {
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&DownloadStateArgs {
                id: id.clone(),
                state: state.clone(),
            })
            .unwrap();
            invoke_with_args("update_download", args).await;
            let mut dls: Vec<Download> = downloads.get_untracked().clone();
            if let Some(download) = dls.iter_mut().find(|d| d.metadata.id == id || d.metadata.url == id) {
                if let DownloadState::MetadataLoading = state {
                    download.metadata.loading = !download.metadata.loading;
                } else {
                    download.download_state = state;
                }
            }
            downloads.set(dls);
        });
    };
    create_effect(move |_| {
        let closure = Closure::<dyn FnMut(_)>::new(move |s: JsValue| {
            let event: Event = serde_wasm_bindgen::from_value(s).unwrap();
            if let EventType::Download(d_ev) = event.payload {
                update_download_state(d_ev.id, DownloadState::Loading(d_ev.progress));
            }
        });
        spawn_local(async move {
            listen("download-progress", closure.as_ref().unchecked_ref()).await;
            closure.forget();
        });
    });
    view! {
        <main class="h-screen bg-gray-200 flex">
            <SideBar set_main_state />
            <div class="flex-1">
                <div class="flex flex-col h-full">
                    <div class="flex items-center justify-center h-12 p-4">
                        <h1 class="flex text-2xl font-bold space-x-1 items-center">
                            <p>Yet Another YouTube</p>
                            <a href="https://www.youtube.com" target="_blank">
                                <Icon icon=icondata::AiYoutubeFilled class="h-8 w-8 text-red-600 hover:text-red-700"/>
                            </a>
                            <p>Downloader</p>
                        </h1>
                    </div>
                    { move || match state.get() {
                        MainState::Download => view! { <MainContent downloads update_download_state /> }.into_view(),
                        MainState::Statistics => view! { <Statistics /> }.into_view(),
                        MainState::Settings => view! { <Settings /> }.into_view(),
                    }}
                </div>
            </div>
            <UpdateModal
                show=show_update.into()
                progress=update_progress.into()
                on_update=Callback::new(move |_| {
                    let update_context = update_context.clone();
                    update_context.state.update(|s| s.progress = Some(0));
                    spawn_local(async move {
                        let _ = invoke_without_args("start_update").await;
                    });
                })
                on_quit=Callback::new(move |_| {
                    spawn_local(async move {
                        invoke_without_args("quit_app").await;
                    });
                })
            />
            <NotificationList />
        </main>
    }
}
