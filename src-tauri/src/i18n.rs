//! Texts the backend renders itself: notices shown as toasts and the desktop
//! notification. Persisted `FriendlyError` messages stay English because the UI
//! localizes those by `ErrorKind`.

use std::{fmt::Display, path::Path};

use tracing::info;
use yaydl_shared::{DownloadId, Locale, Settings, YtDlpChannel};

/// One field per text, so a translation missing from `EN` or `DE` does not
/// compile.
pub struct Texts {
    pub settings_invalid: fn(reason: &str, moved: &Path) -> String,
    pub output_folder_missing: fn(dir: &Path) -> String,
    pub history_file_invalid: fn(reason: &str, moved: &Path) -> String,
    pub queue_file_invalid: fn(reason: &str, moved: &Path) -> String,
    pub queue_save_failed: fn(error: &dyn Display) -> String,
    pub history_write_failed: fn(error: &dyn Display) -> String,
    pub history_details_missing: fn(id: DownloadId) -> String,
    pub start_failed: fn(id: DownloadId, error: &dyn Display) -> String,
    pub playlist_skipped: fn(skipped: u32, playlist: Option<&str>) -> String,
    pub ytdlp_updated: fn(from: &str, to: &str, channel: YtDlpChannel) -> String,
    pub ytdlp_update_failed: fn(error: &dyn Display) -> String,
    pub batch_summary: fn(finished: u32, failed: u32) -> String,
}

pub const EN: Texts = Texts {
    settings_invalid: |reason, moved| {
        format!(
            "Your settings file was invalid ({reason}) and was moved to {}. yaydl started with default settings.",
            moved.display()
        )
    },
    output_folder_missing: |dir| {
        format!(
            "The output folder {} does not exist. Downloads will fail until it is back or you choose another folder in Settings.",
            dir.display()
        )
    },
    history_file_invalid: |reason, moved| {
        format!(
            "The download history was invalid ({reason}) and was moved to {}. The history starts empty.",
            moved.display()
        )
    },
    queue_file_invalid: |reason, moved| {
        format!(
            "The saved download queue was invalid ({reason}) and was moved to {}. The queue starts empty.",
            moved.display()
        )
    },
    queue_save_failed: |error| {
        format!("Saving the download queue failed, changes will be lost when yaydl closes: {error}")
    },
    history_write_failed: |error| {
        format!("The download finished, but saving it to the history failed: {error}")
    },
    history_details_missing: |id| {
        format!("Download {id} finished, but it could not be added to the history because its details are missing.")
    },
    start_failed: |id, error| format!("Starting download {id} failed: {error}"),
    playlist_skipped: |skipped, playlist| {
        let videos = if skipped == 1 { "video" } else { "videos" };
        let playlist = playlist.unwrap_or("the playlist");
        format!("Skipped {skipped} unavailable {videos} from {playlist}.")
    },
    ytdlp_updated: |from, to, channel| {
        format!("yt-dlp was updated from {from} to {to} ({channel}).")
    },
    ytdlp_update_failed: |error| format!("Updating yt-dlp failed: {error}"),
    batch_summary: |finished, failed| {
        let downloads = |n: u32| if n == 1 { "download" } else { "downloads" };
        match (finished, failed) {
            (f, 0) => format!("{f} {} finished", downloads(f)),
            (0, x) => format!("{x} {} failed", downloads(x)),
            (f, x) => format!("{f} finished, {x} failed"),
        }
    },
};

pub const DE: Texts = Texts {
    settings_invalid: |reason, moved| {
        format!(
            "Deine Einstellungsdatei war ungültig ({reason}) und wurde nach {} verschoben. yaydl wurde mit den Standardeinstellungen gestartet.",
            moved.display()
        )
    },
    output_folder_missing: |dir| {
        format!(
            "Der Zielordner {} existiert nicht. Downloads schlagen fehl, bis er wieder da ist oder du in den Einstellungen einen anderen Ordner wählst.",
            dir.display()
        )
    },
    history_file_invalid: |reason, moved| {
        format!(
            "Der Download-Verlauf war ungültig ({reason}) und wurde nach {} verschoben. Der Verlauf startet leer.",
            moved.display()
        )
    },
    queue_file_invalid: |reason, moved| {
        format!(
            "Die gespeicherte Warteschlange war ungültig ({reason}) und wurde nach {} verschoben. Die Warteschlange startet leer.",
            moved.display()
        )
    },
    queue_save_failed: |error| {
        format!("Die Warteschlange konnte nicht gespeichert werden, Änderungen gehen beim Schließen von yaydl verloren: {error}")
    },
    history_write_failed: |error| {
        format!(
            "Der Download ist fertig, aber er konnte nicht im Verlauf gespeichert werden: {error}"
        )
    },
    history_details_missing: |id| {
        format!("Download {id} ist fertig, konnte aber nicht in den Verlauf aufgenommen werden, weil seine Details fehlen.")
    },
    start_failed: |id, error| format!("Download {id} konnte nicht gestartet werden: {error}"),
    playlist_skipped: |skipped, playlist| {
        let videos = if skipped == 1 {
            "nicht verfügbares Video"
        } else {
            "nicht verfügbare Videos"
        };
        let playlist = playlist.unwrap_or("der Playlist");
        format!("{skipped} {videos} aus {playlist} übersprungen.")
    },
    ytdlp_updated: |from, to, channel| {
        format!("yt-dlp wurde von {from} auf {to} aktualisiert ({channel}).")
    },
    ytdlp_update_failed: |error| format!("Das Update von yt-dlp ist fehlgeschlagen: {error}"),
    batch_summary: |finished, failed| {
        let downloads = |n: u32| if n == 1 { "Download" } else { "Downloads" };
        match (finished, failed) {
            (f, 0) => format!("{f} {} abgeschlossen", downloads(f)),
            (0, x) => format!("{x} {} fehlgeschlagen", downloads(x)),
            (f, x) => format!("{f} abgeschlossen, {x} fehlgeschlagen"),
        }
    },
};

