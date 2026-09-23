# YaYDL

Yet Another YouTube Downloader

## Using yaydl

- **Add links** by typing or pasting them into the input field, by pressing Ctrl+V anywhere in the
  window, or by dragging links onto it. Several links separated by spaces or new lines are added at
  once, and a playlist link expands into one entry per video. Links already in the queue are
  skipped.
- **Videos opened from a playlist** (a `watch?v=...&list=...` link) add only that video. Its row
  then offers "Add whole playlist", which replaces the entry with every video of the playlist. This
  also works for YouTube Mixes, which have no playlist page of their own. To add a regular playlist
  right away, paste its `https://www.youtube.com/playlist?list=...` link instead.
- **Pick a format** per entry before it starts: MP3, M4A, Opus or FLAC audio, or MP4 video at best,
  1080p, 720p or 480p. The default for new entries is set in Settings.
- **Rename** an entry before the download to choose the file name, or afterwards to rename the file
  on disk.
- **Parallel downloads** run up to the limit set in Settings (1 to 5, default 2). The rest wait in
  the queue and start as slots free up. The queue survives a restart. Downloads that were running
  when yaydl closed show up as interrupted and can be retried.
- **Duplicate warning**: starting a video you already downloaded in the same format, whose file still
  exists, asks for confirmation first.
- **Statistics** show downloads per day, week or month, the busiest hours and weekdays, streaks, top
  uploaders and formats, built from the download history.
- **Age-restricted or members-only videos** need cookies from a browser where you are signed in.
  Choose that browser under "Cookies from browser" in Settings.
- A system notification reports when a batch of downloads is done while the window is in the
  background. It can be turned off in Settings.
- **Language** is set in Settings: System (German if the system language is German, English
  otherwise), English or German.

Settings are stored in `~/.config/com.yaydl/settings.toml` on Linux and
`%APPDATA%\com.yaydl\settings.toml` on Windows. The queue (`queue.json`) and the download history
(`history.json`) are stored in `~/.local/share/com.yaydl/` on Linux and `%APPDATA%\com.yaydl\` on
Windows. A file yaydl cannot read is moved aside as `<name>.invalid-<timestamp>` and reported in the
window, never silently overwritten.

## How yt-dlp updates

The yt-dlp shipped inside the bundle is only a bootstrap. On first run yaydl copies it into the
application data directory and uses that copy from then on, so updates never touch the installed
bundle. At startup, and at most once every 12 hours, yaydl runs `yt-dlp --update-to <channel>` on
that copy. It also retries the update once automatically after a download fails, which covers the
common case of a site change that a newer yt-dlp already handles. The channel defaults to `nightly`
and can be changed in Settings. ffmpeg and ffprobe are bundled at a pinned build and are never
updated at runtime.

## Logs

yaydl writes one log file per day and keeps the last 7, in
`~/.local/share/com.yaydl/logs/` on Linux and `%LOCALAPPDATA%\com.yaydl\logs\` on Windows. Files are
named `yaydl.<YYYY-MM-DD>.log`, and the same lines go to stderr when yaydl is started from a
terminal. The log records startup paths, every yt-dlp invocation with its exit code, update checks,
queue changes, and what the window itself did, including a panic in the UI. Settings has a "Show
logs" button that displays the tail of today's file and copies it to the clipboard, which is the
quickest way to get a report out of a packaged build. The default verbosity is `info` for
everything and `debug` for the backend. Set `YAYDL_LOG=debug` or `YAYDL_LOG=yaydl_lib=trace` to
raise it. An invalid `YAYDL_LOG` stops the app at startup rather than falling back silently.

## Liability & License notice
yaydl and its maintainers cannot be held liable for misuse of this application,
as stated in the [MIT license](https://github.com/NiklasRhf/yaydl/blob/main/LICENSE).
The maintainers of yaydl do not in any way condone the use of this application in practices
that violate local laws such as but not limited to the DMCA. The maintainers of this application
call upon the personal responsibility of its users to use this application in a fair way, as it is intended to be used.

## Releasing

Releases are cut from `main` by pushing a tag. `scripts/release.sh <version>` bumps the version in
every manifest, commits, and creates the tag `yaydl-v<version>`. Pushing that tag runs the release
workflow, which builds the Linux and Windows bundles and publishes them, together with the updater
manifest, as a GitHub release. The workflow refuses a tag whose version does not match
`src-tauri/tauri.conf.json`.

```
scripts/release.sh 0.4.0
git push origin main yaydl-v0.4.0
```

Once the release workflow has uploaded the bundles, point the Arch package at them and commit:

```
scripts/update-pkgbuild.sh 0.4.0
```

## Arch Linux

On Arch, install the native package instead of the AppImage. The AppImage bundles its own, older
WebKitGTK, which scrolls noticeably worse than the system one. `packaging/arch/PKGBUILD` builds
`yaydl-bin` from the release `.deb`:

```
cd packaging/arch
makepkg -si
```

yaydl's own yt-dlp and ffmpeg go to `/usr/lib/yaydl`, so they do not clash with the `yt-dlp` and
`ffmpeg` packages. The in-app updater only updates AppImages, so update the package with pacman or
by rebuilding it from a newer PKGBUILD.
