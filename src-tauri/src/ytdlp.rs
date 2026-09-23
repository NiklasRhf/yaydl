use std::{
    fs,
    future::Future,
    io,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tracing::{debug, info, warn};
pub use tokio_util::sync::CancellationToken;
use yaydl_shared::{
    Browser, FriendlyError, OutputFormat, Progress, VideoMetadata, YtDlpChannel, YtDlpError,
    YtDlpUpdateEvent,
};

pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

/// Toasts show this text, so only the tail of a yt-dlp traceback is kept.
const STDERR_TAIL_LINES: usize = 8;

const STATE_FILE: &str = "state.toml";

#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }
}

pub trait Runner: Send + Sync + 'static {
    /// Run to completion, capture both streams.
    fn output(
        &self,
        program: &Path,
        args: &[String],
    ) -> impl Future<Output = io::Result<CommandOutput>> + Send;

    /// Stream stdout lines to `on_line` while running; return exit code + captured stderr.
    fn stream(
        &self,
        program: &Path,
        args: &[String],
        on_line: Box<dyn FnMut(&str) + Send>,
    ) -> impl Future<Output = io::Result<CommandOutput>> + Send;
}

/// Runs yt-dlp through `tokio::process` instead of `tauri_plugin_shell` so the
/// manager stays free of an `AppHandle` and can be driven from plain tests.
pub struct ProcessRunner;

impl ProcessRunner {
    fn command(program: &Path, args: &[String]) -> tokio::process::Command {
        debug!(binary = %program.display(), ?args, "spawning yt-dlp");
        let mut command = tokio::process::Command::new(program);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // CREATE_NO_WINDOW, otherwise every yt-dlp call flashes a console.
            command.creation_flags(0x0800_0000);
        }
        command
    }
}

