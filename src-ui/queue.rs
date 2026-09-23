use std::collections::HashMap;

use leptos::prelude::*;
use leptos::task::spawn_local;
use yaydl_shared::{DownloadId, DownloadItem, DownloadStatus};

use crate::i18n::texts_now;
use crate::ipc::{call0, log_to_backend};
use crate::state::AppState;

/// Mirror of the backend queue. The order and each item have their own signal,
/// so a progress update re-renders only the row it belongs to.
#[derive(Clone, Copy)]
pub struct Queue {
    pub order: RwSignal<Vec<DownloadId>>,
    items: StoredValue<HashMap<DownloadId, ArcRwSignal<DownloadItem>>>,
}

#[derive(Clone, Copy, Default, PartialEq)]
pub struct QueueSummary {
    pub resolving: u32,
    pub ready: u32,
    pub queued: u32,
    pub downloading: u32,
    pub finished: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub awaiting: u32,
}

impl Queue {
    pub fn new() -> Self {
        Self {
            order: RwSignal::new(Vec::new()),
            items: StoredValue::new(HashMap::new()),
        }
    }

    pub fn item(self, id: DownloadId) -> Option<ArcRwSignal<DownloadItem>> {
        self.items.with_value(|items| items.get(&id).cloned())
    }

    pub fn replace(self, list: Vec<DownloadItem>) {
        let order: Vec<DownloadId> = list.iter().map(|item| item.id).collect();
        let mut changed = Vec::new();
        // Signals are set after the map lock is released, so nothing that
        // reacts to them can observe the map half-rebuilt.
        self.items.update_value(|items| {
            let mut next = HashMap::with_capacity(list.len());
            for item in list {
                let id = item.id;
                let signal = match items.remove(&id) {
                    Some(signal) => {
                        if signal.with_untracked(|current| current != &item) {
                            changed.push((signal.clone(), item));
                        }
                        signal
                    }
                    None => ArcRwSignal::new(item),
                };
                next.insert(id, signal);
            }
            *items = next;
        });
        for (signal, item) in changed {
            signal.set(item);
        }
        self.order.set(order);
    }

    pub fn update(self, item: DownloadItem, state: AppState) {
        match self.item(item.id) {
            Some(signal) => signal.set(item),
            None => {
                log_to_backend(
                    "warn",
                    format!(
                        "queue-item-updated for unknown download {} ({}), refetching the queue",
                        item.id,
                        item.status.name()
                    ),
                );
                self.refetch(state);
            }
        }
    }

    pub fn refetch(self, state: AppState) {
        spawn_local(async move {
            match call0::<Vec<DownloadItem>>("get_queue").await {
                Ok(list) => self.replace(list),
                Err(e) => state.toasts.error((texts_now(state).queue_load_failed)(&e)),
            }
        });
    }

    /// Reads every item signal, so it re-runs on any change.
    pub fn summary(self) -> QueueSummary {
        let ids = self.order.get();
        let mut summary = QueueSummary::default();
        self.items.with_value(|items| {
            for id in &ids {
                let Some(signal) = items.get(id) else {
                    log_to_backend(
                        "error",
                        format!("queue order lists download {id} but the item map has no entry"),
                    );
                    continue;
                };
                signal.with(|item| match item.status {
                    DownloadStatus::ResolvingMetadata => summary.resolving += 1,
                    DownloadStatus::Ready => summary.ready += 1,
                    DownloadStatus::Queued => summary.queued += 1,
                    DownloadStatus::Downloading { .. } | DownloadStatus::Processing => {
                        summary.downloading += 1
                    }
                    DownloadStatus::Finished => summary.finished += 1,
                    DownloadStatus::Failed { .. } | DownloadStatus::MetadataFailed { .. } => {
                        summary.failed += 1
                    }
                    DownloadStatus::Cancelled => summary.cancelled += 1,
                    DownloadStatus::AwaitingDuplicateConfirmation => summary.awaiting += 1,
                });
            }
        });
        summary
    }
}
