use leptos::*;

#[derive(Clone, Default)]
pub struct UpdateState {
    pub show: bool,
    pub progress: Option<u8>,
}

#[derive(Clone, Default)]
pub struct UpdateContext {
    pub state: RwSignal<UpdateState>,
}

pub fn provide_update_context() -> UpdateContext {
    let update_context = UpdateContext {
        state: create_rw_signal(UpdateState::default()),
    };
    provide_context(update_context.clone());
    update_context
}
