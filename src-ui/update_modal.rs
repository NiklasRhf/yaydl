use leptos::*;

#[component]
pub fn UpdateModal(show: ReadSignal<bool>, progress: ReadSignal<Option<u8>>, on_update: Callback<()>, on_quit: Callback<()>) -> impl IntoView {
    view! {
        <Show when=move || show.get()>
            <div class="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
                <div class="bg-white rounded-lg shadow-lg p-8 flex flex-col items-center min-w-[350px]">
                    <h2 class="text-xl font-bold mb-4">Update Required</h2>
                    <p class="mb-4 text-center">A new version of yaydl is available and required to continue using the app.</p>
                    <Show when=move || progress.get().is_some()>
                        <div class="w-full mb-4">
                            <div class="w-full bg-gray-200 rounded-full h-4">
                                <div class="bg-blue-500 h-4 rounded-full transition-all duration-300" style=move || format!("width: {}%", progress.get().unwrap_or(0))></div>
                            </div>
                            <div class="text-center text-sm mt-1">{move || format!("{}%", progress.get().unwrap_or(0))}</div>
                        </div>
                    </Show>
                    <div class="flex space-x-4 mt-4">
                        <button class="bg-blue-600 text-white px-4 py-2 rounded hover:bg-blue-700 font-semibold" on:click=move |_| on_update.call(()) disabled=move || progress.get().is_some()>
                            {move || if progress.get().is_some() { "Updating...".to_string() } else { "Update now".to_string() }}
                        </button>
                        <button class="bg-gray-300 text-gray-800 px-4 py-2 rounded hover:bg-gray-400 font-semibold" on:click=move |_| on_quit.call(()) disabled=move || progress.get().is_some()>
                            Quit
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}
