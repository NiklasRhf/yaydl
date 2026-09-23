##  v0.4.1 (2026-09-23)

* fix: downloads on Windows no longer fail with "yt-dlp reported a finished file that is not on disk" when the title contains characters Windows replaces in file names, such as quotes
* feat: larger default window (1280x860), shrunk to fit small screens
* build: Arch Linux package (PKGBUILD) that runs on the system WebKitGTK, smoother than the AppImage on Arch

##  v0.4.0 (2026-09-23)

* feat: download queue with several links at once, playlist expansion, and per-entry format and file name
* feat: parallel downloads with a configurable limit (1 to 5, default 2)
* feat: the queue is saved and restored on restart, and downloads cut off by closing yaydl can be retried
* feat: cancel, retry, remove, and clear finished or all entries
* feat: warning before downloading a video again in the same format when the earlier file still exists
* feat: rename finished downloads on disk, open them, or show them in the file manager
* feat: download history and statistics per day, week and month
* feat: video downloads as MP4 (best, 1080p, 720p, 480p) next to MP3, M4A, Opus and FLAC audio
* feat: cookies from a browser for age-restricted and members-only videos
* feat: system notification when a batch of downloads finishes in the background
* feat: light, dark or system theme, and toggles for embedded metadata and notifications
* feat: messages in the window for startup problems, such as an invalid settings file or a missing output folder
* feat: settings from earlier versions are migrated automatically, and an unreadable file is moved aside instead of overwritten
* feat: one log file per day, the last 7 kept
* feat: add links from any site yt-dlp supports, by typing, pasting with Ctrl+V anywhere, or drag and drop
* feat: "Add whole playlist" on video links opened from a playlist, and "Add whole mix" for YouTube Mixes (first 50 videos)
* feat: German translation with a language setting (system, English, German)
* feat: About section with the app version, a manual update check and a link to the release notes
* feat: non-blocking update banner instead of a forced update dialog
* feat: new Deco gold app icon, also shown in the sidebar
* feat: plain-language error messages for age restrictions, bot checks, private or geo-blocked videos and rate limits
* fix: renamed downloads show their new name in the list and in the history
* fix: reading and writing the clipboard works on KDE and GNOME Wayland
* build: bundled yt-dlp and ffmpeg are pinned and verified by checksum at build time

##  v0.2.0 (2024-11-10)

* feat: improve error handling ([a5eab09](https://github.com/NiklasRhf/yaydl/commit/a5eab09))
* feat: add download progress ([704c9ac](https://github.com/NiklasRhf/yaydl/commit/704c9ac))
* feat: add notifications ([05a0626](https://github.com/NiklasRhf/yaydl/commit/05a0626))

##  v0.1.0 (2024-11-01)

* feat: add updater and explicitly set capabilities ([9c5b9af](https://github.com/NiklasRhf/yaydl/commit/9c5b9af))
* chore: add github workflow ([5713521](https://github.com/NiklasRhf/yaydl/commit/5713521))
* Initial commit ([be2c054](https://github.com/NiklasRhf/yaydl/commit/be2c054))
