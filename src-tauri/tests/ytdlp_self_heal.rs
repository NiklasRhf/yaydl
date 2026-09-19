// AGENT CODE: claude-opus-5
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};

use yaydl_lib::ytdlp::{check, ExecFuture, ProcessRunner, Runner, YtDlp};
use yaydl_shared::{YtDlpChannel, YtDlpUpdateEvent};

/// Stands in for yt-dlp: the first download fails like a stale extractor does,
/// `--update-to` drops a marker, and every call afterwards succeeds.
const FAKE_YT_DLP: &str = r#"#!/bin/sh
marker="$(dirname "$0")/updated"
case "$1" in
  --version)
    if [ -f "$marker" ]; then echo 2026.09.18.232920; else echo 2026.02.03; fi
    ;;
  --update-to)
    touch "$marker"
    ;;
  *)
    if [ -f "$marker" ]; then
      echo "[download] Destination: song.webm"
      echo "[download] 100.0% of 3.00MiB"
    else
      echo "ERROR: [youtube] Unable to extract player response" >&2
      exit 1
    fi
    ;;
esac
"#;

fn install_fake(manager: &YtDlp<ProcessRunner>) {
    fs::create_dir_all(manager.binary().parent().unwrap()).unwrap();
    fs::write(manager.binary(), FAKE_YT_DLP).unwrap();
    fs::set_permissions(manager.binary(), fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn a_stale_yt_dlp_updates_itself_and_the_retried_download_reports_progress() {
    let dir = tempfile::tempdir().unwrap();
    let manager = YtDlp::new(
        ProcessRunner,
        dir.path().to_path_buf(),
        dir.path().join("bundled-yt-dlp"),
        dir.path().join("ffmpeg"),
    );
    install_fake(&manager);

    let args = manager.download_args(
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
        dir.path().to_str().unwrap(),
        "mp3",
    );
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let result = manager
        .run_with_self_heal(YtDlpChannel::Nightly, &args, |runner, binary, args| {
            let seen = Arc::clone(&seen);
            Box::pin(async move {
                let output = runner
                    .stream(
                        binary,
                        args,
                        Box::new(move |line| seen.lock().unwrap().push(line.to_string())),
                    )
                    .await
                    .map_err(|e| yaydl_shared::YtDlpError::Io(e.to_string()))?;
                check(output, args).map(|_| ())
            }) as ExecFuture<'_, ()>
        })
        .await;

    assert!(
        result.is_ok(),
        "expected the retry to succeed, got {result:?}"
    );
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|line| line.contains("100.0%")),
        "progress callback never saw the finished download: {seen:?}"
    );
    assert!(dir.path().join("updated").exists(), "no update was run");
    assert!(
        manager.state_file().exists(),
        "update check was not recorded"
    );
}

#[tokio::test]
async fn startup_check_updates_a_fresh_install_and_reports_the_new_version() {
    let dir = tempfile::tempdir().unwrap();
    let manager = YtDlp::new(
        ProcessRunner,
        dir.path().to_path_buf(),
        dir.path().join("bundled-yt-dlp"),
        dir.path().join("ffmpeg"),
    );
    install_fake(&manager);

    let event = manager
        .maybe_update_on_startup(YtDlpChannel::Nightly)
        .await
        .expect("startup check to succeed");

    assert_eq!(
        event,
        Some(YtDlpUpdateEvent::Updated {
            from: "2026.02.03".to_string(),
            to: "2026.09.18.232920".to_string(),
            channel: YtDlpChannel::Nightly,
        })
    );

    let second = manager
        .maybe_update_on_startup(YtDlpChannel::Nightly)
        .await
        .expect("second startup check to succeed");
    assert_eq!(second, None, "the recorded check interval was ignored");
}

#[tokio::test]
async fn a_missing_bootstrap_binary_is_reported_with_both_paths() {
    let dir = tempfile::tempdir().unwrap();
    let manager = YtDlp::new(
        ProcessRunner,
        dir.path().join("yt-dlp"),
        dir.path().join("bundled-yt-dlp"),
        dir.path().to_path_buf(),
    );

    let error = manager.ensure_installed().expect_err("bootstrap to fail");
    let message = error.to_string();

    assert!(message.contains("bundled-yt-dlp"), "got {message}");
    assert!(message.contains("yt-dlp"), "got {message}");
}

#[test]
fn ensure_installed_copies_the_bootstrap_binary_and_makes_it_executable() {
    let dir = tempfile::tempdir().unwrap();
    let bootstrap = dir.path().join("bundled-yt-dlp");
    fs::write(&bootstrap, FAKE_YT_DLP).unwrap();
    fs::set_permissions(&bootstrap, fs::Permissions::from_mode(0o644)).unwrap();

    let manager = YtDlp::new(
        ProcessRunner,
        dir.path().join("managed"),
        bootstrap,
        dir.path().to_path_buf(),
    );
    manager.ensure_installed().expect("bootstrap to succeed");

    let mode = fs::metadata(manager.binary()).unwrap().permissions().mode();
    assert_eq!(mode & 0o111, 0o111, "copied binary is not executable");
    assert_eq!(fs::read_to_string(manager.binary()).unwrap(), FAKE_YT_DLP);
}
