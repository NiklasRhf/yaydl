use crate::notification::{
    provide_notification_context, Notification, NotificationContext, NotificationList,
    NotificationType,
};
use leptos::*;
use leptos_icons::Icon;
use wasm_bindgen::prelude::*;
use yaydl_shared::{Settings, Metadata, MetadataArgs, Download, DownloadState};

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

#[component]
pub fn SideBar<F>(set_main_state: F) -> impl IntoView
where
    F: Fn(MainState) + Clone + Copy + 'static,
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
pub fn MainContent() -> impl IntoView {
    // TODO: remove me
    // let download_dummy = Download {
    //     metadata: Metadata {
    //         url: "https://www.youtube.com/watch?v=cBSUf04SHYY".into(),
    //         thumbnail: "https://i.ytimg.com/vi_webp/cBSUf04SHYY/maxresdefault.webp".into(),
    //         duration: "1:00:00".into(),
    //         title: "Utopia - Calming Ethereal Ambient Music - Deep Meditation and Relaxation"
    //             .into(),
    //         loading: false,
    //         ..Default::default()
    //     },
    //     ..Default::default()
    // };
    // let download_dummy_2 = Download {
    //     metadata: Metadata {
    //         url: "".into(),
    //         ..Default::default()
    //     },
    //     ..Default::default()
    // };
    // let download_dummy_3 = Download {
    //     metadata: Metadata {
    //         url: "1".into(),
    //         ..Default::default()
    //     },
    //     download_state: DownloadState::Loading,
    //     ..Default::default()
    // };
    // let download_dummy_4 = Download {
    //     metadata: Metadata {
    //         url: "1".into(),
    //         ..Default::default()
    //     },
    //     download_state: DownloadState::Finished,
    //     ..Default::default()
    // };
    // let download_dummy_5 = Download {
    //     metadata: Metadata {
    //         url: "1".into(),
    //         ..Default::default()
    //     },
    //     download_state: DownloadState::Failure,
    //     ..Default::default()
    // };
    // let download_dummy_6 = Download {
    //     metadata: Metadata {
    //         url: "1".into(),
    //         ..Default::default()
    //     },
    //     download_state: DownloadState::Failure,
    //     ..Default::default()
    // };
    // let download_dummy_7 = Download {
    //     metadata: Metadata {
    //         url: "1".into(),
    //         ..Default::default()
    //     },
    //     download_state: DownloadState::Failure,
    //     ..Default::default()
    // };
    // let download_dummy_8 = Download {
    //     metadata: Metadata {
    //         url: "1".into(),
    //         ..Default::default()
    //     },
    //     download_state: DownloadState::Failure,
    //     ..Default::default()
    // };
    let downloads = vec![
        // download_dummy,
        // download_dummy_2,
        // download_dummy_3,
        // download_dummy_4,
        // download_dummy_5,
        // download_dummy_6,
        // download_dummy_7,
        // download_dummy_8,
    ];
    let (downloads, set_downloads) = create_signal(downloads);

    let add = move |_| {
        spawn_local(async move {
            match invoke("try_add", JsValue::NULL).await {
                Ok(url) => {
                    let url: String = serde_wasm_bindgen::from_value(url.clone()).unwrap();
                    let mut updated_downloads = downloads.get_untracked();
                    updated_downloads.insert(
                        0,
                        Download {
                            metadata: Metadata {
                                url: url.clone(),
                                loading: true,
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    );
                    set_downloads.set(updated_downloads);
                    let args = serde_wasm_bindgen::to_value(&MetadataArgs { url: &url }).unwrap();
                    match invoke("retreive_metadata", args).await {
                        Ok(js_val) => {
                            let metadata = serde_wasm_bindgen::from_value(js_val).unwrap();
                            let mut updated_downloads = downloads.get_untracked().clone();
                            let latest_download = &mut updated_downloads[0];
                            latest_download.metadata = metadata;
                            set_downloads.set(updated_downloads);
                        },
                        Err(js_val) => {
                            let err: String = serde_wasm_bindgen::from_value(js_val).unwrap();
                            let notification_context = use_context::<NotificationContext>().unwrap();
                            notification_context.add_notification(Notification {
                                text: err,
                                notification_type: NotificationType::Error,
                            });
                        },
                    }
                }
                Err(err) => {
                    let err: String = serde_wasm_bindgen::from_value(err.clone()).unwrap();
                    let notification_context = use_context::<NotificationContext>().unwrap();
                    notification_context.add_notification(Notification {
                        text: err,
                        notification_type: NotificationType::Info,
                    });
                }
            }
        });
    };

    let clear = move |_| {
        set_downloads.set(vec![]);
        // TODO: clear backend list?
    };

    let update_download_state = move |id: String, state: DownloadState| {
        let mut downloads = downloads.get_untracked().clone();
        if let Some(download) = downloads.iter_mut().find(|d| d.metadata.id == id) {
            download.download_state = state;
        }
        set_downloads.set(downloads);
    };

    let download_all = move |_| {
        let downloads = downloads.get_untracked();
        spawn_local(async move {
            for download in downloads {
                // TODO: remove me
                if download.metadata.url.is_empty()
                    || download.metadata.url == "https://www.youtube.com/watch?v=cBSUf04SHYY"
                    || download.download_state == DownloadState::Finished
                {
                    continue;
                }
                update_download_state(download.metadata.id.clone(), DownloadState::Loading);
                let args = serde_wasm_bindgen::to_value(&MetadataArgs {
                    url: &download.metadata.url,
                })
                .unwrap();
                match invoke("execute_yt_dl", args).await {
                    Ok(_) => {
                        update_download_state(download.metadata.id.clone(), DownloadState::Finished);
                    }
                    Err(_) => {
                        update_download_state(download.metadata.id.clone(), DownloadState::Failure);
                    }
                }
            }
        });
    };

    let open_explorer = move |_| {
        spawn_local(async move {
            if let Err(err) = invoke("open_explorer", JsValue::NULL).await {
                let err = serde_wasm_bindgen::from_value::<String>(err).unwrap();
                let notification_context = use_context::<NotificationContext>().unwrap();
                notification_context.add_notification(Notification {
                    text: err,
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
        update_download_state(download_tmp.metadata.id.clone(), DownloadState::Loading);
        spawn_local(async move {
            let args = serde_wasm_bindgen::to_value(&MetadataArgs {
                url: &download_tmp.metadata.url,
            })
            .unwrap();
            match invoke("execute_yt_dl", args).await {
                Ok(_) => {
                    update_download_state(download_tmp.metadata.id.clone(), DownloadState::Finished);
                }
                Err(_) => {
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
                { if download.get_untracked().metadata.url.is_empty() {
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
                                        <button on:click=download_f class="h-8 w-8">
                                            <Icon icon=icondata::BiDownloadSolid class="h-full w-full text-gray-600 hover:text-gray-800"/>
                                        </button>
                                    }.into_view()
                                }
                                DownloadState::Loading => {
                                    view! {
                                        <Icon icon=icondata::CgSpinner class="w-8 h-8 animate-spin text-gray-600" />
                                   }.into_view()
                                }
                                DownloadState::Finished => {
                                    view! {
                                        <Icon icon=icondata::AiCheckCircleTwotone class="h-8 w-8 fill-green-600 stroke-green-600" style="stroke-width: 2%" />
                                   }.into_view()
                                }
                                DownloadState::Failure => {
                                    view! {
                                        <Icon icon=icondata::BiErrorCircleRegular class="h-8 w-8 fill-red-600 stroke-red-600 stroke-[0.5px]" />
                                   }.into_view()
                                }
                            }
                        }}
                    }.into_view()
                }}
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
            match invoke("choose_output_dir", JsValue::NULL).await {
                Ok(js_val) => {
                    let path = serde_wasm_bindgen::from_value(js_val).unwrap();
                    set_output_dir.set(path);
                }
                Err(_) => {
                    // TODO
                }
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
    let _ = provide_notification_context();

    let set_main_state = move |state: MainState| {
        set_state.set(state);
    };
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
                        MainState::Download => view! { <MainContent /> }.into_view(),
                        MainState::Statistics => view! { <Statistics /> }.into_view(),
                        MainState::Settings => view! { <Settings /> }.into_view(),
                    }}
                </div>
            </div>
            <NotificationList />
        </main>
    }
}