impl Runner for ProcessRunner {
    async fn output(&self, program: &Path, args: &[String]) -> io::Result<CommandOutput> {
        let output = Self::command(program, args).output().await?;
        info!(
            exit_code = ?output.status.code(),
            stdout_bytes = output.stdout.len(),
            stderr_bytes = output.stderr.len(),
            "yt-dlp exited"
        );
        Ok(CommandOutput {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    async fn stream(
        &self,
        program: &Path,
        args: &[String],
        mut on_line: Box<dyn FnMut(&str) + Send>,
    ) -> io::Result<CommandOutput> {
        let mut child = Self::command(program, args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("yt-dlp stdout was not captured"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("yt-dlp stderr was not captured"))?;

        let stderr_task = tokio::spawn(async move {
            let mut buffer = String::new();
            stderr.read_to_string(&mut buffer).await.map(|_| buffer)
        });

        let mut collected_stdout = String::new();
        let mut lines = BufReader::new(stdout).lines();
        while let Some(line) = lines.next_line().await? {
            on_line(&line);
            collected_stdout.push_str(&line);
            collected_stdout.push('\n');
        }

        let status = child.wait().await?;
        let stderr = stderr_task.await.map_err(io::Error::other)??;
        info!(
            exit_code = ?status.code(),
            stdout_bytes = collected_stdout.len(),
            stderr_bytes = stderr.len(),
            "yt-dlp exited"
        );

        Ok(CommandOutput {
            exit_code: status.code(),
            stdout: collected_stdout,
            stderr,
        })
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct PersistedState {
    last_update_check_unix: u64,
}

pub type ExecFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, YtDlpError>> + Send + 'a>>;

pub struct YtDlp<R> {
    runner: R,
    data_dir: PathBuf,
    binary: PathBuf,
    bootstrap: PathBuf,
    ffmpeg_dir: PathBuf,
}

impl<R: Runner> YtDlp<R> {
    pub fn new(runner: R, data_dir: PathBuf, bootstrap: PathBuf, ffmpeg_dir: PathBuf) -> Self {
        let binary = data_dir.join(format!("yt-dlp{}", std::env::consts::EXE_SUFFIX));
        Self {
            runner,
            data_dir,
            binary,
            bootstrap,
            ffmpeg_dir,
        }
    }

    pub fn binary(&self) -> &Path {
        &self.binary
    }

    pub fn state_file(&self) -> PathBuf {
        self.data_dir.join(STATE_FILE)
    }

    /// Copies the bundled yt-dlp into the app data dir on first run. Everything
    /// afterwards, including `--update-to`, operates on that writable copy.
    pub fn ensure_installed(&self) -> Result<(), YtDlpError> {
        if self.binary.exists() {
            info!(path = %self.binary.display(), "managed yt-dlp already present");
            return Ok(());
        }
        if !self.bootstrap.exists() {
            return Err(YtDlpError::Bootstrap(format!(
                "no managed yt-dlp at {} and no bundled yt-dlp at {}",
                self.binary.display(),
                self.bootstrap.display()
            )));
        }
        fs::create_dir_all(&self.data_dir).map_err(|e| {
            YtDlpError::Bootstrap(format!("creating {} failed: {e}", self.data_dir.display()))
        })?;
        fs::copy(&self.bootstrap, &self.binary).map_err(|e| {
            YtDlpError::Bootstrap(format!(
                "copying {} to {} failed: {e}",
                self.bootstrap.display(),
                self.binary.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.binary, fs::Permissions::from_mode(0o755)).map_err(|e| {
                YtDlpError::Bootstrap(format!(
                    "making {} executable failed: {e}",
                    self.binary.display()
                ))
            })?;
        }
        info!(
            from = %self.bootstrap.display(),
            path = %self.binary.display(),
            "copied bootstrap yt-dlp"
        );
        Ok(())
    }

    pub async fn version(&self) -> Result<String, YtDlpError> {
        let args = vec![String::from("--version")];
        let output = self.runner.output(&self.binary, &args).await.map_err(|e| {
            YtDlpError::Io(format!(
                "running {} --version failed: {e}",
                self.binary.display()
            ))
        })?;
        Ok(check(output, &args)?.stdout.trim().to_string())
    }

    pub async fn update(&self, channel: YtDlpChannel) -> Result<YtDlpUpdateEvent, YtDlpError> {
        let before = self.version().await?;
        let args = vec![String::from("--update-to"), channel.as_str().to_string()];
        let output = self.runner.output(&self.binary, &args).await.map_err(|e| {
            YtDlpError::Io(format!(
                "running {} --update-to {channel} failed: {e}",
                self.binary.display()
            ))
        })?;
        if !output.succeeded() {
            warn!(
                %channel,
                %before,
                exit_code = ?output.exit_code,
                stderr = %output.stderr,
                "yt-dlp update exited non-zero"
            );
            return Err(YtDlpError::UpdateFailed {
                channel,
                stderr: stderr_tail(&output.stderr),
            });
        }
        let after = self.version().await?;
        self.record_update_check()?;
        info!(
            %channel,
            %before,
            %after,
            exit_code = ?output.exit_code,
            "yt-dlp update finished"
        );

        Ok(if before == after {
            YtDlpUpdateEvent::AlreadyCurrent {
                version: after,
                channel,
            }
        } else {
            YtDlpUpdateEvent::Updated {
                from: before,
                to: after,
                channel,
            }
        })
    }

    pub async fn maybe_update_on_startup(
        &self,
        channel: YtDlpChannel,
    ) -> Result<Option<YtDlpUpdateEvent>, YtDlpError> {
        let path = self.state_file();
        match fs::read_to_string(&path) {
            Ok(raw) => {
                let state: PersistedState = toml::from_str(&raw)
                    .map_err(|e| YtDlpError::InvalidState(format!("{}: {e}", path.display())))?;
                let due = update_check_due(state.last_update_check_unix, now_unix()?);
                info!(
                    state_file = %path.display(),
                    existed = true,
                    last_check_unix = state.last_update_check_unix,
                    due,
                    "yt-dlp startup update check"
                );
                if !due {
                    info!(%channel, "yt-dlp startup update skipped, checked recently");
                    return Ok(None);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                info!(
                    state_file = %path.display(),
                    existed = false,
                    due = true,
                    "yt-dlp startup update check"
                );
            }
            Err(e) => {
                return Err(YtDlpError::Io(format!(
                    "reading {} failed: {e}",
                    path.display()
                )))
            }
        }
        let event = self.update(channel).await?;
        info!(?event, "yt-dlp startup update produced an event");
        Ok(Some(event))
    }

    /// Runs `exec`, and on a non-zero yt-dlp exit updates once and retries once.
    /// A broken extractor is the usual cause and a fresh yt-dlp the usual fix.
    pub async fn run_with_self_heal<'a, T, F>(
        &'a self,
        channel: YtDlpChannel,
        args: &'a [String],
        exec: F,
    ) -> Result<T, YtDlpError>
    where
        F: Fn(&'a R, &'a Path, &'a [String]) -> ExecFuture<'a, T> + Send,
    {
        match exec(&self.runner, &self.binary, args).await {
            Ok(value) => Ok(value),
            Err(failure @ YtDlpError::CommandFailed { .. }) => {
                warn!(%failure, %channel, "updating yt-dlp and retrying once");
                self.update(channel).await?;
                let retry = exec(&self.runner, &self.binary, args).await;
                match &retry {
                    Ok(_) => info!(%channel, "yt-dlp retry after update succeeded"),
                    Err(e) => warn!(error = %e, %channel, "yt-dlp retry after update failed"),
                }
                retry
            }
            Err(other) => Err(other),
        }
    }

    pub fn metadata_args(&self, url: &str) -> Vec<String> {
        [
            "--no-playlist",
            "--get-id",
            "--get-title",
            "--get-duration",
            "--get-thumbnail",
            url,
        ]
        .iter()
        .map(|a| a.to_string())
        .collect()
    }

    pub fn download_args(&self, url: &str, output_dir: &str, output_format: &str) -> Vec<String> {
        [
            "--no-playlist",
            "--newline",
            "-x",
            "--audio-format",
            output_format,
            "-o",
            &format!("{output_dir}/%(title)s.%(ext)s"),
            "--ffmpeg-location",
            &self.ffmpeg_dir.display().to_string(),
            url,
        ]
        .iter()
        .map(|a| a.to_string())
        .collect()
    }

    fn record_update_check(&self) -> Result<(), YtDlpError> {
        let path = self.state_file();
        fs::create_dir_all(&self.data_dir).map_err(|e| {
            YtDlpError::Io(format!("creating {} failed: {e}", self.data_dir.display()))
        })?;
        let state = PersistedState {
            last_update_check_unix: now_unix()?,
        };
        let serialized = toml::to_string(&state)
            .map_err(|e| YtDlpError::Io(format!("serializing yt-dlp state failed: {e}")))?;
        fs::write(&path, serialized)
            .map_err(|e| YtDlpError::Io(format!("writing {} failed: {e}", path.display())))
    }
}

/// Turns a non-zero exit into `CommandFailed` and logs the full stderr, which
/// the trimmed error message deliberately drops.
pub fn check(output: CommandOutput, args: &[String]) -> Result<CommandOutput, YtDlpError> {
    if output.succeeded() {
        return Ok(output);
    }
    warn!(
        ?args,
        exit_code = ?output.exit_code,
        stderr = %output.stderr,
        "yt-dlp exited non-zero"
    );
    Err(YtDlpError::CommandFailed {
        exit_code: output.exit_code,
        stderr: stderr_tail(&output.stderr),
    })
}

fn stderr_tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let start = lines.len().saturating_sub(STDERR_TAIL_LINES);
    lines[start..].join("\n")
}

fn now_unix() -> Result<u64, YtDlpError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| YtDlpError::Io(format!("system clock is before the unix epoch: {e}")))
}

/// A timestamp in the future is not a record of a past check, so it counts as due.
fn update_check_due(last_check: u64, now: u64) -> bool {
    match now.checked_sub(last_check) {
        Some(elapsed) => elapsed >= UPDATE_CHECK_INTERVAL.as_secs(),
        None => true,
    }
}

// --- Contract for the queue. The engine track replaces the `todo!()` bodies and
// keeps these signatures; the old `run_with_self_heal`/`*_args` API may go.

#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    pub channel: YtDlpChannel,
    pub cookies_from_browser: Option<Browser>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    Single(VideoMetadata),
    /// Entries come from `--flat-playlist`, `webpage_url` is each entry's URL.
    Playlist {
        title: Option<String>,
        entries: Vec<VideoMetadata>,
        /// Entries without an id or URL (deleted or private videos), skipped.
        unavailable: u32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadRequest {
    pub url: String,
    pub output_dir: PathBuf,
    pub format: OutputFormat,
    /// Validated with `yaydl_shared::validate_file_stem`. `None` is `<title> [<id>]`.
    pub file_stem: Option<String>,
    pub embed_metadata: bool,
    pub run: RunOptions,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadEvent {
    Progress(Progress),
    Processing,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadOutcome {
    pub file_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
pub enum YtDlpFailure {
    Cancelled,
    Failed(FriendlyError),
}

impl<R: Runner> YtDlp<R> {
    pub async fn resolve(&self, url: &str, run: &RunOptions) -> Result<Resolved, YtDlpFailure> {
        let _ = (url, run);
        todo!("engine track")
    }

    pub async fn download(
        &self,
        request: &DownloadRequest,
        cancel: CancellationToken,
        on_event: Box<dyn FnMut(DownloadEvent) + Send>,
    ) -> Result<DownloadOutcome, YtDlpFailure> {
        let _ = (request, cancel, on_event);
        todo!("engine track")
    }
}

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<(PathBuf, Vec<String>)>>>;

    struct FakeRunner {
        calls: Calls,
        scripted: Mutex<VecDeque<CommandOutput>>,
    }

    impl FakeRunner {
        fn new(scripted: Vec<CommandOutput>) -> (Self, Calls) {
            let calls: Calls = Arc::new(Mutex::new(Vec::new()));
            (
                Self {
                    calls: Arc::clone(&calls),
                    scripted: Mutex::new(scripted.into()),
                },
                calls,
            )
        }

        fn next(&self, program: &Path, args: &[String]) -> CommandOutput {
            self.calls
                .lock()
                .unwrap()
                .push((program.to_path_buf(), args.to_vec()));
            self.scripted
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("unscripted yt-dlp invocation: {args:?}"))
        }
    }

    impl Runner for FakeRunner {
        async fn output(&self, program: &Path, args: &[String]) -> io::Result<CommandOutput> {
            Ok(self.next(program, args))
        }

        async fn stream(
            &self,
            program: &Path,
            args: &[String],
            mut on_line: Box<dyn FnMut(&str) + Send>,
        ) -> io::Result<CommandOutput> {
            let output = self.next(program, args);
            for line in output.stdout.lines() {
                on_line(line);
            }
            Ok(output)
        }
    }

    fn ok(stdout: &str) -> CommandOutput {
        CommandOutput {
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
        }
    }

    fn fail(stderr: &str) -> CommandOutput {
        CommandOutput {
            exit_code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
        }
    }

    fn manager(scripted: Vec<CommandOutput>) -> (YtDlp<FakeRunner>, Calls, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let (runner, calls) = FakeRunner::new(scripted);
        let manager = YtDlp::new(
            runner,
            dir.path().to_path_buf(),
            dir.path().join("bundled-yt-dlp"),
            dir.path().join("ffmpeg"),
        );
        (manager, calls, dir)
    }

    fn args_of(calls: &Calls) -> Vec<Vec<String>> {
        calls
            .lock()
            .unwrap()
            .iter()
            .map(|(_, args)| args.clone())
            .collect()
    }

    fn download_exec<'a>(
        runner: &'a FakeRunner,
        binary: &'a Path,
        args: &'a [String],
    ) -> ExecFuture<'a, ()> {
        Box::pin(async move {
            let output = runner
                .stream(binary, args, Box::new(|_| {}))
                .await
                .map_err(|e| YtDlpError::Io(e.to_string()))?;
            check(output, args).map(|_| ())
        })
    }

    fn version_args() -> Vec<String> {
        vec!["--version".to_string()]
    }

    fn update_args(channel: &str) -> Vec<String> {
        vec!["--update-to".to_string(), channel.to_string()]
    }

    #[tokio::test]
    async fn failed_download_updates_to_nightly_and_retries_once() {
        let (manager, calls, _dir) = manager(vec![
            fail("ERROR: [youtube] Unable to extract player response"),
            ok("2026.02.03"),
            ok("Updated yt-dlp to nightly@2026.09.18.232920"),
            ok("2026.09.18.232920"),
            ok("[download] 100.0% of 3.00MiB"),
        ]);
        let args = manager.download_args("https://youtu.be/x", "/tmp/out", "mp3");

        let result = manager
            .run_with_self_heal(YtDlpChannel::Nightly, &args, download_exec)
            .await;

        assert!(result.is_ok(), "expected retry to succeed, got {result:?}");
        assert_eq!(
            args_of(&calls),
            vec![
                args.clone(),
                version_args(),
                update_args("nightly"),
                version_args(),
                args,
            ]
        );
    }

    #[tokio::test]
    async fn failed_download_updates_to_stable_when_configured() {
        let (manager, calls, _dir) = manager(vec![
            fail("ERROR: [youtube] Unable to extract player response"),
            ok("2026.02.03"),
            ok(""),
            ok("2026.03.31"),
            ok("[download] 100.0% of 3.00MiB"),
        ]);
        let args = manager.download_args("https://youtu.be/x", "/tmp/out", "mp3");

        let result = manager
            .run_with_self_heal(YtDlpChannel::Stable, &args, download_exec)
            .await;

        assert!(result.is_ok(), "expected retry to succeed, got {result:?}");
        assert_eq!(args_of(&calls)[2], update_args("stable"));
    }

    #[tokio::test]
    async fn failing_retry_reports_the_retry_error_and_updates_only_once() {
        let (manager, calls, _dir) = manager(vec![
            fail("ERROR: first attempt"),
            ok("2026.02.03"),
            ok(""),
            ok("2026.09.18.232920"),
            fail("ERROR: retry attempt"),
        ]);
        let args = manager.download_args("https://youtu.be/x", "/tmp/out", "mp3");

        let result = manager
            .run_with_self_heal(YtDlpChannel::Nightly, &args, download_exec)
            .await;

        match result {
            Err(YtDlpError::CommandFailed { exit_code, stderr }) => {
                assert_eq!(exit_code, Some(1));
                assert_eq!(stderr, "ERROR: retry attempt");
            }
            other => panic!("expected CommandFailed, got {other:?}"),
        }
        let invocations = args_of(&calls);
        assert_eq!(invocations.len(), 5);
        assert_eq!(
            invocations
                .iter()
                .filter(|a| a.first().map(String::as_str) == Some("--update-to"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn failed_update_is_reported_instead_of_the_download_error() {
        let (manager, calls, _dir) = manager(vec![
            fail("ERROR: [youtube] Unable to extract player response"),
            ok("2026.02.03"),
            fail("ERROR: unable to write to /opt/yt-dlp"),
        ]);
        let args = manager.download_args("https://youtu.be/x", "/tmp/out", "mp3");

        let result = manager
            .run_with_self_heal(YtDlpChannel::Nightly, &args, download_exec)
            .await;

        match result {
            Err(YtDlpError::UpdateFailed { channel, stderr }) => {
                assert_eq!(channel, YtDlpChannel::Nightly);
                assert_eq!(stderr, "ERROR: unable to write to /opt/yt-dlp");
            }
            other => panic!("expected UpdateFailed, got {other:?}"),
        }
        assert_eq!(
            args_of(&calls),
            vec![args, version_args(), update_args("nightly")]
        );
    }

    #[tokio::test]
    async fn successful_download_never_updates() {
        let (manager, calls, _dir) = manager(vec![ok("[download] 100.0% of 3.00MiB")]);
        let args = manager.download_args("https://youtu.be/x", "/tmp/out", "mp3");

        manager
            .run_with_self_heal(YtDlpChannel::Nightly, &args, download_exec)
            .await
            .expect("download to succeed");

        assert_eq!(args_of(&calls), vec![args]);
    }

    #[tokio::test]
    async fn startup_without_state_file_updates_and_records_the_check() {
        let (manager, calls, _dir) =
            manager(vec![ok("2026.02.03"), ok(""), ok("2026.09.18.232920")]);

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
        assert_eq!(args_of(&calls).len(), 3);
        assert!(manager.state_file().exists());
    }

    #[tokio::test]
    async fn startup_within_the_check_interval_does_nothing() {
        let (manager, calls, _dir) = manager(Vec::new());
        let recent = now_unix().unwrap() - 60;
        fs::write(
            manager.state_file(),
            format!("last_update_check_unix = {recent}\n"),
        )
        .unwrap();

        let event = manager
            .maybe_update_on_startup(YtDlpChannel::Nightly)
            .await
            .expect("startup check to succeed");

        assert_eq!(event, None);
        assert!(args_of(&calls).is_empty());
    }

    #[tokio::test]
    async fn startup_with_a_malformed_state_file_fails_loudly() {
        let (manager, calls, _dir) = manager(Vec::new());
        fs::write(manager.state_file(), "last_update_check_unix = yesterday\n").unwrap();

        let result = manager.maybe_update_on_startup(YtDlpChannel::Nightly).await;

        match result {
            Err(YtDlpError::InvalidState(message)) => assert!(
                message.contains(&manager.state_file().display().to_string()),
                "expected the state file path in {message}"
            ),
            other => panic!("expected InvalidState, got {other:?}"),
        }
        assert!(args_of(&calls).is_empty());
    }

    #[tokio::test]
    async fn startup_past_the_check_interval_updates() {
        let (manager, calls, _dir) = manager(vec![ok("2026.02.03"), ok(""), ok("2026.02.03")]);
        let stale = now_unix().unwrap() - UPDATE_CHECK_INTERVAL.as_secs() - 1;
        fs::write(
            manager.state_file(),
            format!("last_update_check_unix = {stale}\n"),
        )
        .unwrap();

        let event = manager
            .maybe_update_on_startup(YtDlpChannel::Nightly)
            .await
            .expect("startup check to succeed");

        assert_eq!(
            event,
            Some(YtDlpUpdateEvent::AlreadyCurrent {
                version: "2026.02.03".to_string(),
                channel: YtDlpChannel::Nightly,
            })
        );
        assert_eq!(args_of(&calls).len(), 3);
    }

    #[tokio::test]
    async fn update_reports_already_current_for_an_unchanged_version() {
        let (manager, _calls, _dir) = manager(vec![ok("2026.09.18"), ok(""), ok("2026.09.18")]);

        let event = manager.update(YtDlpChannel::Nightly).await.unwrap();

        assert_eq!(
            event,
            YtDlpUpdateEvent::AlreadyCurrent {
                version: "2026.09.18".to_string(),
                channel: YtDlpChannel::Nightly,
            }
        );
    }

    #[tokio::test]
    async fn stderr_is_trimmed_to_the_last_lines() {
        let noisy = (0..20)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let tail = stderr_tail(&noisy);

        assert_eq!(tail.lines().count(), STDERR_TAIL_LINES);
        assert!(tail.starts_with("line 12"));
        assert!(tail.ends_with("line 19"));
    }
}
