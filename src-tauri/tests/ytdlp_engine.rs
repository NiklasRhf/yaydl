// AGENT CODE: claude-opus-5
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yaydl_lib::ytdlp::{
    CancellationToken, DownloadEvent, DownloadRequest, ProcessRunner, Resolved, RunOptions, YtDlp,
    YtDlpFailure,
};
use yaydl_shared::{AudioCodec, OutputFormat, VideoQuality, YtDlpChannel, YtDlpUpdateEvent};

/// Stands in for yt-dlp: the first download fails like a stale extractor does,
/// `--update-to` drops a marker, and every download afterwards succeeds and
/// prints the markers the engine asks for, post-processing on stderr as the
/// real yt-dlp does.
const SELF_HEALING_YT_DLP: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
marker="$dir/updated"
case "$1" in
  --version)
    if [ -f "$marker" ]; then echo 2026.09.18.232920; else echo 2026.02.03; fi
    ;;
  --update-to)
    touch "$marker"
    ;;
  *)
    if [ -f "$marker" ]; then
      out="$dir/out/Song [x].mp3"
      echo "yaydl-progress:downloading|512|1024|NA|NA|NA|$out.part"
      echo "yaydl-progress:finished|1024|1024|NA|NA|NA|NA"
      echo "yaydl-pp:started|ExtractAudio" >&2
      touch "$out"
      echo "yaydl-file:$out"
    else
      echo "ERROR: [youtube] x: Unable to extract player response" >&2
      exit 1
    fi
    ;;
esac
"#;

/// Starts a background child, the way yt-dlp starts ffmpeg, and waits for it.
const HANGING_YT_DLP: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
touch "$dir/out/clip.webm.part"
echo "yaydl-progress:downloading|1|100|NA|NA|NA|$dir/out/clip.webm.part"
sleep 30 &
echo $! > "$dir/child.pid"
wait
"#;

fn manager_with_script(dir: &Path, script: &str) -> YtDlp<ProcessRunner> {
    let manager = YtDlp::new(
        ProcessRunner,
        dir.to_path_buf(),
        dir.join("bundled-yt-dlp"),
        dir.join("ffmpeg"),
    );
    fs::create_dir_all(dir.join("out")).unwrap();
    fs::write(manager.binary(), script).unwrap();
    fs::set_permissions(manager.binary(), fs::Permissions::from_mode(0o755)).unwrap();
    manager
}

fn mp3_request(dir: &Path) -> DownloadRequest {
    DownloadRequest {
        url: "https://www.youtube.com/watch?v=jNQXAC9IVRw".to_string(),
        output_dir: dir.join("out"),
        format: OutputFormat::Audio {
            codec: AudioCodec::Mp3,
        },
        file_stem: None,
        embed_metadata: true,
        run: RunOptions {
            channel: YtDlpChannel::Nightly,
            cookies_from_browser: None,
        },
    }
}

fn process_exists(pid: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()
        .expect("running kill -0")
        .success()
}

#[tokio::test]
async fn a_stale_yt_dlp_updates_itself_and_the_retried_download_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_script(dir.path(), SELF_HEALING_YT_DLP);
    let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&events);

    let outcome = manager
        .download(
            &mp3_request(dir.path()),
            CancellationToken::new(),
            Box::new(move |event| sink.lock().unwrap().push(event)),
        )
        .await
        .expect("the retry to succeed");

    assert_eq!(outcome.file_path, dir.path().join("out/Song [x].mp3"));
    let events = events.lock().unwrap().clone();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DownloadEvent::Progress(p) if p.percent == Some(100.0))),
        "the finished progress never arrived: {events:?}"
    );
    assert_eq!(events.last(), Some(&DownloadEvent::Processing));
    assert!(dir.path().join("updated").exists(), "no update was run");
    assert!(
        manager.state_file().exists(),
        "update check was not recorded"
    );
}

/// Like `HANGING_YT_DLP`, but the script and its child ignore SIGTERM, so only
/// the SIGKILL after the grace period ends them.
const STUBBORN_YT_DLP: &str = r#"#!/bin/sh
dir="$(dirname "$0")"
trap '' TERM
touch "$dir/out/clip.webm.part"
echo "yaydl-progress:downloading|1|100|NA|NA|NA|$dir/out/clip.webm.part"
sleep 30 &
echo $! > "$dir/child.pid"
wait
"#;

#[tokio::test]
async fn cancelling_kills_yt_dlp_and_its_children_and_removes_the_partial_file() {
    let took = cancel_hanging_script(HANGING_YT_DLP).await;
    assert!(took < Duration::from_secs(1), "SIGTERM took {took:?}");
}

#[tokio::test]
async fn children_ignoring_sigterm_are_killed_after_the_grace_period() {
    let took = cancel_hanging_script(STUBBORN_YT_DLP).await;
    assert!(
        took >= Duration::from_millis(2900),
        "returned after {took:?}, before the SIGTERM grace period ended"
    );
}

