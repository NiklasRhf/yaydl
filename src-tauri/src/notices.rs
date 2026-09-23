use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Runtime};
use tracing::{error, info, warn};
use yaydl_shared::{events, Notice, NoticeLevel, YtDlpError, YtDlpUpdateEvent};

use crate::i18n::Texts;

#[derive(Default)]
struct State {
    ui_ready: bool,
    buffered: Vec<Notice>,
}

/// Routes notices to the webview. Anything raised before the webview asked for
/// its startup notices is buffered, because an event emitted before the UI
/// registered its listener is lost.
#[derive(Default)]
pub struct Notices {
    state: Mutex<State>,
}

impl Notices {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notify<R: Runtime>(&self, app: &AppHandle<R>, notice: Notice) {
        if let Some(notice) = self.route(notice) {
            if let Err(e) = app.emit(events::NOTICE, &notice) {
                error!(error = %e, ?notice, "emitting a notice failed");
            }
        }
    }

    /// Logs the notice and returns it when it has to be emitted now, `None`
    /// when it was buffered for `take_startup`.
    pub fn route(&self, notice: Notice) -> Option<Notice> {
        match notice.level {
            NoticeLevel::Info | NoticeLevel::Success => {
                info!(level = ?notice.level, text = %notice.text, "notice")
            }
            NoticeLevel::Warning => warn!(text = %notice.text, "notice"),
            NoticeLevel::Error => error!(text = %notice.text, "notice"),
        }
        let mut state = self.state.lock().expect("notices lock poisoned");
        if state.ui_ready {
            Some(notice)
        } else {
            state.buffered.push(notice);
            None
        }
    }

    pub fn take_startup(&self) -> Vec<Notice> {
        let mut state = self.state.lock().expect("notices lock poisoned");
        if state.ui_ready {
            // A webview reload calls this again. Everything since the first call
            // was emitted live, so there is nothing left to return.
            info!("take_startup_notices called again after the UI was ready");
        }
        state.ui_ready = true;
        std::mem::take(&mut state.buffered)
    }
}

pub fn notice(level: NoticeLevel, text: impl Into<String>) -> Notice {
    Notice {
        level,
        text: text.into(),
    }
}

/// `AlreadyCurrent` and a skipped check are not worth a toast.
pub fn startup_update_notice(
    result: &Result<Option<YtDlpUpdateEvent>, YtDlpError>,
    texts: &Texts,
) -> Option<Notice> {
    match result {
        Ok(Some(YtDlpUpdateEvent::Updated { from, to, channel })) => Some(notice(
            NoticeLevel::Success,
            (texts.ytdlp_updated)(from, to, *channel),
        )),
        Ok(Some(YtDlpUpdateEvent::Failed { message })) => Some(notice(
            NoticeLevel::Error,
            (texts.ytdlp_update_failed)(message),
        )),
        Ok(Some(YtDlpUpdateEvent::AlreadyCurrent { .. })) | Ok(None) => None,
        Err(e) => Some(notice(NoticeLevel::Error, (texts.ytdlp_update_failed)(e))),
    }
}

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;
    use yaydl_shared::YtDlpChannel;

    use crate::i18n::{DE, EN};

    #[test]
    fn notices_are_buffered_until_the_ui_takes_them() {
        let notices = Notices::new();
        assert_eq!(notices.route(notice(NoticeLevel::Warning, "early")), None);

        let taken = notices.take_startup();
        assert_eq!(taken, vec![notice(NoticeLevel::Warning, "early")]);

        let late = notice(NoticeLevel::Info, "late");
        assert_eq!(notices.route(late.clone()), Some(late));
        assert!(notices.take_startup().is_empty());
    }

    #[test]
    fn startup_update_notices() {
        let updated = Ok(Some(YtDlpUpdateEvent::Updated {
            from: "2026.01.01".into(),
            to: "2026.09.01".into(),
            channel: YtDlpChannel::Nightly,
        }));
        assert_eq!(
            startup_update_notice(&updated, &EN),
            Some(notice(
                NoticeLevel::Success,
                "yt-dlp was updated from 2026.01.01 to 2026.09.01 (nightly)."
            ))
        );
        assert_eq!(
            startup_update_notice(&updated, &DE),
            Some(notice(
                NoticeLevel::Success,
                "yt-dlp wurde von 2026.01.01 auf 2026.09.01 aktualisiert (nightly)."
            ))
        );

        let current = Ok(Some(YtDlpUpdateEvent::AlreadyCurrent {
            version: "2026.09.01".into(),
            channel: YtDlpChannel::Nightly,
        }));
        assert_eq!(startup_update_notice(&current, &EN), None);
        assert_eq!(startup_update_notice(&Ok(None), &EN), None);

        let failed = Err(YtDlpError::Io("disk gone".into()));
        let n = startup_update_notice(&failed, &DE).expect("a failure raises a notice");
        assert_eq!(n.level, NoticeLevel::Error);
        assert!(n
            .text
            .starts_with("Das Update von yt-dlp ist fehlgeschlagen"));
        assert!(n.text.contains("disk gone"));
    }
}
