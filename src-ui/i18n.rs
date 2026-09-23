//! Every text the webview shows. Each language is one `const` of [`Texts`], so
//! a text missing from a translation is a compile error.

use leptos::prelude::*;
use yaydl_shared::{ErrorKind, FriendlyError, Locale, OutputFormat, VideoQuality};

use crate::format::{count, plural};
use crate::ipc::log_to_backend;
use crate::state::{use_app, AppState};

/// Picks one text from the table, for components that take a label as a prop.
pub type Text = fn(&Texts) -> &'static str;

pub struct Texts {
    pub locale: Locale,

    pub loading: &'static str,
    pub try_again: &'static str,
    pub unknown: &'static str,
    pub dismiss: &'static str,
    pub cancel: &'static str,
    pub save: &'static str,
    pub days: fn(u64) -> String,
    pub weekdays: [&'static str; 7],
    pub weekdays_short: [&'static str; 7],
    /// "on Sundays" in English, "sonntags" in German.
    pub weekdays_habitual: [&'static str; 7],
    pub months_short: [&'static str; 12],

    pub toast_success: &'static str,
    pub toast_info: &'static str,
    pub toast_warning: &'static str,
    pub toast_error: &'static str,

    pub nav_label: &'static str,
    pub nav_downloads: &'static str,
    pub nav_statistics: &'static str,
    pub nav_settings: &'static str,

    pub listen_failed: fn(&str) -> String,
    pub queue_load_failed: fn(&str) -> String,
    pub startup_notices_failed: fn(&str) -> String,
    pub settings_load_failed: fn(&str) -> String,
    pub system_language_unknown: &'static str,
    pub theme_listen_failed: fn(&str) -> String,
    pub theme_unsupported: &'static str,
    pub theme_query_failed: fn(&str) -> String,
    pub theme_apply_failed: fn(&str) -> String,

    pub downloads_title: &'static str,
    pub add_links_label: &'static str,
    pub add_links_placeholder: &'static str,
    pub add: &'static str,
    pub paste_from_clipboard: &'static str,
    pub download_all: &'static str,
    pub clear_finished: &'static str,
    pub clear_all: &'static str,
    pub clear_all_confirm: &'static str,
    pub open_folder: &'static str,
    pub queue_empty: &'static str,
    pub drop_overlay: &'static str,
    pub added: fn(u64) -> String,
    pub already_queued: fn(u64) -> String,
    /// The first rejected tokens joined by commas, and how many were left out.
    pub not_a_link: fn(&str, u64) -> String,
    pub no_links_found: &'static str,
    pub kept_running: fn(u64) -> String,
    pub paste_unreadable: &'static str,
    pub paste_read_failed: fn(&str) -> String,
    pub drop_unreadable: &'static str,
    pub drop_nothing: &'static str,
    pub drop_links_read_failed: fn(&str) -> String,
    pub drop_text_read_failed: fn(&str) -> String,
    /// Singular and plural label per queue state, in the order resolving,
    /// awaiting confirmation, ready, downloading, queued, finished, failed,
    /// cancelled.
    pub summary: [(&'static str, &'static str); 8],

    pub item_waiting_slot: &'static str,
    pub item_converting: &'static str,
    pub item_finished: &'static str,
    pub item_cancelled: &'static str,
    pub item_fetching: &'static str,
    pub item_starting: &'static str,
    pub action_remove: &'static str,
    pub action_retry: &'static str,
    pub action_cancel: &'static str,
    pub action_rename: &'static str,
    pub action_download: &'static str,
    pub action_open: &'static str,
    pub action_reveal: &'static str,
    pub details: &'static str,
    pub cookie_hint: &'static str,
    pub format_label: &'static str,
    pub downloaded_before: fn(&str) -> String,
    pub duplicate_prompt: fn(&str, &str) -> String,
    pub duplicate_prompt_unknown: &'static str,
    pub download_again: &'static str,
    pub skip: &'static str,
    pub progress_of: fn(&str) -> String,
    pub eta: fn(&str) -> String,
    pub file_name_label: &'static str,
    pub original_title: fn(&str) -> String,
    pub name_empty: &'static str,
    pub name_whitespace: &'static str,
    pub name_trailing_dot: &'static str,
    pub name_too_long: fn(usize) -> String,
    pub name_forbidden_char: fn(&str) -> String,
    pub name_reserved: fn(&str) -> String,
    pub add_whole_playlist: &'static str,
    pub add_whole_mix: &'static str,
    pub add_whole_playlist_hint: &'static str,
    pub add_whole_mix_hint: &'static str,

    pub err_age_restricted: &'static str,
    pub err_bot_check: &'static str,
    pub err_private: &'static str,
    pub err_unavailable: &'static str,
    pub err_geo_blocked: &'static str,
    pub err_rate_limited: &'static str,
    pub err_network: &'static str,
    pub err_unsupported_url: &'static str,
    pub err_not_yet_live: &'static str,
    pub err_disk_full: &'static str,
    pub err_interrupted: &'static str,
    pub err_output_folder_missing: fn(&str) -> String,
    pub err_empty_playlist: &'static str,

    pub format_audio: fn(&str) -> String,
    pub format_video: fn(&str) -> String,
    pub format_video_best: &'static str,

    pub settings_title: &'static str,
    pub settings_loading: &'static str,
    pub settings_not_loaded: &'static str,
    pub invalid_settings: fn(&str) -> String,
    pub settings_save_failed: fn(&str) -> String,
    pub section_downloads: &'static str,
    pub output_folder: &'static str,
    pub change_folder: &'static str,
    pub default_format: &'static str,
    pub parallel_downloads: &'static str,
    pub parallel_help: &'static str,
    pub embed_metadata: &'static str,
    pub embed_help: &'static str,
    pub notify_finish: &'static str,
    pub section_cookies: &'static str,
    pub browser_cookies: &'static str,
    pub cookies_help: &'static str,
    pub cookies_none: &'static str,
    pub section_appearance: &'static str,
    pub theme: &'static str,
    pub theme_system: &'static str,
    pub theme_light: &'static str,
    pub theme_dark: &'static str,
    pub language: &'static str,
    pub language_system: &'static str,
    pub installed_version: &'static str,
    pub version_unknown: fn(&str) -> String,
    pub release_channel: &'static str,
    pub channel_help: &'static str,
    pub update_label: &'static str,
    pub updating: &'static str,
    pub update_now: &'static str,
    pub ytdlp_updated: fn(&str, &str, &str) -> String,
    pub ytdlp_current: fn(&str, &str) -> String,
    pub ytdlp_update_failed: fn(&str) -> String,
    pub section_logs: &'static str,
    pub show_logs: &'static str,
    pub hide_logs: &'static str,

    pub logs_refresh: &'static str,
    pub logs_copy: &'static str,
    pub logs_showing_last: fn(usize) -> String,
    pub logs_not_loaded: &'static str,
    pub logs_copied: &'static str,
    pub logs_load_failed: fn(&str) -> String,

    pub update_check_failed: fn(&str) -> String,
    pub update_available: fn(&str) -> String,
    pub update_install: &'static str,
    pub update_later: &'static str,
    pub update_starting: &'static str,
    /// Downloaded size, total size and percentage, already formatted.
    pub update_progress: fn(&str, &str, &str) -> String,
    pub update_downloaded: fn(&str) -> String,
    pub update_installing: fn(&str) -> String,
    pub update_hide: &'static str,
    pub update_restarting: fn(&str) -> String,
    pub update_failed: fn(&str) -> String,
    pub update_retry: &'static str,

    pub stats_title: &'static str,
    pub stats_load_failed: fn(&str) -> String,
    pub stats_loading: &'static str,
    pub stats_empty_title: &'static str,
    pub stats_empty_body: &'static str,
    pub tile_total_downloads: &'static str,
    pub tile_total_size: &'static str,
    pub tile_total_duration: &'static str,
    pub tile_streak: &'static str,
    pub streak_longest: fn(&str) -> String,
    pub over_time: &'static str,
    pub time_range: &'static str,
    pub granularity_day: &'static str,
    pub granularity_week: &'static str,
    pub granularity_month: &'static str,
    pub caption_day: &'static str,
    pub caption_week: &'static str,
    pub caption_month: &'static str,
    pub when_you_download: &'static str,
    pub top_uploaders: &'static str,
    pub formats: &'static str,
    pub no_uploaders: &'static str,
    pub no_formats: &'static str,
    /// Habitual weekday and hour phrase, see `weekdays_habitual`.
    pub most_active_day_hour: fn(&str, &str) -> String,
    pub most_active_day: fn(&str) -> String,
    pub most_active_hour: fn(&str) -> String,
    pub busiest_day: fn(&str, u64) -> String,
    pub since_uploaders: fn(&str, u64) -> String,
    /// Bucket label, download count and size.
    pub bar_tip: fn(&str, u64, &str) -> String,
    pub chart_label: &'static str,
    pub show_table: &'static str,
    pub period: &'static str,
    pub col_downloads: &'static str,
    pub size: &'static str,
    pub heatmap_label: &'static str,
    pub heatmap_broken: fn(&str) -> String,
    /// Weekday, hour 0 to 23 and download count.
    pub heat_tip: fn(&str, usize, u64) -> String,
    pub heat_legend: fn(u64) -> String,

    pub history_title: &'static str,
    pub history_search_label: &'static str,
    pub history_search_placeholder: &'static str,
    pub clear_history: &'static str,
    pub clear_history_confirm: &'static str,
    pub history_loading: &'static str,
    pub history_empty: &'static str,
    pub history_no_match: &'static str,
    pub history_truncated: fn(usize, usize) -> String,
    pub col_name: &'static str,
    pub col_uploader: &'static str,
    pub col_format: &'static str,
    pub col_date: &'static str,
    pub history_load_failed: fn(&str) -> String,
    pub history_clear_failed: fn(&str) -> String,
}

fn downloads_en(n: u64) -> String {
    plural(n, Locale::En, "download", "downloads")
}

fn downloads_de(n: u64) -> String {
    plural(n, Locale::De, "Download", "Downloads")
}

pub const EN: Texts = Texts {
    locale: Locale::En,

    loading: "Loading\u{2026}",
    try_again: "Try again",
    unknown: "Unknown",
    dismiss: "Dismiss",
    cancel: "Cancel",
    save: "Save",
    days: |n| plural(n, Locale::En, "day", "days"),
    weekdays: [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ],
    weekdays_short: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
    weekdays_habitual: [
        "on Mondays",
        "on Tuesdays",
        "on Wednesdays",
        "on Thursdays",
        "on Fridays",
        "on Saturdays",
        "on Sundays",
    ],
    months_short: [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ],

    toast_success: "Success",
    toast_info: "Info",
    toast_warning: "Warning",
    toast_error: "Error",

    nav_label: "Main",
    nav_downloads: "Downloads",
    nav_statistics: "Statistics",
    nav_settings: "Settings",

    listen_failed: |e| {
        format!("The UI cannot receive updates from the backend, so the queue may be stale:\n{e}")
    },
    queue_load_failed: |e| format!("Loading the download queue failed: {e}"),
    startup_notices_failed: |e| format!("Loading startup messages failed: {e}"),
    settings_load_failed: |e| format!("Loading the settings failed: {e}"),
    system_language_unknown: "The webview does not report the system language, so System shows English",
    theme_listen_failed: |e| format!("Listening for OS theme changes failed: {e}"),
    theme_unsupported: "The webview cannot report the OS theme, so System uses the light theme",
    theme_query_failed: |e| {
        format!("Querying the OS theme failed, so System uses the light theme: {e}")
    },
    theme_apply_failed: |e| format!("Applying the theme failed: {e}"),

    downloads_title: "Downloads",
    add_links_label: "Links to add",
    add_links_placeholder: "Paste links (YouTube and 1000+ sites)",
    add: "Add",
    paste_from_clipboard: "Paste from clipboard",
    download_all: "Download all",
    clear_finished: "Clear finished",
    clear_all: "Clear all",
    clear_all_confirm: "Click again to clear all",
    open_folder: "Open folder",
    queue_empty: "Paste a link with Ctrl+V, drop it here, or type it above.",
    drop_overlay: "Drop links to add them",
    added: |n| format!("Added {}", count(n, Locale::En)),
    already_queued: |n| format!("{} already in the queue", count(n, Locale::En)),
    not_a_link: |shown, rest| {
        if rest > 0 {
            format!("Not a link: {shown} and {} more", count(rest, Locale::En))
        } else {
            format!("Not a link: {shown}")
        }
    },
    no_links_found: "No links found",
    kept_running: |n| {
        if n == 1 {
            "Kept 1 download that is still running".to_string()
        } else {
            format!(
                "Kept {} downloads that are still running",
                count(n, Locale::En)
            )
        }
    },
    paste_unreadable: "The pasted content could not be read",
    paste_read_failed: |e| format!("Reading the pasted text failed: {e}"),
    drop_unreadable: "The dropped content could not be read",
    drop_nothing: "Nothing to add. Drop a link from your browser's address bar or a page.",
    drop_links_read_failed: |e| format!("Reading the dropped links failed: {e}"),
    drop_text_read_failed: |e| format!("Reading the dropped text failed: {e}"),
    summary: [
        ("fetching info", "fetching info"),
        ("awaiting confirmation", "awaiting confirmation"),
        ("ready", "ready"),
        ("downloading", "downloading"),
        ("queued", "queued"),
        ("finished", "finished"),
        ("failed", "failed"),
        ("cancelled", "cancelled"),
    ],

    item_waiting_slot: "Waiting for a free slot",
    item_converting: "Converting\u{2026}",
    item_finished: "Finished",
    item_cancelled: "Cancelled",
    item_fetching: "Fetching info\u{2026}",
    item_starting: "Starting\u{2026}",
    action_remove: "Remove",
    action_retry: "Retry",
    action_cancel: "Cancel",
    action_rename: "Rename",
    action_download: "Download",
    action_open: "Open",
    action_reveal: "Show in folder",
    details: "Details",
    cookie_hint: "Choose a browser under Settings \u{2192} Cookies, then retry.",
    format_label: "Format",
    downloaded_before: |date| format!("Downloaded before on {date}"),
    duplicate_prompt: |date, path| {
        format!("You already downloaded this on {date} to {path}. Download it again?")
    },
    duplicate_prompt_unknown: "You already downloaded this. Download it again?",
    download_again: "Download again",
    skip: "Skip",
    progress_of: |total| format!("of {total}"),
    eta: |clock| format!("{clock} left"),
    file_name_label: "File name",
    original_title: |title| format!("Original: {title}"),
    name_empty: "The name is empty",
    name_whitespace: "The name starts or ends with a space",
    name_trailing_dot: "The name ends with a dot",
    name_too_long: |max| format!("The name is longer than {max} characters"),
    name_forbidden_char: |c| format!("The name contains the forbidden character \u{201c}{c}\u{201d}"),
    name_reserved: |name| format!("\u{201c}{name}\u{201d} is a reserved name on Windows"),
    add_whole_playlist: "Add whole playlist",
    add_whole_mix: "Add whole mix",
    add_whole_playlist_hint: "Replace this video with all videos of its playlist",
    add_whole_mix_hint: "Replace this video with all videos of its mix",

    err_age_restricted: "This video is age-restricted. Choose a browser you are signed in with under cookies in the settings, then retry.",
    err_bot_check: "YouTube wants to confirm you are not a bot. Choose a browser you are signed in with under cookies in the settings, then retry.",
    err_private: "This video is private or for channel members only. If your account has access, choose a browser you are signed in with under cookies in the settings, then retry.",
    err_unavailable: "This video is unavailable. It may have been removed, or the link is wrong.",
    err_geo_blocked: "This video is not available in your country. It can only be downloaded from a network in a country where it is available.",
    err_rate_limited: "The site is limiting how many requests yaydl makes. Wait a while, or lower the number of parallel downloads, then retry.",
    err_network: "The site could not be reached. Check your internet connection, then retry.",
    err_unsupported_url: "yt-dlp does not support this URL. Check that it links to a video or playlist page.",
    err_not_yet_live: "This live stream or premiere has not started yet. Retry once it is live or over.",
    err_disk_full: "The disk is full. Free up space or choose another download folder, then retry.",
    err_interrupted: "The download was interrupted because yaydl closed. Retry to start it again.",
    err_output_folder_missing: |path| {
        format!("The output folder {path} does not exist. Choose another one in the settings.")
    },
    err_empty_playlist: "This playlist has no videos that can be downloaded.",

    format_audio: |codec| format!("{codec} audio"),
    format_video: |quality| format!("MP4 video {quality}"),
    format_video_best: "MP4 video best",

    settings_title: "Settings",
    settings_loading: "Loading settings\u{2026}",
    settings_not_loaded: "The settings have not loaded yet, so nothing was saved",
    invalid_settings: |e| format!("Invalid settings: {e}"),
    settings_save_failed: |e| format!("Saving the settings failed: {e}"),
    section_downloads: "Downloads",
    output_folder: "Output folder",
    change_folder: "Change\u{2026}",
    default_format: "Default format",
    parallel_downloads: "Parallel downloads",
    parallel_help: "How many downloads run at the same time.",
    embed_metadata: "Embed metadata",
    embed_help: "Adds title, artist and cover art to the file.",
    notify_finish: "Notify when downloads finish",
    section_cookies: "Cookies",
    browser_cookies: "Browser cookies",
    cookies_help: "Needed for age-restricted videos and YouTube bot checks. yaydl reads cookies from this browser's profile.",
    cookies_none: "None",
    section_appearance: "Appearance",
    theme: "Theme",
    theme_system: "System",
    theme_light: "Light",
    theme_dark: "Dark",
    language: "Language",
    language_system: "System",
    installed_version: "Installed version",
    version_unknown: |e| format!("Unknown: {e}"),
    release_channel: "Release channel",
    channel_help: "Nightly gets YouTube fixes days before stable.",
    update_label: "Update",
    updating: "Updating\u{2026}",
    update_now: "Update yt-dlp now",
    ytdlp_updated: |from, to, channel| format!("yt-dlp updated from {from} to {to} ({channel})"),
    ytdlp_current: |version, channel| format!("yt-dlp {version} is the latest {channel} version"),
    ytdlp_update_failed: |e| format!("Updating yt-dlp failed: {e}"),
    section_logs: "Logs",
    show_logs: "Show logs",
    hide_logs: "Hide logs",

    logs_refresh: "Refresh",
    logs_copy: "Copy to clipboard",
    logs_showing_last: |n| format!("Showing the last {} lines", count(n as u64, Locale::En)),
    logs_not_loaded: "The logs have not loaded yet",
    logs_copied: "Logs copied to the clipboard",
    logs_load_failed: |e| format!("Loading the logs failed: {e}"),

    update_check_failed: |e| format!("Checking for a yaydl update failed: {e}"),
    update_available: |v| format!("yaydl {v} is available"),
    update_install: "Update and restart",
    update_later: "Later",
    update_starting: "Starting the download\u{2026}",
    update_progress: |done, total, percent| format!("{done} of {total} ({percent})"),
    update_downloaded: |done| format!("{done} downloaded"),
    update_installing: |v| format!("Installing yaydl {v}"),
    update_hide: "Hide",
    update_restarting: |v| {
        format!("yaydl {v} is installed and restarts now. If it does not, restart it yourself.")
    },
    update_failed: |v| format!("Updating to yaydl {v} failed"),
    update_retry: "Retry",

    stats_title: "Statistics",
    stats_load_failed: |e| format!("Loading the statistics failed: {e}"),
    stats_loading: "Loading statistics\u{2026}",
    stats_empty_title: "No downloads yet",
    stats_empty_body: "Finished downloads show up here with charts and your download history.",
    tile_total_downloads: "Total downloads",
    tile_total_size: "Total size",
    tile_total_duration: "Total duration",
    tile_streak: "Current streak",
    streak_longest: |days| format!("Longest {days}"),
    over_time: "Downloads over time",
    time_range: "Time range",
    granularity_day: "Day",
    granularity_week: "Week",
    granularity_month: "Month",
    caption_day: "Downloads per day, last 30 days",
    caption_week: "Downloads per week, last 12 weeks",
    caption_month: "Downloads per month, last 12 months",
    when_you_download: "When you download",
    top_uploaders: "Top uploaders",
    formats: "Formats",
    no_uploaders: "No uploader information yet",
    no_formats: "No formats yet",
    most_active_day_hour: |day, hour| format!("You're most active {day} {hour}"),
    most_active_day: |day| format!("You're most active {day}"),
    most_active_hour: |hour| format!("You're most active {hour}"),
    busiest_day: |date, n| {
        format!(
            "Busiest day: {date} with {}",
            downloads_en(n)
        )
    },
    since_uploaders: |date, n| {
        format!(
            "Since {date} you downloaded from {}",
            plural(n, Locale::En, "uploader", "uploaders")
        )
    },
    bar_tip: |label, n, size| {
        format!(
            "{label}: {}, {size}",
            downloads_en(n)
        )
    },
    chart_label: "Downloads per period",
    show_table: "Show as table",
    period: "Period",
    col_downloads: "Downloads",
    size: "Size",
    heatmap_label: "Downloads by weekday and hour",
    heatmap_broken: |e| format!("The activity chart cannot be drawn: {e}"),
    heat_tip: |day, hour, n| {
        format!(
            "{day} {hour:02}:00 to {:02}:00: {}",
            (hour + 1) % 24,
            downloads_en(n)
        )
    },
    heat_legend: |n| {
        format!(
            "{} in one hour slot",
            downloads_en(n)
        )
    },

    history_title: "History",
    history_search_label: "Search history",
    history_search_placeholder: "Search title or uploader",
    clear_history: "Clear history",
    clear_history_confirm: "Click again to clear",
    history_loading: "Loading history\u{2026}",
    history_empty: "Nothing downloaded yet",
    history_no_match: "No downloads match the search",
    history_truncated: |shown, total| {
        format!(
            "Showing {} of {} matches. Refine the search to see others.",
            count(shown as u64, Locale::En),
            count(total as u64, Locale::En)
        )
    },
    col_name: "Name",
    col_uploader: "Uploader",
    col_format: "Format",
    col_date: "Date",
    history_load_failed: |e| format!("Loading the download history failed: {e}"),
    history_clear_failed: |e| format!("Clearing the history failed: {e}"),
};

pub const DE: Texts = Texts {
    locale: Locale::De,

    loading: "Wird geladen \u{2026}",
    try_again: "Nochmal versuchen",
    unknown: "Unbekannt",
    dismiss: "Schließen",
    cancel: "Abbrechen",
    save: "Speichern",
    days: |n| plural(n, Locale::De, "Tag", "Tage"),
    weekdays: [
        "Montag",
        "Dienstag",
        "Mittwoch",
        "Donnerstag",
        "Freitag",
        "Samstag",
        "Sonntag",
    ],
    weekdays_short: ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"],
    weekdays_habitual: [
        "montags",
        "dienstags",
        "mittwochs",
        "donnerstags",
        "freitags",
        "samstags",
        "sonntags",
    ],
    months_short: [
        "Jan.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.",
        "Dez.",
    ],

    toast_success: "Erfolg",
    toast_info: "Info",
    toast_warning: "Warnung",
    toast_error: "Fehler",

    nav_label: "Hauptmenü",
    nav_downloads: "Downloads",
    nav_statistics: "Statistik",
    nav_settings: "Einstellungen",

    listen_failed: |e| {
        format!(
            "Die Oberfläche bekommt keine Updates vom Backend, die Warteschlange ist eventuell veraltet:\n{e}"
        )
    },
    queue_load_failed: |e| format!("Warteschlange konnte nicht geladen werden: {e}"),
    startup_notices_failed: |e| format!("Startmeldungen konnten nicht geladen werden: {e}"),
    settings_load_failed: |e| format!("Einstellungen konnten nicht geladen werden: {e}"),
    system_language_unknown: "Die Webview meldet keine Systemsprache, deshalb zeigt System Englisch",
    theme_listen_failed: |e| format!("Änderungen am System-Design werden nicht erkannt: {e}"),
    theme_unsupported: "Die Webview meldet kein System-Design, deshalb nutzt System das helle Design",
    theme_query_failed: |e| {
        format!(
            "System-Design konnte nicht abgefragt werden, deshalb nutzt System das helle Design: {e}"
        )
    },
    theme_apply_failed: |e| format!("Design konnte nicht angewendet werden: {e}"),

    downloads_title: "Downloads",
    add_links_label: "Links zum Hinzufügen",
    add_links_placeholder: "Links einfügen (YouTube und 1000+ Seiten)",
    add: "Hinzufügen",
    paste_from_clipboard: "Aus Zwischenablage",
    download_all: "Alle herunterladen",
    clear_finished: "Abgeschlossene entfernen",
    clear_all: "Alle entfernen",
    clear_all_confirm: "Nochmal klicken zum Entfernen",
    open_folder: "Ordner öffnen",
    queue_empty: "Füge einen Link mit Strg+V ein, zieh ihn hierher oder tipp ihn oben ein.",
    drop_overlay: "Links loslassen zum Hinzufügen",
    added: |n| format!("{} hinzugefügt", count(n, Locale::De)),
    already_queued: |n| format!("{} schon in der Warteschlange", count(n, Locale::De)),
    not_a_link: |shown, rest| {
        if rest > 0 {
            format!("Kein Link: {shown} und {} weitere", count(rest, Locale::De))
        } else {
            format!("Kein Link: {shown}")
        }
    },
    no_links_found: "Keine Links gefunden",
    kept_running: |n| {
        if n == 1 {
            "1 laufender Download wurde behalten".to_string()
        } else {
            format!(
                "{} laufende Downloads wurden behalten",
                count(n, Locale::De)
            )
        }
    },
    paste_unreadable: "Der eingefügte Inhalt konnte nicht gelesen werden",
    paste_read_failed: |e| format!("Eingefügter Text konnte nicht gelesen werden: {e}"),
    drop_unreadable: "Der abgelegte Inhalt konnte nicht gelesen werden",
    drop_nothing: "Nichts zum Hinzufügen. Zieh einen Link aus der Adressleiste oder von einer Seite hierher.",
    drop_links_read_failed: |e| format!("Abgelegte Links konnten nicht gelesen werden: {e}"),
    drop_text_read_failed: |e| format!("Abgelegter Text konnte nicht gelesen werden: {e}"),
    summary: [
        ("lädt Infos", "laden Infos"),
        ("wartet auf Bestätigung", "warten auf Bestätigung"),
        ("bereit", "bereit"),
        ("lädt herunter", "laden herunter"),
        ("in der Warteschlange", "in der Warteschlange"),
        ("abgeschlossen", "abgeschlossen"),
        ("fehlgeschlagen", "fehlgeschlagen"),
        ("abgebrochen", "abgebrochen"),
    ],

    item_waiting_slot: "Wartet auf einen freien Platz",
    item_converting: "Wird konvertiert \u{2026}",
    item_finished: "Abgeschlossen",
    item_cancelled: "Abgebrochen",
    item_fetching: "Infos werden geladen \u{2026}",
    item_starting: "Startet \u{2026}",
    action_remove: "Entfernen",
    action_retry: "Wiederholen",
    action_cancel: "Abbrechen",
    action_rename: "Umbenennen",
    action_download: "Herunterladen",
    action_open: "Öffnen",
    action_reveal: "Im Ordner zeigen",
    details: "Details",
    cookie_hint: "Wähle unter Einstellungen \u{2192} Cookies einen Browser und versuch es dann nochmal.",
    format_label: "Format",
    downloaded_before: |date| format!("Schon am {date} heruntergeladen"),
    duplicate_prompt: |date, path| {
        format!("Du hast das am {date} schon nach {path} heruntergeladen. Nochmal herunterladen?")
    },
    duplicate_prompt_unknown: "Du hast das schon heruntergeladen. Nochmal herunterladen?",
    download_again: "Nochmal herunterladen",
    skip: "Überspringen",
    progress_of: |total| format!("von {total}"),
    eta: |clock| format!("noch {clock}"),
    file_name_label: "Dateiname",
    original_title: |title| format!("Original: {title}"),
    name_empty: "Der Name ist leer",
    name_whitespace: "Der Name beginnt oder endet mit einem Leerzeichen",
    name_trailing_dot: "Der Name endet mit einem Punkt",
    name_too_long: |max| format!("Der Name ist länger als {max} Zeichen"),
    name_forbidden_char: |c| format!("Das Zeichen \u{201e}{c}\u{201c} ist im Namen nicht erlaubt"),
    name_reserved: |name| format!("\u{201e}{name}\u{201c} ist unter Windows ein reservierter Name"),
    add_whole_playlist: "Ganze Playlist hinzufügen",
    add_whole_mix: "Ganzen Mix hinzufügen",
    add_whole_playlist_hint: "Ersetze dieses Video durch alle Videos seiner Playlist",
    add_whole_mix_hint: "Ersetze dieses Video durch alle Videos seines Mixes",

    err_age_restricted: "Dieses Video hat eine Altersbeschränkung. Wähle in den Einstellungen unter Cookies einen Browser, in dem du angemeldet bist, und versuch es dann nochmal.",
    err_bot_check: "YouTube will prüfen, ob du ein Bot bist. Wähle in den Einstellungen unter Cookies einen Browser, in dem du angemeldet bist, und versuch es dann nochmal.",
    err_private: "Dieses Video ist privat oder nur für Kanalmitglieder. Wenn dein Konto Zugriff hat, wähle in den Einstellungen unter Cookies einen Browser, in dem du angemeldet bist, und versuch es dann nochmal.",
    err_unavailable: "Dieses Video ist nicht verfügbar. Es wurde vielleicht entfernt, oder der Link ist falsch.",
    err_geo_blocked: "Dieses Video ist in deinem Land nicht verfügbar. Es lässt sich nur aus einem Netz in einem Land herunterladen, in dem es verfügbar ist.",
    err_rate_limited: "Die Seite begrenzt, wie viele Anfragen yaydl stellen darf. Warte eine Weile oder stell weniger parallele Downloads ein und versuch es dann nochmal.",
    err_network: "Die Seite ist nicht erreichbar. Prüf deine Internetverbindung und versuch es dann nochmal.",
    err_unsupported_url: "yt-dlp unterstützt diese URL nicht. Prüf, ob sie auf ein Video oder eine Playlist zeigt.",
    err_not_yet_live: "Dieser Livestream oder diese Premiere hat noch nicht angefangen. Versuch es nochmal, sobald sie läuft oder vorbei ist.",
    err_disk_full: "Die Festplatte ist voll. Mach Platz frei oder wähle einen anderen Zielordner und versuch es dann nochmal.",
    err_interrupted: "Der Download wurde unterbrochen, weil yaydl beendet wurde. Versuch es nochmal, um ihn neu zu starten.",
    err_output_folder_missing: |path| {
        format!(
            "Der Zielordner {path} existiert nicht. Wähle in den Einstellungen einen anderen Ordner."
        )
    },
    err_empty_playlist: "Diese Playlist enthält keine Videos, die sich herunterladen lassen.",

    format_audio: |codec| format!("{codec}-Audio"),
    format_video: |quality| format!("MP4-Video {quality}"),
    format_video_best: "MP4-Video, beste Qualität",

    settings_title: "Einstellungen",
    settings_loading: "Einstellungen werden geladen \u{2026}",
    settings_not_loaded: "Die Einstellungen sind noch nicht geladen, deshalb wurde nichts gespeichert",
    invalid_settings: |e| format!("Ungültige Einstellungen: {e}"),
    settings_save_failed: |e| format!("Einstellungen konnten nicht gespeichert werden: {e}"),
    section_downloads: "Downloads",
    output_folder: "Zielordner",
    change_folder: "Ändern \u{2026}",
    default_format: "Standardformat",
    parallel_downloads: "Parallele Downloads",
    parallel_help: "Wie viele Downloads gleichzeitig laufen.",
    embed_metadata: "Metadaten einbetten",
    embed_help: "Schreibt Titel, Interpret und Cover in die Datei.",
    notify_finish: "Benachrichtigen, wenn Downloads abgeschlossen sind",
    section_cookies: "Cookies",
    browser_cookies: "Browser-Cookies",
    cookies_help: "Nötig für Videos mit Altersbeschränkung und die Bot-Prüfung von YouTube. yaydl liest die Cookies aus dem Profil dieses Browsers.",
    cookies_none: "Keine",
    section_appearance: "Darstellung",
    theme: "Design",
    theme_system: "System",
    theme_light: "Hell",
    theme_dark: "Dunkel",
    language: "Sprache",
    language_system: "System",
    installed_version: "Installierte Version",
    version_unknown: |e| format!("Unbekannt: {e}"),
    release_channel: "Release-Kanal",
    channel_help: "Nightly bekommt YouTube-Fixes Tage vor Stable.",
    update_label: "Update",
    updating: "Wird aktualisiert \u{2026}",
    update_now: "yt-dlp jetzt aktualisieren",
    ytdlp_updated: |from, to, channel| {
        format!("yt-dlp von {from} auf {to} aktualisiert ({channel})")
    },
    ytdlp_current: |version, channel| {
        format!("yt-dlp {version} ist die neueste {channel}-Version")
    },
    ytdlp_update_failed: |e| format!("yt-dlp konnte nicht aktualisiert werden: {e}"),
    section_logs: "Logs",
    show_logs: "Logs anzeigen",
    hide_logs: "Logs ausblenden",

    logs_refresh: "Aktualisieren",
    logs_copy: "In die Zwischenablage",
    logs_showing_last: |n| format!("Zeigt die letzten {} Zeilen", count(n as u64, Locale::De)),
    logs_not_loaded: "Die Logs sind noch nicht geladen",
    logs_copied: "Logs in die Zwischenablage kopiert",
    logs_load_failed: |e| format!("Logs konnten nicht geladen werden: {e}"),

    update_check_failed: |e| format!("Suche nach einem yaydl-Update fehlgeschlagen: {e}"),
    update_available: |v| format!("yaydl {v} ist verfügbar"),
    update_install: "Aktualisieren und neu starten",
    update_later: "Später",
    update_starting: "Download startet \u{2026}",
    update_progress: |done, total, percent| format!("{done} von {total} ({percent})"),
    update_downloaded: |done| format!("{done} heruntergeladen"),
    update_installing: |v| format!("yaydl {v} wird installiert"),
    update_hide: "Ausblenden",
    update_restarting: |v| {
        format!("yaydl {v} ist installiert und startet jetzt neu. Falls nicht, starte es selbst neu.")
    },
    update_failed: |v| format!("Update auf yaydl {v} fehlgeschlagen"),
    update_retry: "Nochmal versuchen",

    stats_title: "Statistik",
    stats_load_failed: |e| format!("Statistik konnte nicht geladen werden: {e}"),
    stats_loading: "Statistik wird geladen \u{2026}",
    stats_empty_title: "Noch keine Downloads",
    stats_empty_body: "Abgeschlossene Downloads erscheinen hier mit Diagrammen und deinem Verlauf.",
    tile_total_downloads: "Downloads gesamt",
    tile_total_size: "Größe gesamt",
    tile_total_duration: "Dauer gesamt",
    tile_streak: "Aktuelle Serie",
    streak_longest: |days| format!("Rekord: {days}"),
    over_time: "Downloads im Zeitverlauf",
    time_range: "Zeitraum",
    granularity_day: "Tag",
    granularity_week: "Woche",
    granularity_month: "Monat",
    caption_day: "Downloads pro Tag, letzte 30 Tage",
    caption_week: "Downloads pro Woche, letzte 12 Wochen",
    caption_month: "Downloads pro Monat, letzte 12 Monate",
    when_you_download: "Wann du herunterlädst",
    top_uploaders: "Top-Uploader",
    formats: "Formate",
    no_uploaders: "Noch keine Uploader-Infos",
    no_formats: "Noch keine Formate",
    most_active_day_hour: |day, hour| format!("Am aktivsten bist du {day} {hour}"),
    most_active_day: |day| format!("Am aktivsten bist du {day}"),
    most_active_hour: |hour| format!("Am aktivsten bist du {hour}"),
    busiest_day: |date, n| {
        format!(
            "Aktivster Tag: {date} mit {}",
            downloads_de(n)
        )
    },
    since_uploaders: |date, n| {
        format!(
            "Seit dem {date} hast du von {} heruntergeladen",
            plural(n, Locale::De, "Uploader", "Uploadern")
        )
    },
    bar_tip: |label, n, size| {
        format!(
            "{label}: {}, {size}",
            downloads_de(n)
        )
    },
    chart_label: "Downloads pro Zeitraum",
    show_table: "Als Tabelle anzeigen",
    period: "Zeitraum",
    col_downloads: "Downloads",
    size: "Größe",
    heatmap_label: "Downloads nach Wochentag und Uhrzeit",
    heatmap_broken: |e| format!("Das Aktivitätsdiagramm kann nicht gezeichnet werden: {e}"),
    heat_tip: |day, hour, n| {
        format!(
            "{day} {hour} bis {} Uhr: {}",
            (hour + 1) % 24,
            downloads_de(n)
        )
    },
    heat_legend: |n| {
        format!(
            "{} in einer Stunde",
            downloads_de(n)
        )
    },

    history_title: "Verlauf",
    history_search_label: "Verlauf durchsuchen",
    history_search_placeholder: "Titel oder Uploader suchen",
    clear_history: "Verlauf löschen",
    clear_history_confirm: "Nochmal klicken zum Löschen",
    history_loading: "Verlauf wird geladen \u{2026}",
    history_empty: "Noch nichts heruntergeladen",
    history_no_match: "Keine Downloads passen zur Suche",
    history_truncated: |shown, total| {
        format!(
            "{} von {} Treffern. Verfeinere die Suche, um weitere zu sehen.",
            count(shown as u64, Locale::De),
            count(total as u64, Locale::De)
        )
    },
    col_name: "Name",
    col_uploader: "Uploader",
    col_format: "Format",
    col_date: "Datum",
    history_load_failed: |e| format!("Verlauf konnte nicht geladen werden: {e}"),
    history_clear_failed: |e| format!("Verlauf konnte nicht gelöscht werden: {e}"),
};

pub fn texts(locale: Locale) -> &'static Texts {
    match locale {
        Locale::En => &EN,
        Locale::De => &DE,
    }
}

/// The texts of the current locale. Reading it inside a reactive closure
/// re-renders that closure when the language changes.
pub fn use_texts() -> impl Fn() -> &'static Texts + Copy + Send + Sync + 'static {
    let locale = use_app().locale;
    move || texts(locale.get())
}

/// For event handlers and async tasks, which must not subscribe to the locale.
pub fn texts_now(state: AppState) -> &'static Texts {
    texts(state.locale.get_untracked())
}

/// The message for a failed download. The backend's `message` is English, so
/// every kind with a fixed meaning is rendered from the table instead.
pub fn error_message(t: &Texts, error: &FriendlyError) -> String {
    let text = match error.kind {
        ErrorKind::AgeRestricted => t.err_age_restricted,
        ErrorKind::BotCheck => t.err_bot_check,
        ErrorKind::Private => t.err_private,
        ErrorKind::Unavailable => t.err_unavailable,
        ErrorKind::GeoBlocked => t.err_geo_blocked,
        ErrorKind::RateLimited => t.err_rate_limited,
        ErrorKind::Network => t.err_network,
        ErrorKind::UnsupportedUrl => t.err_unsupported_url,
        ErrorKind::NotYetLive => t.err_not_yet_live,
        ErrorKind::DiskFull => t.err_disk_full,
        ErrorKind::Interrupted => t.err_interrupted,
        ErrorKind::EmptyPlaylist => t.err_empty_playlist,
        ErrorKind::OutputFolderMissing => return (t.err_output_folder_missing)(&error.detail),
        ErrorKind::Other => return error.message.clone(),
    };
    text.to_string()
}

pub fn format_label(t: &Texts, format: OutputFormat) -> String {
    match format {
        OutputFormat::Audio { codec } => (t.format_audio)(codec.label()),
        OutputFormat::Video {
            quality: VideoQuality::Best,
        } => t.format_video_best.to_string(),
        OutputFormat::Video { quality } => (t.format_video)(quality.label()),
    }
}

/// Statistics name formats by their English `Display` text. A name that
/// matches no current format, e.g. from an older version, is shown as stored.
pub fn format_name_label(t: &Texts, name: &str) -> String {
    match OutputFormat::all()
        .into_iter()
        .find(|f| f.to_string() == name)
    {
        Some(format) => format_label(t, format),
        None => {
            log_to_backend(
                "warn",
                format!("statistics list the unknown format {name:?}, shown untranslated"),
            );
            name.to_string()
        }
    }
}

/// Keeps `<html lang>` in sync with the locale, for screen readers and
/// hyphenation.
pub fn install(state: AppState, system_tag_missing: bool) {
    if system_tag_missing {
        let message = texts_now(state).system_language_unknown;
        log_to_backend("warn", message.to_string());
        state.toasts.warning(message);
    }
    Effect::new(move |_| {
        let lang = match state.locale.get() {
            Locale::En => "en",
            Locale::De => "de",
        };
        let Some(root) = document().document_element() else {
            log_to_backend(
                "error",
                format!("setting <html lang> to {lang} failed: the document has no root element"),
            );
            return;
        };
        if let Err(e) = root.set_attribute("lang", lang) {
            log_to_backend(
                "error",
                format!("setting <html lang> to {lang} failed: {e:?}"),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_follow_the_kind() {
        let error = |kind| FriendlyError {
            kind,
            message: "backend text".to_string(),
            detail: "/home/me/Music".to_string(),
        };
        assert_eq!(
            error_message(&DE, &error(ErrorKind::Network)),
            DE.err_network
        );
        assert_eq!(error_message(&DE, &error(ErrorKind::Other)), "backend text");
        assert_eq!(
            error_message(&DE, &error(ErrorKind::OutputFolderMissing)),
            "Der Zielordner /home/me/Music existiert nicht. Wähle in den Einstellungen einen anderen Ordner."
        );
    }

    #[test]
    fn format_labels() {
        let mp3 = OutputFormat::Audio {
            codec: yaydl_shared::AudioCodec::Mp3,
        };
        let p720 = OutputFormat::Video {
            quality: VideoQuality::P720,
        };
        assert_eq!(format_label(&EN, mp3), mp3.to_string());
        assert_eq!(format_label(&EN, p720), p720.to_string());
        assert_eq!(format_label(&DE, mp3), "MP3-Audio");
        assert_eq!(format_label(&DE, p720), "MP4-Video 720p");
        for format in OutputFormat::all() {
            assert_eq!(format_label(&EN, format), format.to_string());
        }
    }

    #[test]
    fn german_sentences() {
        assert_eq!(
            (DE.most_active_day_hour)(
                DE.weekdays_habitual[6],
                &crate::format::around_hour(21, Locale::De)
            ),
            "Am aktivsten bist du sonntags gegen 21 Uhr"
        );
        assert_eq!(
            (EN.most_active_day_hour)(
                EN.weekdays_habitual[6],
                &crate::format::around_hour(21, Locale::En)
            ),
            "You're most active on Sundays around 21:00"
        );
        assert_eq!(
            (DE.duplicate_prompt)("12. Sep. 2026", "/m/a.mp3"),
            "Du hast das am 12. Sep. 2026 schon nach /m/a.mp3 heruntergeladen. Nochmal herunterladen?"
        );
        assert_eq!((DE.kept_running)(1), "1 laufender Download wurde behalten");
        assert_eq!((DE.kept_running)(2), "2 laufende Downloads wurden behalten");
        assert_eq!(
            (DE.since_uploaders)("1. Jan. 2026", 1),
            "Seit dem 1. Jan. 2026 hast du von 1 Uploader heruntergeladen"
        );
        assert_eq!(
            (DE.heat_tip)("Montag", 23, 1),
            "Montag 23 bis 0 Uhr: 1 Download"
        );
    }
}