/// Cancels a download once the script's child runs, asserts `Cancelled` came
/// back within 5 s, the child is gone and the partial file removed, and
/// returns how long the cancel took.
async fn cancel_hanging_script(script: &str) -> Duration {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_script(dir.path(), script);
    let pid_file = dir.path().join("child.pid");
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    let cancelled_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    let cancelled_at_writer = Arc::clone(&cancelled_at);
    let request = mp3_request(dir.path());

    let cancel_once_the_child_runs = async {
        let deadline = Instant::now() + Duration::from_secs(5);
        while fs::read_to_string(&pid_file).map_or(true, |pid| pid.trim().is_empty()) {
            assert!(
                Instant::now() < deadline,
                "the fake never started its child"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        *cancelled_at_writer.lock().unwrap() = Some(Instant::now());
        trigger.cancel();
    };
    let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            manager.download(&request, cancel, Box::new(|_| {})),
            cancel_once_the_child_runs
        )
    })
    .await
    .expect("the cancelled download to return");

    let took = cancelled_at
        .lock()
        .unwrap()
        .expect("cancel fired")
        .elapsed();
    assert_eq!(result, Err(YtDlpFailure::Cancelled));
    assert!(took < Duration::from_secs(5), "cancelling took {took:?}");

    let pid = fs::read_to_string(&pid_file).unwrap().trim().to_string();
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_exists(&pid) {
        assert!(
            Instant::now() < deadline,
            "the child sleep {pid} survived the cancel"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        !dir.path().join("out/clip.webm.part").exists(),
        "the partial file was left behind"
    );
    took
}

fn required_env(name: &str) -> PathBuf {
    PathBuf::from(
        std::env::var(name).unwrap_or_else(|e| panic!("the live test needs {name} set: {e}")),
    )
}

fn live_manager(dir: &Path) -> (YtDlp<ProcessRunner>, RunOptions) {
    let manager = YtDlp::new(
        ProcessRunner,
        dir.join("data"),
        required_env("YAYDL_LIVE_YT_DLP"),
        required_env("YAYDL_LIVE_FFMPEG_DIR"),
    );
    manager.ensure_installed().unwrap();
    // A recorded check keeps a transient failure from updating the bundled copy.
    fs::write(
        manager.state_file(),
        format!(
            "last_update_check_unix = {}\n",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        ),
    )
    .unwrap();
    let run = RunOptions {
        channel: YtDlpChannel::Nightly,
        cookies_from_browser: None,
    };
    (manager, run)
}

/// Hits YouTube with the real yt-dlp, so it only runs on request:
/// `YAYDL_LIVE_YT_DLP=<yt-dlp binary> YAYDL_LIVE_FFMPEG_DIR=<dir with ffmpeg
/// and ffprobe> cargo test -p yaydl --test ytdlp_engine -- --ignored --nocapture`
#[tokio::test]
#[ignore = "downloads from YouTube with the real yt-dlp"]
async fn live_resolve_and_download_with_the_real_yt_dlp() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, run) = live_manager(dir.path());

    let single = manager
        .resolve(
            "https://www.youtube.com/watch?v=jNQXAC9IVRw&list=PLbpi6ZahtOH6Blw3RGYpWkSByi_T7Rygb",
            &run,
        )
        .await
        .unwrap();
    println!("single: {single:?}");
    assert!(matches!(single, Resolved::Single(ref m) if m.video_id == "jNQXAC9IVRw"));

    let playlist = manager
        .resolve(
            "https://www.youtube.com/playlist?list=PLbpi6ZahtOH6Blw3RGYpWkSByi_T7Rygb",
            &run,
        )
        .await
        .unwrap();
    let Resolved::Playlist {
        title,
        entries,
        unavailable,
    } = &playlist
    else {
        panic!("expected a playlist, got {playlist:?}");
    };
    println!(
        "playlist {title:?}: {} entries, {unavailable} unavailable, first {:?}",
        entries.len(),
        entries.first()
    );
    assert!(!entries.is_empty());

    for format in [
        OutputFormat::Audio {
            codec: AudioCodec::Mp3,
        },
        OutputFormat::Video {
            quality: VideoQuality::P720,
        },
    ] {
        let mut request = mp3_request(dir.path());
        request.format = format;
        let events: Arc<Mutex<Vec<DownloadEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        let outcome = manager
            .download(
                &request,
                CancellationToken::new(),
                Box::new(move |event| sink.lock().unwrap().push(event)),
            )
            .await
            .unwrap();
        let size = fs::metadata(&outcome.file_path).unwrap().len();
        println!("{format}: {} ({size} bytes)", outcome.file_path.display());
        for event in events.lock().unwrap().iter() {
            println!("  {event:?}");
        }
        assert!(events.lock().unwrap().contains(&DownloadEvent::Processing));
    }
}

/// Run like the test above.
#[tokio::test]
#[ignore = "resolves a YouTube playlist with the real yt-dlp"]
async fn live_resolve_playlist_lists_the_playlist_of_a_video_link() {
    let dir = tempfile::tempdir().unwrap();
    let (manager, run) = live_manager(dir.path());

    let resolved = manager
        .resolve_playlist(
            "https://www.youtube.com/watch?v=viuYLuyILeo&list=PLhCCfdELbr0Pi9RLMNAGhzKyF50LqlB4U",
            &run,
        )
        .await
        .unwrap();

    let Resolved::Playlist {
        title,
        entries,
        unavailable,
    } = &resolved
    else {
        panic!("expected a playlist, got {resolved:?}");
    };
    println!(
        "playlist {title:?}: {} entries, {unavailable} unavailable",
        entries.len()
    );
    assert_eq!(entries.len() + *unavailable as usize, 13);
    assert!(entries.iter().any(|e| e.video_id == "viuYLuyILeo"));
}

#[tokio::test]
async fn startup_check_updates_a_fresh_install_and_reports_the_new_version() {
    let dir = tempfile::tempdir().unwrap();
    let manager = manager_with_script(dir.path(), SELF_HEALING_YT_DLP);

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
    fs::write(&bootstrap, SELF_HEALING_YT_DLP).unwrap();
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
    assert_eq!(
        fs::read_to_string(manager.binary()).unwrap(),
        SELF_HEALING_YT_DLP
    );
}
