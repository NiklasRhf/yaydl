# YaYDL

Yet Another YouTube Downloader

## How yt-dlp updates

The yt-dlp shipped inside the bundle is only a bootstrap. On first run yaydl copies it into the
application data directory and uses that copy from then on, so updates never touch the installed
bundle. At startup, and at most once every 12 hours, yaydl runs `yt-dlp --update-to <channel>` on
that copy. It also retries the update once automatically after a download fails, which covers the
common case of a site change that a newer yt-dlp already handles. The channel defaults to `nightly`
and can be changed in Settings. ffmpeg and ffprobe are bundled at a pinned build and are never
updated at runtime.

## Logs

yaydl writes a log file next to its application data, at `~/.local/share/com.yaydl/logs/yaydl.log`
on Linux and `%LOCALAPPDATA%\com.yaydl\logs\yaydl.log` on Windows, and mirrors the same lines to
stderr when started from a terminal. It records startup paths, every yt-dlp invocation with its
exit code, update checks, and what the window itself did, including a panic in the UI. Settings has
a "Show logs" button that displays the last 300 lines and copies them to the clipboard, which is
the quickest way to get a report out of a packaged build. The default verbosity is `info` for
everything and `debug` for the backend; set `YAYDL_LOG=debug` or `YAYDL_LOG=yaydl_lib=trace` to
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