pub fn texts(locale: Locale) -> &'static Texts {
    match locale {
        Locale::En => &EN,
        Locale::De => &DE,
    }
}

/// Read on every render, so a language change applies to the next text.
pub fn current_locale(settings: &Settings) -> Locale {
    settings
        .language
        .resolve(sys_locale::get_locale().as_deref())
}

pub fn texts_for(settings: &Settings) -> &'static Texts {
    texts(current_locale(settings))
}

pub fn log_startup_locale(settings: &Settings) {
    let system_tag = sys_locale::get_locale();
    let locale = settings.language.resolve(system_tag.as_deref());
    info!(
        language = settings.language.as_str(),
        system_tag = ?system_tag,
        ?locale,
        "resolved the backend locale"
    );
}

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use yaydl_shared::Language;

    use crate::settings::default_settings;

    #[test]
    fn batch_summary_plurals() {
        let en = texts(Locale::En).batch_summary;
        assert_eq!(en(1, 0), "1 download finished");
        assert_eq!(en(3, 0), "3 downloads finished");
        assert_eq!(en(0, 1), "1 download failed");
        assert_eq!(en(0, 2), "2 downloads failed");
        assert_eq!(en(2, 1), "2 finished, 1 failed");

        let de = texts(Locale::De).batch_summary;
        assert_eq!(de(1, 0), "1 Download abgeschlossen");
        assert_eq!(de(3, 0), "3 Downloads abgeschlossen");
        assert_eq!(de(0, 1), "1 Download fehlgeschlagen");
        assert_eq!(de(0, 2), "2 Downloads fehlgeschlagen");
        assert_eq!(de(2, 1), "2 abgeschlossen, 1 fehlgeschlagen");
    }

    #[test]
    fn playlist_skipped_plurals() {
        let en = texts(Locale::En).playlist_skipped;
        assert_eq!(en(1, Some("Mix")), "Skipped 1 unavailable video from Mix.");
        assert_eq!(
            en(4, None),
            "Skipped 4 unavailable videos from the playlist."
        );

        let de = texts(Locale::De).playlist_skipped;
        assert_eq!(
            de(1, Some("Mix")),
            "1 nicht verfügbares Video aus Mix übersprungen."
        );
        assert_eq!(
            de(4, None),
            "4 nicht verfügbare Videos aus der Playlist übersprungen."
        );
    }

    #[test]
    fn ytdlp_update_notice() {
        let (from, to, channel) = ("2026.01.01", "2026.09.01", YtDlpChannel::Nightly);
        assert_eq!(
            (texts(Locale::En).ytdlp_updated)(from, to, channel),
            "yt-dlp was updated from 2026.01.01 to 2026.09.01 (nightly)."
        );
        assert_eq!(
            (texts(Locale::De).ytdlp_updated)(from, to, channel),
            "yt-dlp wurde von 2026.01.01 auf 2026.09.01 aktualisiert (nightly)."
        );
        assert_eq!(
            (texts(Locale::De).ytdlp_update_failed)(&"disk gone"),
            "Das Update von yt-dlp ist fehlgeschlagen: disk gone"
        );
    }

    #[test]
    fn explicit_language_ignores_the_system_locale() {
        let german = Settings {
            language: Language::German,
            ..default_settings(PathBuf::from("/music"))
        };
        assert_eq!(current_locale(&german), Locale::De);
        assert_eq!(
            (texts_for(&german).batch_summary)(2, 0),
            "2 Downloads abgeschlossen"
        );

        let english = Settings {
            language: Language::English,
            ..german
        };
        assert_eq!(current_locale(&english), Locale::En);
        assert_eq!(
            (texts_for(&english).batch_summary)(2, 0),
            "2 downloads finished"
        );
    }
}
