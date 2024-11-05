use leptos::{
    component, create_rw_signal, provide_context, use_context, view, For, IntoView, RwSignal,
    SignalGet, SignalUpdate,
};
use leptos_icons::Icon;
use std::sync::Arc;

#[derive(Clone)]
pub enum NotificationType {
    Info,
    Warning,
    Error,
    Success,
}

#[derive(Clone)]
pub struct Notification {
    pub text: String,
    pub notification_type: NotificationType,
}

#[derive(Clone, Default)]
pub struct NotificationContext {
    pub queue: Arc<RwSignal<Vec<Notification>>>,
}

impl NotificationContext {
    pub fn add_notification(&self, notification: Notification) {
        self.queue.update(|queue| queue.push(notification));
    }

    pub fn remove_notification(&self) {
        self.queue.update(|queue| {
            queue.remove(0);
        });
    }
}

pub fn provide_notification_context() -> NotificationContext {
    let notification_context = NotificationContext {
        queue: Arc::new(create_rw_signal(Vec::new())),
    };
    provide_context(notification_context.clone());
    notification_context
}

#[component]
pub fn NotificationList() -> impl IntoView {
    let notification_context = use_context::<NotificationContext>().unwrap();

    view! {
        <For
            each=move || notification_context.queue.get().clone()
            key=|notification| notification.text.clone()
            children=move |notification| view! {
                <NotificationDisplay notification=notification.clone() />
            }
        />
    }
}

#[component]
pub fn NotificationDisplay(notification: Notification) -> impl IntoView {
    let notification_context = use_context::<NotificationContext>().unwrap();

    let bg_color_class = match notification.notification_type {
        NotificationType::Info => "bg-blue-300",
        NotificationType::Warning => "bg-orange-300",
        NotificationType::Error => "bg-red-300",
        NotificationType::Success => "bg-green-600",
    };

    view! {
        <div
            class=format!("rounded absolute bottom-2 right-2 flex items-center {} text-gray-600 text-sm font-bold px-4 py-1 space-x-2 shadow-md", bg_color_class)
            role="alert"
        >
            { match notification.notification_type {
                NotificationType::Info => view! { <Icon icon=icondata::AiInfoCircleOutlined class="h-6 w-6 fill-white"/> },
                NotificationType::Warning => view! { <Icon icon=icondata::LuAlertTriangle class="h-6 w-6 fill-yellow-500"/> },
                NotificationType::Error => view! { <Icon icon=icondata::BiErrorCircleRegular class="h-6 w-6 fill-red-500"/> },
                NotificationType::Success => view! { <Icon icon=icondata::AiCheckOutlined class="h-6 w-6 fill-green-500"/> },
            }}
            <p>{notification.text}</p>
            <button
                on:click=move |_| notification_context.remove_notification()
            >
                <Icon icon=icondata::CgClose class="h-4 w-4 text-gray-500 hover:text-gray-600"/>
            </button>
        </div>
    }
}
