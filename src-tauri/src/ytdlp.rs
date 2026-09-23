use std::{
    borrow::Cow,
    fs,
    future::Future,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::RwLock;
pub use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use yaydl_shared::{
    Browser, ErrorKind, FriendlyError, OutputFormat, Progress, VideoMetadata, YtDlpChannel,
    YtDlpError, YtDlpUpdateEvent,
};

pub const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(12 * 60 * 60);

/// A video that keeps failing for a reason no update can fix would otherwise
/// trigger an update on every retry.
pub const SELF_HEAL_MIN_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Toasts show this text, so only the tail of a yt-dlp traceback is kept.
const STDERR_TAIL_LINES: usize = 8;

/// The queue re-emits every progress event to the webview, which does not need
/// more than a few repaints per second.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

/// How long a cancelled yt-dlp and its ffmpeg children get to exit after
/// SIGTERM before they are killed.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_secs(3);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

const STATE_FILE: &str = "state.toml";

const PROGRESS_MARKER: &str = "yaydl-progress:";
const POSTPROCESS_MARKER: &str = "yaydl-pp:";
const FILE_MARKER: &str = "yaydl-file:";

/// `tmpfilename` goes last because it is the only field that can contain `|`.
const PROGRESS_TEMPLATE: &str = "download:yaydl-progress:%(progress.status)s|%(progress.downloaded_bytes)s|%(progress.total_bytes)s|%(progress.total_bytes_estimate)s|%(progress.speed)s|%(progress.eta)s|%(progress.tmpfilename)s";
const POSTPROCESS_TEMPLATE: &str =
    "postprocess:yaydl-pp:%(progress.status)s|%(progress.postprocessor)s";
const FILE_PRINT: &str = "after_move:yaydl-file:%(filepath)s";
const DEFAULT_OUTPUT_TEMPLATE: &str = "%(title)s [%(id)s].%(ext)s";

/// yt-dlp's `--output-na-placeholder`, printed for fields it does not know.
const NA: &str = "NA";

const UNAVAILABLE_ENTRY_TITLES: [&str; 2] = ["[Private video]", "[Deleted video]"];

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq)]
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

#[derive(Debug, Clone, PartialEq)]
pub enum RunOutcome {
    Exited(CommandOutput),
    /// `cancel` fired and the process tree was killed.
    Cancelled,
}

pub trait Runner: Send + Sync + 'static {
    /// Runs `program` to completion. Every stdout and stderr line is passed to
    /// `on_line` as it arrives, without its line terminator, and both streams
    /// are also captured in the returned output. When `cancel` fires, the whole
    /// process tree is killed and the run returns `RunOutcome::Cancelled`.
    fn run(
        &self,
        program: &Path,
        args: &[String],
        cancel: &CancellationToken,
        on_line: &mut (dyn FnMut(Stream, &str) + Send),
    ) -> impl Future<Output = io::Result<RunOutcome>> + Send;
}

/// Runs yt-dlp through `tokio::process` instead of `tauri_plugin_shell` so the
/// manager stays free of an `AppHandle` and can be driven from plain tests.
pub struct ProcessRunner;

impl ProcessRunner {
    fn command(program: &Path, args: &[String]) -> tokio::process::Command {
        debug!(binary = %program.display(), ?args, "spawning yt-dlp");
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            // Piped Python output uses the ANSI code page on Windows, which
            // mangles titles outside it, and the printed file path has to match
            // the file on disk exactly.
            .env("PYTHONIOENCODING", "utf-8");
        // Its own process group, so cancelling can signal the ffmpeg children too.
        #[cfg(unix)]
        command.process_group(0);
        // Otherwise every yt-dlp call flashes a console window.
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
}

impl Runner for ProcessRunner {
    async fn run(
        &self,
        program: &Path,
        args: &[String],
        cancel: &CancellationToken,
        on_line: &mut (dyn FnMut(Stream, &str) + Send),
    ) -> io::Result<RunOutcome> {
        let mut child = Self::command(program, args).spawn()?;
        let pid = child
            .id()
            .ok_or_else(|| io::Error::other("the spawned yt-dlp has no process id"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("yt-dlp stdout was not captured"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("yt-dlp stderr was not captured"))?;
        let mut stdout = BufReader::new(stdout);
        let mut stderr = BufReader::new(stderr);
        let mut stdout_line = Vec::new();
        let mut stderr_line = Vec::new();
        let mut captured_stdout = String::new();
        let mut captured_stderr = String::new();
        let mut stdout_open = true;
        let mut stderr_open = true;

        // `read_until` keeps partial data in the buffer when another branch
        // wins, so no bytes are lost between iterations.
        while stdout_open || stderr_open {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    kill_tree(&mut child, pid).await?;
                    return Ok(RunOutcome::Cancelled);
                }
                read = stdout.read_until(b'\n', &mut stdout_line), if stdout_open => {
                    if read? == 0 {
                        stdout_open = false;
                    } else {
                        deliver_line(Stream::Stdout, &mut stdout_line, &mut captured_stdout, on_line);
                    }
                }
                read = stderr.read_until(b'\n', &mut stderr_line), if stderr_open => {
                    if read? == 0 {
                        stderr_open = false;
                    } else {
                        deliver_line(Stream::Stderr, &mut stderr_line, &mut captured_stderr, on_line);
                    }
                }
            }
        }

        let status = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                kill_tree(&mut child, pid).await?;
                return Ok(RunOutcome::Cancelled);
            }
            status = child.wait() => status?,
        };
        Ok(RunOutcome::Exited(CommandOutput {
            exit_code: status.code(),
            stdout: captured_stdout,
            stderr: captured_stderr,
        }))
    }
}

fn deliver_line(
    stream: Stream,
    raw: &mut Vec<u8>,
    captured: &mut String,
    on_line: &mut (dyn FnMut(Stream, &str) + Send),
) {
    let mut end = raw.len();
    while end > 0 && matches!(raw[end - 1], b'\n' | b'\r') {
        end -= 1;
    }
    let line = String::from_utf8_lossy(&raw[..end]);
    if let Cow::Owned(_) = line {
        warn!(?stream, %line, "yt-dlp printed a line that is not valid UTF-8, invalid bytes replaced");
    }
    on_line(stream, &line);
    captured.push_str(&line);
    captured.push('\n');
    raw.clear();
}

#[cfg(unix)]
async fn kill_tree(child: &mut tokio::process::Child, pid: u32) -> io::Result<()> {
    let pgid = libc::pid_t::try_from(pid)
        .map_err(|e| io::Error::other(format!("pid {pid} does not fit a pid_t: {e}")))?;
    info!(
        pid,
        "cancelling yt-dlp, sending SIGTERM to its process group"
    );
    signal_group(pgid, libc::SIGTERM)?;
    let deadline = tokio::time::Instant::now() + KILL_GRACE;
    loop {
        // The group outlives yt-dlp while an ffmpeg child is still running.
        if child.try_wait()?.is_some() && !signal_group(pgid, 0)? {
            debug!(pid, "yt-dlp process group exited after SIGTERM");
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    warn!(
        pid,
        grace_secs = KILL_GRACE.as_secs(),
        "yt-dlp process group survived SIGTERM, sending SIGKILL"
    );
    signal_group(pgid, libc::SIGKILL)?;
    child.wait().await?;
    Ok(())
}

/// Returns `false` when the group no longer exists.
#[cfg(unix)]
fn signal_group(pgid: libc::pid_t, signal: libc::c_int) -> io::Result<bool> {
    // SAFETY: killpg only takes two integers and touches no memory of ours.
    if unsafe { libc::killpg(pgid, signal) } == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    Err(io::Error::other(format!(
        "sending signal {signal} to process group {pgid} failed: {error}"
    )))
}

#[cfg(windows)]
async fn kill_tree(child: &mut tokio::process::Child, pid: u32) -> io::Result<()> {
    info!(
        pid,
        "cancelling yt-dlp, killing its process tree with taskkill"
    );
    let output = tokio::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .await?;
    if !output.status.success() {
        warn!(
            pid,
            exit_code = ?output.status.code(),
            stdout = %String::from_utf8_lossy(&output.stdout),
            stderr = %String::from_utf8_lossy(&output.stderr),
            "taskkill failed, killing yt-dlp alone, its ffmpeg children may survive"
        );
        child.start_kill()?;
    }
    child.wait().await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Public engine types
// ---------------------------------------------------------------------------

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

/// Result of a single yt-dlp attempt, before the self-heal policy is applied.
#[derive(Debug)]
enum AttemptError {
    Cancelled,
    Failed(FriendlyError),
    /// yt-dlp ran and failed with an unclassified error, which a newer yt-dlp
    /// than the one of `generation` might fix.
    Healable {
        error: FriendlyError,
        generation: u64,
    },
}

impl AttemptError {
    fn into_failure(self) -> YtDlpFailure {
        match self {
            AttemptError::Cancelled => YtDlpFailure::Cancelled,
            AttemptError::Failed(error) | AttemptError::Healable { error, .. } => {
                YtDlpFailure::Failed(error)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
struct PersistedState {
    last_update_check_unix: u64,
}

pub struct YtDlp<R> {
    runner: R,
    data_dir: PathBuf,
    binary: PathBuf,
    bootstrap: PathBuf,
    ffmpeg_dir: PathBuf,
    /// Runs hold a read guard while yt-dlp executes and updates hold the write
    /// guard, because Windows locks a running exe and `--update-to` could not
    /// replace it. The value is the update generation, bumped by every
    /// successful update, so a run that failed under an older generation knows
    /// a fresher yt-dlp is already in place.
    binary_lock: RwLock<u64>,
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
            binary_lock: RwLock::new(0),
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
        let _guard = self.binary_lock.read().await;
        self.version_unguarded().await
    }

    pub async fn update(&self, channel: YtDlpChannel) -> Result<YtDlpUpdateEvent, YtDlpError> {
        let mut generation = self.binary_lock.write().await;
        self.update_guarded(&mut generation, channel).await
    }

    pub async fn maybe_update_on_startup(
        &self,
        channel: YtDlpChannel,
    ) -> Result<Option<YtDlpUpdateEvent>, YtDlpError> {
        let path = self.state_file();
        match self.read_state()? {
            Some(state) => {
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
            None => {
                info!(
                    state_file = %path.display(),
                    existed = false,
                    due = true,
                    "yt-dlp startup update check"
                );
            }
        }
        let event = self.update(channel).await?;
        info!(?event, "yt-dlp startup update produced an event");
        Ok(Some(event))
    }

    /// Resolves only the video a link points to, even when it names a playlist.
    pub async fn resolve(&self, url: &str, run: &RunOptions) -> Result<Resolved, YtDlpFailure> {
        self.resolve_scoped(url, run, PlaylistScope::VideoOnly)
            .await
    }

    /// Resolves the playlist a video link was opened from, the only way to
    /// list a YouTube Mix, which has no playlist page.
    pub async fn resolve_playlist(
        &self,
        url: &str,
        run: &RunOptions,
    ) -> Result<Resolved, YtDlpFailure> {
        self.resolve_scoped(url, run, PlaylistScope::WholePlaylist)
            .await
    }

    async fn resolve_scoped(
        &self,
        url: &str,
        run: &RunOptions,
        scope: PlaylistScope,
    ) -> Result<Resolved, YtDlpFailure> {
        let args = resolve_args(url, run, scope);
        // Resolving has no cancel handle in the queue contract.
        let never = CancellationToken::new();
        let result = match self.resolve_once(&args, &never).await {
            Err(AttemptError::Healable { error, generation }) => {
                self.self_heal(error, generation, run.channel, &never)
                    .await?;
                let retry = self.resolve_once(&args, &never).await;
                log_retry(url, &retry);
                retry
            }
            other => other,
        };
        result.map_err(AttemptError::into_failure)
    }

    pub async fn download(
        &self,
        request: &DownloadRequest,
        cancel: CancellationToken,
        mut on_event: Box<dyn FnMut(DownloadEvent) + Send>,
    ) -> Result<DownloadOutcome, YtDlpFailure> {
        let args = download_args(request, &self.ffmpeg_dir).map_err(|error| {
            error!(url = %request.url, detail = %error.detail, "building yt-dlp download arguments failed");
            YtDlpFailure::Failed(error)
        })?;
        let mut temp_files = Vec::new();
        let first = self
            .download_once(&args, &cancel, &mut *on_event, &mut temp_files)
            .await;
        let result = match first {
            Err(AttemptError::Healable { error, generation }) => {
                match self
                    .self_heal(error, generation, request.run.channel, &cancel)
                    .await
                {
                    Ok(()) => {
                        let retry = self
                            .download_once(&args, &cancel, &mut *on_event, &mut temp_files)
                            .await;
                        log_retry(&request.url, &retry);
                        retry.map_err(AttemptError::into_failure)
                    }
                    Err(failure) => Err(failure),
                }
            }
            other => other.map_err(AttemptError::into_failure),
        };
        if matches!(result, Err(YtDlpFailure::Cancelled)) {
            info!(url = %request.url, "download cancelled, removing partial files");
            remove_partial_files(&temp_files);
        }
        result
    }

    async fn resolve_once(
        &self,
        args: &[String],
        never: &CancellationToken,
    ) -> Result<Resolved, AttemptError> {
        let mut on_line = |stream: Stream, line: &str| {
            if stream == Stream::Stderr {
                debug!(%line, "yt-dlp stderr");
            }
        };
        let (generation, output) = self.run_guarded(args, never, &mut on_line).await?;
        if !output.succeeded() {
            return Err(command_failure(args, &output, generation));
        }
        parse_resolved(&output.stdout).map_err(|detail| {
            error!(?args, %detail, "yt-dlp metadata could not be read");
            AttemptError::Failed(FriendlyError {
                kind: ErrorKind::Other,
                message: "yt-dlp returned video information yaydl could not read. Retry, and \
                          report it with the details if it keeps happening."
                    .to_string(),
                detail,
            })
        })
    }

    async fn download_once(
        &self,
        args: &[String],
        cancel: &CancellationToken,
        on_event: &mut (dyn FnMut(DownloadEvent) + Send),
        temp_files: &mut Vec<PathBuf>,
    ) -> Result<DownloadOutcome, AttemptError> {
        let mut parser = DownloadParser::default();
        let run = {
            let mut on_line = |_: Stream, line: &str| {
                parser.handle_line(line, Instant::now(), &mut *on_event);
            };
            self.run_guarded(args, cancel, &mut on_line).await
        };
        for path in parser.temp_files.drain(..) {
            if !temp_files.contains(&path) {
                temp_files.push(path);
            }
        }
        let (generation, output) = run?;
        if let Some(progress) = parser.throttle.flush() {
            on_event(DownloadEvent::Progress(progress));
        }
        if !output.succeeded() {
            return Err(command_failure(args, &output, generation));
        }

        let Some(file_path) = parser.file_path else {
            error!(
                ?args,
                stdout = %output.stdout,
                stderr = %output.stderr,
                "yt-dlp exited successfully without printing the output file"
            );
            return Err(AttemptError::Failed(FriendlyError {
                kind: ErrorKind::Other,
                message: "yt-dlp finished but did not say where it saved the file. Check the \
                          download folder, and retry if the file is missing."
                    .to_string(),
                detail: format!(
                    "no \"{FILE_MARKER}\" line in the yt-dlp output\n{}",
                    stderr_tail(&output.stderr)
                ),
            }));
        };
        if let Err(e) = fs::metadata(&file_path) {
            error!(
                path = %file_path.display(),
                error = %e,
                "yt-dlp reported an output file that cannot be found"
            );
            return Err(AttemptError::Failed(FriendlyError {
                kind: ErrorKind::Other,
                message: "yt-dlp reported a finished file that is not on disk. Check the \
                          download folder and retry."
                    .to_string(),
                detail: format!("{}: {e}", file_path.display()),
            }));
        }
        info!(path = %file_path.display(), "download finished");
        Ok(DownloadOutcome { file_path })
    }

    /// Runs yt-dlp under a read guard, which is released as soon as the
    /// process exits.
    async fn run_guarded(
        &self,
        args: &[String],
        cancel: &CancellationToken,
        on_line: &mut (dyn FnMut(Stream, &str) + Send),
    ) -> Result<(u64, CommandOutput), AttemptError> {
        let guard = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                info!(?args, "yt-dlp run cancelled before it started");
                return Err(AttemptError::Cancelled);
            }
            guard = self.binary_lock.read() => guard,
        };
        let generation = *guard;
        let started = Instant::now();
        let outcome = self.runner.run(&self.binary, args, cancel, on_line).await;
        drop(guard);
        let elapsed_ms = started.elapsed().as_millis();
        match outcome {
            Ok(RunOutcome::Exited(output)) => {
                info!(
                    exit_code = ?output.exit_code,
                    elapsed_ms,
                    stdout_bytes = output.stdout.len(),
                    stderr_bytes = output.stderr.len(),
                    "yt-dlp exited"
                );
                Ok((generation, output))
            }
            Ok(RunOutcome::Cancelled) => {
                info!(?args, elapsed_ms, "yt-dlp run cancelled");
                Err(AttemptError::Cancelled)
            }
            Err(e) => {
                error!(binary = %self.binary.display(), ?args, error = %e, "running yt-dlp failed");
                Err(AttemptError::Failed(FriendlyError {
                    kind: ErrorKind::Other,
                    message: "yt-dlp could not be started. Restart yaydl, and reinstall it if \
                              this keeps happening."
                        .to_string(),
                    detail: format!("running {} failed: {e}", self.binary.display()),
                }))
            }
        }
    }

    /// Returns `Ok` when the failed run should be retried.
    async fn self_heal(
        &self,
        error: FriendlyError,
        failed_generation: u64,
        channel: YtDlpChannel,
        cancel: &CancellationToken,
    ) -> Result<(), YtDlpFailure> {
        warn!(
            %channel,
            failed_generation,
            detail = %error.detail,
            "yt-dlp failed with an unclassified error, trying a self-heal update"
        );
        let mut generation = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(YtDlpFailure::Cancelled),
            guard = self.binary_lock.write() => guard,
        };
        if *generation != failed_generation {
            info!(
                failed_generation,
                current_generation = *generation,
                "yt-dlp was updated while this run was failing, retrying without another update"
            );
            return Ok(());
        }
        match self.last_update_check_age() {
            Ok(Some(age)) if age < SELF_HEAL_MIN_INTERVAL => {
                info!(
                    age_secs = age.as_secs(),
                    min_interval_secs = SELF_HEAL_MIN_INTERVAL.as_secs(),
                    "self-heal skipped, yt-dlp was checked for updates recently"
                );
                return Err(YtDlpFailure::Failed(error));
            }
            Ok(_) => {}
            Err(e) => {
                error!(error = %e, "self-heal skipped, the update-check state is unreadable");
                return Err(YtDlpFailure::Failed(FriendlyError {
                    detail: format!("{}\n\nSelf-heal skipped: {e}", error.detail),
                    ..error
                }));
            }
        }
        // Not cancellable, an update killed halfway could leave a broken binary.
        match self.update_guarded(&mut generation, channel).await {
            Ok(event) => {
                info!(?event, "self-heal update done, retrying once");
                Ok(())
            }
            Err(e) => {
                warn!(error = %e, %channel, "self-heal update failed");
                Err(YtDlpFailure::Failed(FriendlyError {
                    detail: format!("{}\n\nUpdating yt-dlp failed: {e}", error.detail),
                    ..error
                }))
            }
        }
    }

    /// `generation` is borrowed from the write guard, which proves no run is
    /// using the binary.
    async fn update_guarded(
        &self,
        generation: &mut u64,
        channel: YtDlpChannel,
    ) -> Result<YtDlpUpdateEvent, YtDlpError> {
        let before = self.version_unguarded().await?;
        let args = vec![String::from("--update-to"), channel.as_str().to_string()];
        let output = self.run_unguarded(&args).await?;
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
        let after = self.version_unguarded().await?;
        *generation += 1;
        self.record_update_check()?;
        info!(
            %channel,
            %before,
            %after,
            generation = *generation,
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

    /// Callers must hold a guard of `binary_lock`.
    async fn version_unguarded(&self) -> Result<String, YtDlpError> {
        let args = vec![String::from("--version")];
        let output = self.run_unguarded(&args).await?;
        Ok(check(output, &args)?.stdout.trim().to_string())
    }

    /// Callers must hold a guard of `binary_lock`.
    async fn run_unguarded(&self, args: &[String]) -> Result<CommandOutput, YtDlpError> {
        let never = CancellationToken::new();
        let outcome = self
            .runner
            .run(&self.binary, args, &never, &mut |_: Stream, _: &str| {})
            .await
            .map_err(|e| {
                YtDlpError::Io(format!(
                    "running {} {} failed: {e}",
                    self.binary.display(),
                    args.join(" ")
                ))
            })?;
        match outcome {
            RunOutcome::Exited(output) => Ok(output),
            RunOutcome::Cancelled => Err(YtDlpError::Io(format!(
                "{} {} reported a cancellation nobody requested",
                self.binary.display(),
                args.join(" ")
            ))),
        }
    }

    fn read_state(&self) -> Result<Option<PersistedState>, YtDlpError> {
        let path = self.state_file();
        match fs::read_to_string(&path) {
            Ok(raw) => toml::from_str(&raw)
                .map(Some)
                .map_err(|e| YtDlpError::InvalidState(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(YtDlpError::Io(format!(
                "reading {} failed: {e}",
                path.display()
            ))),
        }
    }

    /// `None` when no past check is recorded. A timestamp in the future is not
    /// a record of a past check either.
    fn last_update_check_age(&self) -> Result<Option<Duration>, YtDlpError> {
        let Some(state) = self.read_state()? else {
            return Ok(None);
        };
        Ok(now_unix()?
            .checked_sub(state.last_update_check_unix)
            .map(Duration::from_secs))
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

fn log_retry<T>(url: &str, retry: &Result<T, AttemptError>) {
    match retry {
        Ok(_) => info!(%url, "yt-dlp retry after self-heal succeeded"),
        Err(e) => warn!(%url, error = ?e, "yt-dlp retry after self-heal failed"),
    }
}

/// Logs the full stderr, which the friendly error trims to its tail.
fn command_failure(args: &[String], output: &CommandOutput, generation: u64) -> AttemptError {
    let error = classify(&output.stderr, output.exit_code);
    warn!(
        ?args,
        exit_code = ?output.exit_code,
        kind = ?error.kind,
        stderr = %output.stderr,
        "yt-dlp exited non-zero"
    );
    if error.kind == ErrorKind::Other {
        AttemptError::Healable { error, generation }
    } else {
        AttemptError::Failed(error)
    }
}

/// Turns a non-zero exit of `--version` into `CommandFailed` and logs the full
/// stderr, which the trimmed error message deliberately drops.
fn check(output: CommandOutput, args: &[String]) -> Result<CommandOutput, YtDlpError> {
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

// ---------------------------------------------------------------------------
// Arguments
// ---------------------------------------------------------------------------

/// What yt-dlp resolves for a link that names both a video and a playlist.
#[derive(Clone, Copy, Debug)]
enum PlaylistScope {
    VideoOnly,
    WholePlaylist,
}

/// YouTube generates Mixes endlessly per viewer. yt-dlp returned 1100 to 3500
/// entries (about 550 unique) for one Mix, so only the start is taken, about
/// what YouTube's own Mix panel shows.
pub const MIX_ENTRY_LIMIT: u32 = 50;

/// yt-dlp encodes printed lines with the pipe's encoding and silently drops
/// what does not fit (`s.encode(enc, 'ignore')`). On Windows that is the ANSI
/// code page, which has no fullwidth quote, the character yt-dlp puts into file
/// names in place of `"`. The printed path then misses characters and does not
/// exist. `PYTHONIOENCODING` does not reach the bundled yt-dlp.exe there.
const FORCE_UTF8: [&str; 2] = ["--encoding", "utf-8"];

fn resolve_args(url: &str, run: &RunOptions, scope: PlaylistScope) -> Vec<String> {
    let playlist = match scope {
        PlaylistScope::VideoOnly => "--no-playlist",
        PlaylistScope::WholePlaylist => "--yes-playlist",
    };
    let mut args = strings(&FORCE_UTF8);
    args.extend(strings(&["-J", "--flat-playlist", playlist]));
    let is_mix = yaydl_shared::playlist_in_video_link(url).is_some_and(|link| link.is_mix);
    if matches!(scope, PlaylistScope::WholePlaylist) && is_mix {
        args.push("--playlist-items".to_string());
        args.push(format!("1:{MIX_ENTRY_LIMIT}"));
    }
    push_cookies(&mut args, run);
    args.push("--".to_string());
    args.push(url.to_string());
    args
}

fn download_args(
    request: &DownloadRequest,
    ffmpeg_dir: &Path,
) -> Result<Vec<String>, FriendlyError> {
    let template = match &request.file_stem {
        Some(stem) => format!("{}.%(ext)s", stem.replace('%', "%%")),
        None => DEFAULT_OUTPUT_TEMPLATE.to_string(),
    };
    let mut args = strings(&FORCE_UTF8);
    args.extend(strings(&[
        "--newline",
        "--no-playlist",
        "--progress",
        "--progress-template",
        PROGRESS_TEMPLATE,
        "--progress-template",
        POSTPROCESS_TEMPLATE,
        "--print",
        FILE_PRINT,
        "--ffmpeg-location",
    ]));
    args.push(utf8_path(ffmpeg_dir, "ffmpeg folder")?);
    args.push("-P".to_string());
    args.push(utf8_path(&request.output_dir, "download folder")?);
    args.push("-o".to_string());
    args.push(template);
    match request.format {
        OutputFormat::Audio { codec } => {
            args.extend(strings(&[
                "-x",
                "--audio-format",
                codec.as_str(),
                "--audio-quality",
                "0",
            ]));
        }
        OutputFormat::Video { quality } => {
            let sort = match quality.max_height() {
                Some(height) => format!("res:{height},ext:mp4:m4a"),
                None => "ext:mp4:m4a".to_string(),
            };
            args.push("-S".to_string());
            args.push(sort);
            args.extend(strings(&["--merge-output-format", "mp4"]));
        }
    }
    if request.embed_metadata {
        args.extend(strings(&["--embed-metadata", "--embed-thumbnail"]));
    }
    push_cookies(&mut args, &request.run);
    args.push("--".to_string());
    args.push(request.url.clone());
    Ok(args)
}

fn push_cookies(args: &mut Vec<String>, run: &RunOptions) {
    if let Some(browser) = run.cookies_from_browser {
        args.push("--cookies-from-browser".to_string());
        args.push(browser.as_str().to_string());
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

/// yt-dlp arguments are strings, and a lossy conversion would point it at a
/// different folder.
fn utf8_path(path: &Path, what: &str) -> Result<String, FriendlyError> {
    path.to_str()
        .map(str::to_string)
        .ok_or_else(|| FriendlyError {
            kind: ErrorKind::Other,
            message: format!(
                "The {what} path contains characters yt-dlp cannot handle. Pick another folder."
            ),
            detail: format!("{what} {} is not valid Unicode", path.display()),
        })
}

// ---------------------------------------------------------------------------
// Download output parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct ProgressLine {
    status: String,
    progress: Progress,
    tmpfilename: Option<PathBuf>,
}

/// Parses what follows `yaydl-progress:` in a line produced by `PROGRESS_TEMPLATE`.
fn parse_progress_line(rest: &str) -> Result<ProgressLine, String> {
    let mut fields = rest.splitn(7, '|');
    let status = fields
        .next()
        .filter(|s| !s.is_empty())
        .ok_or("missing status")?
        .to_string();
    let downloaded = parse_number(fields.next(), "downloaded_bytes")?;
    let total = parse_number(fields.next(), "total_bytes")?;
    let estimate = parse_number(fields.next(), "total_bytes_estimate")?;
    let speed = parse_number(fields.next(), "speed")?;
    let eta = parse_number(fields.next(), "eta")?;
    let tmpfilename = match fields.next().ok_or("missing tmpfilename")? {
        NA => None,
        path => Some(PathBuf::from(path)),
    };

    let total = total.or(estimate);
    let percent = match (downloaded, total) {
        (Some(done), Some(total)) if total > 0.0 => Some((done / total * 100.0).min(100.0) as f32),
        _ => None,
    };
    Ok(ProgressLine {
        status,
        progress: Progress {
            percent,
            downloaded_bytes: downloaded.map(|v| v.round() as u64),
            total_bytes: total.map(|v| v.round() as u64),
            speed_bytes_per_sec: speed,
            eta_secs: eta.map(|v| v.round() as u64),
        },
        tmpfilename,
    })
}

/// yt-dlp prints `total_bytes_estimate` and `speed` as floats, so every
/// number is read as one.
fn parse_number(field: Option<&str>, name: &str) -> Result<Option<f64>, String> {
    let raw = field.ok_or_else(|| format!("missing {name}"))?;
    if raw == NA {
        return Ok(None);
    }
    let value: f64 = raw
        .parse()
        .map_err(|e| format!("{name} {raw:?} is not a number: {e}"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!("{name} {raw:?} is not a non-negative number"));
    }
    Ok(Some(value))
}

#[derive(Debug, Default)]
struct ProgressThrottle {
    last_emit: Option<Instant>,
    pending: Option<Progress>,
}

impl ProgressThrottle {
    fn offer(&mut self, progress: Progress, now: Instant, force: bool) -> Option<Progress> {
        let due = force
            || self
                .last_emit
                .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if due {
            self.last_emit = Some(now);
            self.pending = None;
            Some(progress)
        } else {
            self.pending = Some(progress);
            None
        }
    }

    fn flush(&mut self) -> Option<Progress> {
        self.pending.take()
    }
}

/// `--print` implies `--quiet`, which moves some of yt-dlp's output to stderr,
/// so the markers are parsed from both streams alike.
#[derive(Debug, Default)]
struct DownloadParser {
    throttle: ProgressThrottle,
    processing: bool,
    file_path: Option<PathBuf>,
    /// Every distinct `tmpfilename`, since a merged video downloads one per format.
    temp_files: Vec<PathBuf>,
}

impl DownloadParser {
    fn handle_line(&mut self, line: &str, now: Instant, emit: &mut dyn FnMut(DownloadEvent)) {
        if let Some(rest) = line.strip_prefix(PROGRESS_MARKER) {
            match parse_progress_line(rest) {
                Ok(parsed) => {
                    if let Some(tmp) = parsed.tmpfilename {
                        if !self.temp_files.contains(&tmp) {
                            debug!(path = %tmp.display(), "yt-dlp writing a temporary file");
                            self.temp_files.push(tmp);
                        }
                    }
                    let force = parsed.status == "finished";
                    if let Some(progress) = self.throttle.offer(parsed.progress, now, force) {
                        emit(DownloadEvent::Progress(progress));
                    }
                }
                Err(e) => warn!(%line, error = %e, "unparseable yt-dlp progress line"),
            }
        } else if let Some(rest) = line.strip_prefix(POSTPROCESS_MARKER) {
            debug!(step = %rest, "yt-dlp post-processing");
            if !self.processing {
                self.processing = true;
                if let Some(progress) = self.throttle.flush() {
                    emit(DownloadEvent::Progress(progress));
                }
                emit(DownloadEvent::Processing);
            }
        } else if let Some(rest) = line.strip_prefix(FILE_MARKER) {
            let path = PathBuf::from(rest);
            if let Some(previous) = &self.file_path {
                warn!(
                    previous = %previous.display(),
                    new = %path.display(),
                    "yt-dlp reported more than one output file, keeping the last"
                );
            }
            self.file_path = Some(path);
        } else if !line.trim().is_empty() {
            debug!(%line, "yt-dlp output");
        }
    }
}

/// Removes what a cancelled run leaves behind for each temporary file
/// `X.part`: the file itself, the finished format file `X` of a merged video,
/// the fragment state `X.ytdl` and fragments `X.part-Frag*`.
fn remove_partial_files(temp_files: &[PathBuf]) {
    for temp in temp_files {
        let mut candidates = vec![temp.clone()];
        if let Some(base) = temp.to_str().and_then(|s| s.strip_suffix(".part")) {
            candidates.push(PathBuf::from(base));
            candidates.push(PathBuf::from(format!("{base}.ytdl")));
            candidates.extend(fragment_files(temp));
        }
        for path in candidates {
            match fs::remove_file(&path) {
                Ok(()) => info!(path = %path.display(), "removed partial download file"),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => error!(
                    path = %path.display(),
                    error = %e,
                    "removing a partial download file failed, it stays on disk"
                ),
            }
        }
    }
}

fn fragment_files(temp: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (temp.parent(), temp.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let prefix = format!("{name}-Frag");
    match fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|entry| match entry {
                Ok(entry) => Some(entry.path()),
                Err(e) => {
                    warn!(dir = %dir.display(), error = %e, "reading a directory entry failed");
                    None
                }
            })
            .filter(|path| {
                path.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(e) => {
            warn!(dir = %dir.display(), error = %e, "listing fragment files failed");
            Vec::new()
        }
    }
}

// ---------------------------------------------------------------------------
// Metadata parsing
// ---------------------------------------------------------------------------

fn parse_resolved(stdout: &str) -> Result<Resolved, String> {
    let root: Value = serde_json::from_str(stdout.trim())
        .map_err(|e| format!("yt-dlp printed invalid JSON: {e}"))?;
    let object = root
        .as_object()
        .ok_or("yt-dlp printed JSON that is not an object")?;
    match optional_str(object, "_type")?.as_deref() {
        None | Some("video") => parse_single(object).map(Resolved::Single),
        Some("playlist") => parse_playlist(object),
        Some(other) => Err(format!("unsupported yt-dlp result type {other:?}")),
    }
}

fn parse_single(object: &Map<String, Value>) -> Result<VideoMetadata, String> {
    Ok(VideoMetadata {
        extractor: required_str(object, "extractor_key")?,
        video_id: required_str(object, "id")?,
        title: required_str(object, "title")?,
        uploader: uploader(object)?,
        duration_secs: optional_f64(object, "duration")?,
        thumbnail: optional_str(object, "thumbnail")?,
        webpage_url: required_str(object, "webpage_url")?,
    })
}

fn parse_playlist(object: &Map<String, Value>) -> Result<Resolved, String> {
    let title = optional_str(object, "title")?;
    let raw_entries = object
        .get("entries")
        .ok_or("the playlist has no `entries`")?
        .as_array()
        .ok_or("the playlist `entries` is not an array")?;
    let mut entries = Vec::with_capacity(raw_entries.len());
    let mut unavailable = 0u32;
    for (index, entry) in raw_entries.iter().enumerate() {
        match parse_entry(entry) {
            Ok(metadata) => entries.push(metadata),
            Err(EntrySkip::Unavailable(reason)) => {
                debug!(index, %reason, "skipping unavailable playlist entry");
                unavailable += 1;
            }
            Err(EntrySkip::Malformed(reason)) => {
                warn!(index, %reason, %entry, "skipping malformed playlist entry");
                unavailable += 1;
            }
        }
    }
    Ok(Resolved::Playlist {
        title,
        entries,
        unavailable,
    })
}

enum EntrySkip {
    Unavailable(String),
    Malformed(String),
}

fn parse_entry(entry: &Value) -> Result<VideoMetadata, EntrySkip> {
    let object = entry
        .as_object()
        .ok_or_else(|| EntrySkip::Unavailable(format!("entry is {entry}, not an object")))?;
    let entry_type = optional_str(object, "_type").map_err(EntrySkip::Malformed)?;
    if entry_type.as_deref() == Some("playlist") {
        return Err(EntrySkip::Unavailable("nested playlist".to_string()));
    }
    let title = optional_str(object, "title").map_err(EntrySkip::Malformed)?;
    if let Some(title) = title
        .as_deref()
        .filter(|t| UNAVAILABLE_ENTRY_TITLES.contains(t))
    {
        return Err(EntrySkip::Unavailable(format!("titled {title}")));
    }
    let Some(video_id) = optional_str(object, "id").map_err(EntrySkip::Malformed)? else {
        return Err(EntrySkip::Unavailable("no id".to_string()));
    };
    // Flat entries are url references, where `url` is the page. Extractors
    // without flat support return full results, where `url` is the media stream.
    let (extractor_field, url_field) = match entry_type.as_deref() {
        Some("url" | "url_transparent") => ("ie_key", "url"),
        _ => ("extractor_key", "webpage_url"),
    };
    let Some(webpage_url) = optional_str(object, url_field).map_err(EntrySkip::Malformed)? else {
        return Err(EntrySkip::Unavailable(format!("no {url_field}")));
    };
    let extractor = optional_str(object, extractor_field)
        .map_err(EntrySkip::Malformed)?
        .ok_or_else(|| EntrySkip::Malformed(format!("no {extractor_field}")))?;
    let title = title.ok_or_else(|| EntrySkip::Malformed("no title".to_string()))?;
    let thumbnail = match object
        .get("thumbnails")
        .and_then(Value::as_array)
        .and_then(|t| t.last())
    {
        Some(last) => last
            .as_object()
            .map(|t| optional_str(t, "url"))
            .transpose()
            .map_err(EntrySkip::Malformed)?
            .flatten(),
        None => optional_str(object, "thumbnail").map_err(EntrySkip::Malformed)?,
    };
    Ok(VideoMetadata {
        extractor,
        video_id,
        title,
        uploader: uploader(object).map_err(EntrySkip::Malformed)?,
        duration_secs: optional_f64(object, "duration").map_err(EntrySkip::Malformed)?,
        thumbnail,
        webpage_url,
    })
}

/// `channel` is the same person as `uploader` on most sites, and some
/// extractors only fill one of them.
fn uploader(object: &Map<String, Value>) -> Result<Option<String>, String> {
    match optional_str(object, "uploader")? {
        Some(uploader) => Ok(Some(uploader)),
        None => optional_str(object, "channel"),
    }
}

fn required_str(object: &Map<String, Value>, field: &str) -> Result<String, String> {
    optional_str(object, field)?.ok_or_else(|| format!("yt-dlp metadata has no `{field}`"))
}

fn optional_str(object: &Map<String, Value>, field: &str) -> Result<Option<String>, String> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(other) => Err(format!(
            "yt-dlp metadata `{field}` is {other}, not a string"
        )),
    }
}

fn optional_f64(object: &Map<String, Value>, field: &str) -> Result<Option<f64>, String> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(value)) => value
            .as_f64()
            .map(Some)
            .ok_or_else(|| format!("yt-dlp metadata `{field}` {value} does not fit an f64")),
        Some(other) => Err(format!(
            "yt-dlp metadata `{field}` is {other}, not a number"
        )),
    }
}

// ---------------------------------------------------------------------------
// Error classification
// ---------------------------------------------------------------------------

/// First match wins. Age, private and members-only errors also tell the user
/// to pass cookies, so they come before the generic cookie hint of the bot
/// check, and the broad network patterns come last.
const ERROR_PATTERNS: &[(ErrorKind, &[&str])] = &[
    (
        ErrorKind::AgeRestricted,
        &[
            "Sign in to confirm your age",
            "age-restricted",
            "inappropriate for some users",
        ],
    ),
    (
        ErrorKind::Private,
        &["Private video", "members-only", "Join this channel"],
    ),
    (
        ErrorKind::BotCheck,
        &[
            "confirm you're not a bot",
            "--cookies-from-browser or --cookies for the authentication",
        ],
    ),
    (
        ErrorKind::GeoBlocked,
        &[
            "not available in your country",
            "made this video available in your country",
            "geo restrict",
            "blocked it in your country",
        ],
    ),
    (
        ErrorKind::NotYetLive,
        &["This live event will begin", "Premieres in", "is not live"],
    ),
    (
        ErrorKind::Unavailable,
        &[
            "Video unavailable",
            "This video has been removed",
            "HTTP Error 404",
            "does not exist",
        ],
    ),
    (
        ErrorKind::RateLimited,
        &["HTTP Error 429", "rate-limit", "Too Many Requests"],
    ),
    (ErrorKind::UnsupportedUrl, &["Unsupported URL"]),
    (ErrorKind::DiskFull, &["No space left on device"]),
    (
        ErrorKind::Network,
        &[
            "Unable to download webpage",
            "getaddrinfo",
            "Name or service not known",
            "Connection refused",
            "timed out",
            "Network is unreachable",
            "Temporary failure in name resolution",
        ],
    ),
];

fn friendly_message(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::AgeRestricted => {
            "This video is age-restricted. Choose a browser you are signed in with under \
             cookies in the settings, then retry."
        }
        ErrorKind::BotCheck => {
            "YouTube wants to confirm you are not a bot. Choose a browser you are signed in \
             with under cookies in the settings, then retry."
        }
        ErrorKind::Private => {
            "This video is private or for channel members only. If your account has access, \
             choose a browser you are signed in with under cookies in the settings, then retry."
        }
        ErrorKind::Unavailable => {
            "This video is unavailable. It may have been removed, or the link is wrong."
        }
        ErrorKind::GeoBlocked => {
            "This video is not available in your country. It can only be downloaded from a \
             network in a country where it is available."
        }
        ErrorKind::RateLimited => {
            "The site is limiting how many requests yaydl makes. Wait a while, or lower the \
             number of parallel downloads, then retry."
        }
        ErrorKind::Network => {
            "The site could not be reached. Check your internet connection, then retry."
        }
        ErrorKind::UnsupportedUrl => {
            "yt-dlp does not support this URL. Check that it links to a video or playlist page."
        }
        ErrorKind::NotYetLive => {
            "This live stream or premiere has not started yet. Retry once it is live or over."
        }
        ErrorKind::DiskFull => {
            "The disk is full. Free up space or choose another download folder, then retry."
        }
        ErrorKind::Interrupted => {
            "The download was interrupted because yaydl closed. Retry to start it again."
        }
        ErrorKind::OutputFolderMissing => {
            "The output folder does not exist. Choose another folder in Settings."
        }
        ErrorKind::EmptyPlaylist => "This playlist has no downloadable videos.",
        ErrorKind::Other => {
            "yt-dlp failed with an unexpected error. Retry later, and check the details if it \
             keeps happening."
        }
    }
}

/// Classifies a failed yt-dlp run from its stderr. Only `ERROR:` lines are
/// matched when there are any, because retries print warnings (for example
/// timeouts) that say nothing about why the run finally failed.
pub fn classify(stderr: &str, exit_code: Option<i32>) -> FriendlyError {
    let error_lines: Vec<&str> = stderr
        .lines()
        .filter(|line| line.trim_start().starts_with("ERROR:"))
        .collect();
    let haystack = if error_lines.is_empty() {
        stderr.to_lowercase()
    } else {
        error_lines.join("\n").to_lowercase()
    }
    .replace('\u{2019}', "'");
    let kind = ERROR_PATTERNS
        .iter()
        .find(|(_, needles)| {
            needles
                .iter()
                .any(|needle| haystack.contains(&needle.to_lowercase()))
        })
        .map_or(ErrorKind::Other, |(kind, _)| *kind);

    let tail = stderr_tail(stderr);
    let detail = if tail.is_empty() {
        match exit_code {
            Some(code) => format!("yt-dlp exited with code {code} and printed nothing on stderr"),
            None => "yt-dlp was killed by a signal and printed nothing on stderr".to_string(),
        }
    } else {
        tail
    };
    FriendlyError {
        kind,
        message: friendly_message(kind).to_string(),
        detail,
    }
}

/// Our own marker lines are left out, they are not diagnostics.
fn stderr_tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("yaydl-"))
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

// AGENT CODE: claude-opus-5
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::sync::Barrier;
    use yaydl_shared::{AudioCodec, VideoQuality};

    enum FakeStep {
        Exit {
            code: Option<i32>,
            lines: Vec<(Stream, String)>,
        },
        /// Prints `lines`, then runs until cancelled.
        Hang { lines: Vec<(Stream, String)> },
        /// Waits for the other gated runs before continuing with the inner step.
        Gated(Arc<Barrier>, Box<FakeStep>),
    }

    type Handler = Box<dyn Fn(&[String]) -> FakeStep + Send + Sync>;
    type Calls = Arc<Mutex<Vec<Vec<String>>>>;

    struct FakeRunner {
        calls: Calls,
        handler: Handler,
    }

    impl Runner for FakeRunner {
        async fn run(
            &self,
            _program: &Path,
            args: &[String],
            cancel: &CancellationToken,
            on_line: &mut (dyn FnMut(Stream, &str) + Send),
        ) -> io::Result<RunOutcome> {
            self.calls.lock().unwrap().push(args.to_vec());
            let mut step = (self.handler)(args);
            loop {
                match step {
                    FakeStep::Gated(barrier, inner) => {
                        barrier.wait().await;
                        step = *inner;
                    }
                    FakeStep::Exit { code, lines } => {
                        let mut output = CommandOutput {
                            exit_code: code,
                            stdout: String::new(),
                            stderr: String::new(),
                        };
                        for (stream, line) in lines {
                            on_line(stream, &line);
                            let target = match stream {
                                Stream::Stdout => &mut output.stdout,
                                Stream::Stderr => &mut output.stderr,
                            };
                            target.push_str(&line);
                            target.push('\n');
                        }
                        return Ok(RunOutcome::Exited(output));
                    }
                    FakeStep::Hang { lines } => {
                        for (stream, line) in lines {
                            on_line(stream, &line);
                        }
                        cancel.cancelled().await;
                        return Ok(RunOutcome::Cancelled);
                    }
                }
            }
        }
    }

    fn out(line: &str) -> (Stream, String) {
        (Stream::Stdout, line.to_string())
    }

    fn err(line: &str) -> (Stream, String) {
        (Stream::Stderr, line.to_string())
    }

    fn ok(lines: Vec<(Stream, String)>) -> FakeStep {
        FakeStep::Exit {
            code: Some(0),
            lines,
        }
    }

    fn fail(stderr: &str) -> FakeStep {
        FakeStep::Exit {
            code: Some(1),
            lines: vec![err(stderr)],
        }
    }

    fn manager_with(handler: Handler) -> (YtDlp<FakeRunner>, Calls, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let runner = FakeRunner {
            calls: Arc::clone(&calls),
            handler,
        };
        let manager = YtDlp::new(
            runner,
            dir.path().to_path_buf(),
            dir.path().join("bundled-yt-dlp"),
            dir.path().join("ffmpeg"),
        );
        (manager, calls, dir)
    }

    fn manager(steps: Vec<FakeStep>) -> (YtDlp<FakeRunner>, Calls, tempfile::TempDir) {
        let queue = Mutex::new(VecDeque::from(steps));
        manager_with(Box::new(move |args| {
            queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| panic!("unscripted yt-dlp invocation: {args:?}"))
        }))
    }

    fn calls_of(calls: &Calls) -> Vec<Vec<String>> {
        calls.lock().unwrap().clone()
    }

    fn update_count(calls: &Calls) -> usize {
        calls_of(calls)
            .iter()
            .filter(|args| args.first().map(String::as_str) == Some("--update-to"))
            .count()
    }

    fn version_args() -> Vec<String> {
        vec!["--version".to_string()]
    }

    fn update_args(channel: &str) -> Vec<String> {
        vec!["--update-to".to_string(), channel.to_string()]
    }

    fn run_options() -> RunOptions {
        RunOptions {
            channel: YtDlpChannel::Nightly,
            cookies_from_browser: None,
        }
    }

    fn request(output_dir: &Path, format: OutputFormat) -> DownloadRequest {
        DownloadRequest {
            url: "https://www.youtube.com/watch?v=jNQXAC9IVRw".to_string(),
            output_dir: output_dir.to_path_buf(),
            format,
            file_stem: None,
            embed_metadata: false,
            run: run_options(),
        }
    }

    fn mp3() -> OutputFormat {
        OutputFormat::Audio {
            codec: AudioCodec::Mp3,
        }
    }

    type Events = Arc<Mutex<Vec<DownloadEvent>>>;

    fn recorder() -> (Events, Box<dyn FnMut(DownloadEvent) + Send>) {
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        (
            events,
            Box::new(move |event| sink.lock().unwrap().push(event)),
        )
    }

    fn write_update_check(manager: &YtDlp<FakeRunner>, age_secs: u64) {
        let at = now_unix().unwrap() - age_secs;
        fs::write(
            manager.state_file(),
            format!("last_update_check_unix = {at}\n"),
        )
        .unwrap();
    }

    const STALE_EXTRACTOR: &str = "ERROR: [youtube] jNQXAC9IVRw: Unable to extract player response";
    const VIDEO_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

    // --- arguments

    fn after_template(args: &[String]) -> Vec<&str> {
        let o = args.iter().position(|a| a == "-o").expect("-o in args");
        args[o + 2..].iter().map(String::as_str).collect()
    }

    #[test]
    fn audio_download_args() {
        let request = request(Path::new("/music"), mp3());
        let args = download_args(&request, Path::new("/opt/yaydl")).unwrap();

        let expected = strings(&[
            "--encoding",
            "utf-8",
            "--newline",
            "--no-playlist",
            "--progress",
            "--progress-template",
            PROGRESS_TEMPLATE,
            "--progress-template",
            POSTPROCESS_TEMPLATE,
            "--print",
            FILE_PRINT,
            "--ffmpeg-location",
            "/opt/yaydl",
            "-P",
            "/music",
            "-o",
            "%(title)s [%(id)s].%(ext)s",
            "-x",
            "--audio-format",
            "mp3",
            "--audio-quality",
            "0",
            "--",
            VIDEO_URL,
        ]);
        assert_eq!(args, expected);
    }

    #[test]
    fn video_download_args_for_best_and_1080p() {
        let best = request(
            Path::new("/v"),
            OutputFormat::Video {
                quality: VideoQuality::Best,
            },
        );
        let args = download_args(&best, Path::new("/ff")).unwrap();
        assert_eq!(
            after_template(&args),
            [
                "-S",
                "ext:mp4:m4a",
                "--merge-output-format",
                "mp4",
                "--",
                VIDEO_URL
            ]
        );

        let p1080 = request(
            Path::new("/v"),
            OutputFormat::Video {
                quality: VideoQuality::P1080,
            },
        );
        let args = download_args(&p1080, Path::new("/ff")).unwrap();
        assert_eq!(
            after_template(&args),
            [
                "-S",
                "res:1080,ext:mp4:m4a",
                "--merge-output-format",
                "mp4",
                "--",
                VIDEO_URL
            ]
        );
    }

    #[test]
    fn custom_stem_escapes_percent_signs() {
        let mut request = request(Path::new("/music"), mp3());
        request.file_stem = Some("100% %(title)s".to_string());
        let args = download_args(&request, Path::new("/ff")).unwrap();

        let o = args.iter().position(|a| a == "-o").unwrap();
        assert_eq!(args[o + 1], "100%% %%(title)s.%(ext)s");
    }

    #[test]
    fn embed_and_cookies_come_before_the_separator_and_the_url_is_last() {
        let mut request = request(Path::new("/music"), mp3());
        request.embed_metadata = true;
        request.run.cookies_from_browser = Some(Browser::Firefox);
        request.url = "-looks-like-a-flag".to_string();
        let args = download_args(&request, Path::new("/ff")).unwrap();

        assert_eq!(
            after_template(&args)[5..],
            [
                "--embed-metadata",
                "--embed-thumbnail",
                "--cookies-from-browser",
                "firefox",
                "--",
                "-looks-like-a-flag",
            ]
        );
    }

    #[test]
    fn resolve_args_with_and_without_cookies() {
        let url = "https://www.youtube.com/watch?v=x&list=y";
        assert_eq!(
            resolve_args(url, &run_options(), PlaylistScope::VideoOnly),
            strings(&[
                "--encoding",
                "utf-8",
                "-J",
                "--flat-playlist",
                "--no-playlist",
                "--",
                url
            ])
        );
        let with_cookies = RunOptions {
            cookies_from_browser: Some(Browser::Chrome),
            ..run_options()
        };
        assert_eq!(
            resolve_args(url, &with_cookies, PlaylistScope::VideoOnly),
            strings(&[
                "--encoding",
                "utf-8",
                "-J",
                "--flat-playlist",
                "--no-playlist",
                "--cookies-from-browser",
                "chrome",
                "--",
                url
            ])
        );
    }

    #[test]
    fn only_a_whole_mix_is_limited() {
        let mix = "https://www.youtube.com/watch?v=onVL3uQD8UY&list=RDyrc0yef1xoU";
        let limit = format!("1:{MIX_ENTRY_LIMIT}");
        assert_eq!(
            resolve_args(mix, &run_options(), PlaylistScope::WholePlaylist),
            strings(&[
                "--encoding",
                "utf-8",
                "-J",
                "--flat-playlist",
                "--yes-playlist",
                "--playlist-items",
                &limit,
                "--",
                mix
            ])
        );
        assert_eq!(
            resolve_args(mix, &run_options(), PlaylistScope::VideoOnly),
            strings(&[
                "--encoding",
                "utf-8",
                "-J",
                "--flat-playlist",
                "--no-playlist",
                "--",
                mix
            ])
        );
        let playlist =
            "https://www.youtube.com/watch?v=viuYLuyILeo&list=PLhCCfdELbr0Pi9RLMNAGhzKyF50LqlB4U";
        assert!(
            !resolve_args(playlist, &run_options(), PlaylistScope::WholePlaylist)
                .contains(&"--playlist-items".to_string())
        );
    }
    #[test]
    fn playlist_resolve_args_ask_for_the_whole_playlist() {
        let url = "https://www.youtube.com/watch?v=x&list=PLy";
        assert_eq!(
            resolve_args(url, &run_options(), PlaylistScope::WholePlaylist),
            strings(&[
                "--encoding",
                "utf-8",
                "-J",
                "--flat-playlist",
                "--yes-playlist",
                "--",
                url
            ])
        );
        let with_cookies = RunOptions {
            cookies_from_browser: Some(Browser::Firefox),
            ..run_options()
        };
        assert_eq!(
            resolve_args(url, &with_cookies, PlaylistScope::WholePlaylist),
            strings(&[
                "--encoding",
                "utf-8",
                "-J",
                "--flat-playlist",
                "--yes-playlist",
                "--cookies-from-browser",
                "firefox",
                "--",
                url
            ])
        );
    }

    // --- progress parsing

    #[test]
    fn progress_line_with_exact_total() {
        let line = parse_progress_line(
            "downloading|130048|252182|NA|4972341.102398381|0|/tmp/a|b [x].webm.part",
        )
        .unwrap();

        assert_eq!(line.status, "downloading");
        assert_eq!(
            line.tmpfilename,
            Some(PathBuf::from("/tmp/a|b [x].webm.part"))
        );
        let p = line.progress;
        assert_eq!(p.downloaded_bytes, Some(130048));
        assert_eq!(p.total_bytes, Some(252182));
        assert_eq!(p.speed_bytes_per_sec, Some(4972341.102398381));
        assert_eq!(p.eta_secs, Some(0));
        let percent = p.percent.unwrap();
        assert!((percent - 51.57).abs() < 0.01, "percent {percent}");
    }

    #[test]
    fn progress_line_with_only_an_estimated_total() {
        let line =
            parse_progress_line("downloading|500|NA|2000.0|NA|12.6|/tmp/v.mp4.part").unwrap();

        assert_eq!(line.progress.total_bytes, Some(2000));
        assert_eq!(line.progress.percent, Some(25.0));
        assert_eq!(line.progress.speed_bytes_per_sec, None);
        assert_eq!(line.progress.eta_secs, Some(13));
    }

    #[test]
    fn progress_line_with_everything_unknown() {
        let line = parse_progress_line("finished|NA|NA|NA|NA|NA|NA").unwrap();

        assert_eq!(line.status, "finished");
        assert_eq!(line.progress, Progress::default());
        assert_eq!(line.tmpfilename, None);
    }

    #[test]
    fn malformed_progress_lines_are_rejected() {
        assert!(parse_progress_line("downloading|12|34").is_err());
        assert!(parse_progress_line("downloading|lots|34|NA|NA|NA|x").is_err());
        assert!(parse_progress_line("downloading|-5|34|NA|NA|NA|x").is_err());
        assert!(parse_progress_line("|1|2|NA|NA|NA|x").is_err());
    }

    #[test]
    fn throttle_emits_at_most_every_interval_and_flushes_the_rest() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let p = |n: u64| Progress {
            downloaded_bytes: Some(n),
            ..Progress::default()
        };
        let mut throttle = ProgressThrottle::default();

        assert_eq!(throttle.offer(p(1), at(0), false), Some(p(1)));
        assert_eq!(throttle.offer(p(2), at(50), false), None);
        assert_eq!(throttle.offer(p(3), at(100), false), None);
        assert_eq!(throttle.offer(p(4), at(200), false), Some(p(4)));
        assert_eq!(throttle.offer(p(5), at(210), false), None);
        assert_eq!(throttle.flush(), Some(p(5)));
        assert_eq!(throttle.flush(), None);
        assert_eq!(throttle.offer(p(6), at(220), true), Some(p(6)));
    }

    #[test]
    fn parser_flushes_progress_before_processing_and_reads_the_file_path() {
        let start = Instant::now();
        let mut parser = DownloadParser::default();
        let mut events = Vec::new();
        let mut emit = |e: DownloadEvent| events.push(e);

        parser.handle_line(
            "yaydl-progress:downloading|10|100|NA|NA|NA|/o/a.webm.part",
            start,
            &mut emit,
        );
        parser.handle_line(
            "yaydl-progress:downloading|60|100|NA|NA|NA|/o/a.webm.part",
            start + Duration::from_millis(10),
            &mut emit,
        );
        parser.handle_line("yaydl-pp:started|ExtractAudio", start, &mut emit);
        parser.handle_line("yaydl-pp:finished|ExtractAudio", start, &mut emit);
        parser.handle_line("yaydl-file:/o/a [id].mp3", start, &mut emit);

        let percents: Vec<Option<f32>> = events
            .iter()
            .filter_map(|e| match e {
                DownloadEvent::Progress(p) => Some(p.percent),
                DownloadEvent::Processing => None,
            })
            .collect();
        assert_eq!(percents, vec![Some(10.0), Some(60.0)]);
        assert_eq!(events.last(), Some(&DownloadEvent::Processing));
        assert_eq!(
            events
                .iter()
                .filter(|e| **e == DownloadEvent::Processing)
                .count(),
            1
        );
        assert_eq!(parser.file_path, Some(PathBuf::from("/o/a [id].mp3")));
        assert_eq!(parser.temp_files, vec![PathBuf::from("/o/a.webm.part")]);
    }

    // --- metadata parsing

    const SINGLE_JSON: &str = r#"{
        "_type": "video",
        "id": "jNQXAC9IVRw",
        "title": "Me at the zoo",
        "uploader": "jawed",
        "channel": "jawed channel",
        "duration": 19,
        "thumbnail": "https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg",
        "webpage_url": "https://www.youtube.com/watch?v=jNQXAC9IVRw",
        "extractor_key": "Youtube",
        "formats": [{"format_id": "18"}]
    }"#;

    const PLAYLIST_JSON: &str = r#"{
        "_type": "playlist",
        "id": "PLx",
        "title": "Top Trending Videos of the Week",
        "extractor_key": "YoutubeTab",
        "entries": [
            {
                "_type": "url", "ie_key": "Youtube", "id": "Vh4O04Bpovw",
                "url": "https://www.youtube.com/watch?v=Vh4O04Bpovw",
                "title": "I Explored A Forgotten Space Colony", "duration": 969,
                "channel": "Yes Theory", "uploader": null,
                "thumbnails": [
                    {"url": "https://i.ytimg.com/small.jpg", "height": 94},
                    {"url": "https://i.ytimg.com/large.jpg", "height": 188}
                ]
            },
            {
                "_type": "url", "ie_key": "Youtube", "id": "priv1",
                "url": "https://www.youtube.com/watch?v=priv1",
                "title": "[Private video]", "duration": null
            },
            {
                "_type": "url", "ie_key": "Youtube", "id": "del1",
                "url": "https://www.youtube.com/watch?v=del1",
                "title": "[Deleted video]"
            },
            {
                "_type": "playlist", "id": "UCLA_DiR1FfKNvjuUpBHmylQ",
                "title": "NASA - Shorts", "entries": []
            },
            {
                "_type": "url", "ie_key": "Youtube", "title": "no id here",
                "url": "https://www.youtube.com/watch?v=unknown"
            },
            {
                "_type": "url", "ie_key": "Youtube", "id": "nourl", "title": "no url"
            },
            {
                "_type": "url", "ie_key": "Youtube", "id": "5TIp7oVKHq8",
                "url": "https://www.youtube.com/watch?v=5TIp7oVKHq8",
                "title": "Second", "duration": 1158.5, "uploader": "Uploader",
                "thumbnail": "https://i.ytimg.com/only.jpg"
            }
        ]
    }"#;

    #[test]
    fn single_video_metadata_prefers_uploader_over_channel() {
        let resolved = parse_resolved(SINGLE_JSON).unwrap();

        assert_eq!(
            resolved,
            Resolved::Single(VideoMetadata {
                extractor: "Youtube".to_string(),
                video_id: "jNQXAC9IVRw".to_string(),
                title: "Me at the zoo".to_string(),
                uploader: Some("jawed".to_string()),
                duration_secs: Some(19.0),
                thumbnail: Some("https://i.ytimg.com/vi/jNQXAC9IVRw/hqdefault.jpg".to_string()),
                webpage_url: VIDEO_URL.to_string(),
            })
        );
    }

    #[test]
    fn single_video_falls_back_to_channel_without_uploader() {
        let json = SINGLE_JSON.replace("\"uploader\": \"jawed\",", "");
        let Resolved::Single(metadata) = parse_resolved(&json).unwrap() else {
            panic!("expected a single video");
        };
        assert_eq!(metadata.uploader, Some("jawed channel".to_string()));
    }

    #[test]
    fn single_video_without_a_required_field_names_the_field() {
        for field in ["id", "title", "extractor_key", "webpage_url"] {
            let mut value: Value = serde_json::from_str(SINGLE_JSON).unwrap();
            value.as_object_mut().unwrap().remove(field);
            let error = parse_resolved(&value.to_string()).expect_err("missing field to fail");
            assert!(error.contains(field), "{field}: {error}");
        }
    }

    #[test]
    fn playlist_skips_private_deleted_nested_and_incomplete_entries() {
        let Resolved::Playlist {
            title,
            entries,
            unavailable,
        } = parse_resolved(PLAYLIST_JSON).unwrap()
        else {
            panic!("expected a playlist");
        };

        assert_eq!(title.as_deref(), Some("Top Trending Videos of the Week"));
        assert_eq!(unavailable, 5);
        assert_eq!(
            entries,
            vec![
                VideoMetadata {
                    extractor: "Youtube".to_string(),
                    video_id: "Vh4O04Bpovw".to_string(),
                    title: "I Explored A Forgotten Space Colony".to_string(),
                    uploader: Some("Yes Theory".to_string()),
                    duration_secs: Some(969.0),
                    thumbnail: Some("https://i.ytimg.com/large.jpg".to_string()),
                    webpage_url: "https://www.youtube.com/watch?v=Vh4O04Bpovw".to_string(),
                },
                VideoMetadata {
                    extractor: "Youtube".to_string(),
                    video_id: "5TIp7oVKHq8".to_string(),
                    title: "Second".to_string(),
                    uploader: Some("Uploader".to_string()),
                    duration_secs: Some(1158.5),
                    thumbnail: Some("https://i.ytimg.com/only.jpg".to_string()),
                    webpage_url: "https://www.youtube.com/watch?v=5TIp7oVKHq8".to_string(),
                },
            ]
        );
    }

    #[test]
    fn fully_resolved_playlist_entries_use_the_page_url_not_the_media_url() {
        let json = r#"{"_type": "playlist", "entries": [{
            "id": "a1", "title": "Track", "extractor_key": "Bandcamp",
            "url": "https://cdn.example/stream.mp3",
            "webpage_url": "https://artist.bandcamp.com/track/a1"
        }]}"#;
        let Resolved::Playlist { entries, .. } = parse_resolved(json).unwrap() else {
            panic!("expected a playlist");
        };
        assert_eq!(
            entries[0].webpage_url,
            "https://artist.bandcamp.com/track/a1"
        );
        assert_eq!(entries[0].extractor, "Bandcamp");
    }

    #[test]
    fn unknown_result_types_and_garbage_are_rejected() {
        assert!(parse_resolved(r#"{"_type": "multi_video"}"#).is_err());
        assert!(parse_resolved("not json").is_err());
        assert!(parse_resolved("[]").is_err());
    }

    // --- classification

    #[test]
    fn classification_table() {
        let cases = [
            (
                "ERROR: [youtube] x: Sign in to confirm your age. This video may be inappropriate \
                 for some users. Use --cookies-from-browser or --cookies for the authentication.",
                ErrorKind::AgeRestricted,
            ),
            (
                "ERROR: [youtube] x: Sign in to confirm you\u{2019}re not a bot. Use \
                 --cookies-from-browser or --cookies for the authentication.",
                ErrorKind::BotCheck,
            ),
            (
                "ERROR: [youtube] x: Sign in to confirm you're not a bot",
                ErrorKind::BotCheck,
            ),
            (
                "ERROR: [youtube] x: Private video. Sign in if you've been granted access to this \
                 video. Use --cookies-from-browser or --cookies for the authentication.",
                ErrorKind::Private,
            ),
            (
                "ERROR: [youtube] x: Join this channel to get access to members-only content",
                ErrorKind::Private,
            ),
            (
                "ERROR: [youtube] x: Video unavailable. This video has been removed by the uploader",
                ErrorKind::Unavailable,
            ),
            (
                "ERROR: [youtube] x: Video unavailable. The uploader has not made this video \
                 available in your country",
                ErrorKind::GeoBlocked,
            ),
            (
                "ERROR: [youtube] x: Unable to download webpage: HTTP Error 429: Too Many Requests",
                ErrorKind::RateLimited,
            ),
            (
                "ERROR: [youtube] x: Unable to download webpage: <urlopen error [Errno -3] \
                 Temporary failure in name resolution>",
                ErrorKind::Network,
            ),
            (
                "ERROR: Unsupported URL: https://example.com/",
                ErrorKind::UnsupportedUrl,
            ),
            (
                "ERROR: [youtube] x: Premieres in 5 hours",
                ErrorKind::NotYetLive,
            ),
            (
                "ERROR: unable to write data: [Errno 28] No space left on device",
                ErrorKind::DiskFull,
            ),
            (STALE_EXTRACTOR, ErrorKind::Other),
            (
                "ERROR: unable to download video data: HTTP Error 403: Forbidden",
                ErrorKind::Other,
            ),
        ];
        for (stderr, expected) in cases {
            let error = classify(stderr, Some(1));
            assert_eq!(error.kind, expected, "{stderr}");
            assert_eq!(error.message, friendly_message(expected));
            assert_eq!(error.detail, stderr);
        }
    }

    #[test]
    fn classification_ignores_warnings_when_there_is_an_error_line() {
        let stderr =
            format!("WARNING: [youtube] Read timed out. Retrying (1/3)...\n{STALE_EXTRACTOR}");
        assert_eq!(classify(&stderr, Some(1)).kind, ErrorKind::Other);
        assert_eq!(
            classify("WARNING: Read timed out", Some(1)).kind,
            ErrorKind::Network
        );
    }

    #[test]
    fn detail_is_the_stderr_tail_without_markers() {
        let noisy = (0..20)
            .map(|i| format!("line {i}"))
            .chain(["yaydl-pp:started|Merger".to_string(), String::new()])
            .collect::<Vec<_>>()
            .join("\n");
        let detail = classify(&noisy, Some(1)).detail;
        assert_eq!(detail.lines().count(), STDERR_TAIL_LINES);
        assert!(detail.starts_with("line 12"), "{detail}");
        assert!(detail.ends_with("line 19"), "{detail}");

        assert_eq!(
            classify("", Some(2)).detail,
            "yt-dlp exited with code 2 and printed nothing on stderr"
        );
        assert!(classify("", None).detail.contains("signal"));
    }

    // --- resolve and download through the fake runner

    #[tokio::test]
    async fn resolve_returns_parsed_metadata() {
        let (manager, calls, _dir) = manager(vec![ok(vec![out(SINGLE_JSON)])]);

        let resolved = manager.resolve(VIDEO_URL, &run_options()).await.unwrap();

        assert!(matches!(resolved, Resolved::Single(ref m) if m.video_id == "jNQXAC9IVRw"));
        assert_eq!(
            calls_of(&calls),
            vec![resolve_args(
                VIDEO_URL,
                &run_options(),
                PlaylistScope::VideoOnly
            )]
        );
    }

    #[tokio::test]
    async fn resolve_playlist_parses_entries_and_self_heals_with_the_same_args() {
        let url = "https://www.youtube.com/watch?v=Vh4O04Bpovw&list=PLx";
        let (manager, calls, _dir) = manager(vec![
            fail(STALE_EXTRACTOR),
            ok(vec![out("2026.02.03")]),
            ok(vec![out("Updated yt-dlp to nightly@2026.09.18.232920")]),
            ok(vec![out("2026.09.18.232920")]),
            ok(vec![out(PLAYLIST_JSON)]),
        ]);

        let resolved = manager.resolve_playlist(url, &run_options()).await;

        let Ok(Resolved::Playlist {
            entries,
            unavailable,
            ..
        }) = resolved
        else {
            panic!("expected a playlist, got {resolved:?}");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(unavailable, 5);
        let resolve = resolve_args(url, &run_options(), PlaylistScope::WholePlaylist);
        assert_eq!(
            calls_of(&calls),
            vec![
                resolve.clone(),
                version_args(),
                update_args("nightly"),
                version_args(),
                resolve,
            ]
        );
    }

    #[tokio::test]
    async fn download_reports_progress_from_both_streams_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Me at the zoo [jNQXAC9IVRw].mp3");
        fs::write(&file, b"mp3").unwrap();
        let (manager, _calls, _state) = manager(vec![ok(vec![
            out("yaydl-progress:downloading|1024|4096|NA|NA|NA|/o/a.webm.part"),
            err("yaydl-progress:finished|4096|4096|NA|NA|NA|NA"),
            err("yaydl-pp:started|ExtractAudio"),
            out(&format!("yaydl-file:{}", file.display())),
        ])]);
        let (events, on_event) = recorder();

        let outcome = manager
            .download(
                &request(dir.path(), mp3()),
                CancellationToken::new(),
                on_event,
            )
            .await
            .unwrap();

        assert_eq!(outcome.file_path, file);
        let events = events.lock().unwrap().clone();
        assert_eq!(events.len(), 3, "{events:?}");
        assert!(matches!(&events[0], DownloadEvent::Progress(p) if p.percent == Some(25.0)));
        assert!(matches!(&events[1], DownloadEvent::Progress(p) if p.percent == Some(100.0)));
        assert_eq!(events[2], DownloadEvent::Processing);
    }

    #[tokio::test]
    async fn download_without_the_reported_file_on_disk_fails() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.mp3");
        let (manager, _calls, _state) = manager(vec![
            ok(vec![out(&format!("yaydl-file:{}", missing.display()))]),
            ok(vec![]),
        ]);
        let request = request(dir.path(), mp3());

        let result = manager
            .download(&request, CancellationToken::new(), recorder().1)
            .await;
        match result {
            Err(YtDlpFailure::Failed(e)) => {
                assert_eq!(e.kind, ErrorKind::Other);
                assert!(e.detail.contains("gone.mp3"), "{}", e.detail);
            }
            other => panic!("expected Failed, got {other:?}"),
        }

        let result = manager
            .download(&request, CancellationToken::new(), recorder().1)
            .await;
        match result {
            Err(YtDlpFailure::Failed(e)) => {
                assert!(e.detail.contains(FILE_MARKER), "{}", e.detail)
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cancelled_download_returns_cancelled_and_removes_partial_files() {
        let dir = tempfile::tempdir().unwrap();
        let first_format = dir.path().join("clip.f395.mp4");
        let part = dir.path().join("clip.f140.m4a.part");
        let state = dir.path().join("clip.f140.m4a.ytdl");
        let fragment = dir.path().join("clip.f140.m4a.part-Frag3");
        let unrelated = dir.path().join("other.mp4");
        for path in [&first_format, &part, &state, &fragment, &unrelated] {
            fs::write(path, b"x").unwrap();
        }
        let (manager, _calls, _state) = manager(vec![FakeStep::Hang {
            lines: vec![
                out(&format!(
                    "yaydl-progress:downloading|1|2|NA|NA|NA|{}.part",
                    first_format.display()
                )),
                out(&format!(
                    "yaydl-progress:downloading|1|2|NA|NA|NA|{}",
                    part.display()
                )),
            ],
        }]);
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();

        let result = manager
            .download(
                &request(dir.path(), mp3()),
                cancel,
                Box::new(move |_| trigger.cancel()),
            )
            .await;

        assert_eq!(result, Err(YtDlpFailure::Cancelled));
        for path in [&first_format, &part, &state, &fragment] {
            assert!(!path.exists(), "{} was not removed", path.display());
        }
        assert!(unrelated.exists(), "an unrelated file was removed");
    }

    #[tokio::test]
    async fn a_download_cancelled_before_it_starts_never_runs() {
        let (manager, calls, dir) = manager(Vec::new());
        let cancel = CancellationToken::new();
        cancel.cancel();

        let result = manager
            .download(&request(dir.path(), mp3()), cancel, recorder().1)
            .await;

        assert_eq!(result, Err(YtDlpFailure::Cancelled));
        assert!(calls_of(&calls).is_empty());
    }

    // --- self-heal

    #[tokio::test]
    async fn an_unclassified_failure_updates_and_retries_once() {
        let (manager, calls, _dir) = manager(vec![
            fail(STALE_EXTRACTOR),
            ok(vec![out("2026.02.03")]),
            ok(vec![out("Updated yt-dlp to nightly@2026.09.18.232920")]),
            ok(vec![out("2026.09.18.232920")]),
            ok(vec![out(SINGLE_JSON)]),
        ]);

        let resolved = manager.resolve(VIDEO_URL, &run_options()).await;

        assert!(matches!(resolved, Ok(Resolved::Single(_))), "{resolved:?}");
        let resolve = resolve_args(VIDEO_URL, &run_options(), PlaylistScope::VideoOnly);
        assert_eq!(
            calls_of(&calls),
            vec![
                resolve.clone(),
                version_args(),
                update_args("nightly"),
                version_args(),
                resolve,
            ]
        );
        assert!(manager.state_file().exists());
    }

    #[tokio::test]
    async fn self_heal_uses_the_configured_channel() {
        let (manager, calls, _dir) = manager(vec![
            fail(STALE_EXTRACTOR),
            ok(vec![out("2026.02.03")]),
            ok(vec![]),
            ok(vec![out("2026.03.31")]),
            ok(vec![out(SINGLE_JSON)]),
        ]);
        let run = RunOptions {
            channel: YtDlpChannel::Stable,
            ..run_options()
        };

        manager.resolve(VIDEO_URL, &run).await.unwrap();

        assert_eq!(calls_of(&calls)[2], update_args("stable"));
    }

    #[tokio::test]
    async fn a_failing_retry_reports_the_retry_error() {
        let (manager, calls, _dir) = manager(vec![
            fail("ERROR: first attempt"),
            ok(vec![out("2026.02.03")]),
            ok(vec![]),
            ok(vec![out("2026.09.18")]),
            fail("ERROR: retry attempt"),
        ]);

        let result = manager.resolve(VIDEO_URL, &run_options()).await;

        match result {
            Err(YtDlpFailure::Failed(e)) => {
                assert_eq!(e.kind, ErrorKind::Other);
                assert_eq!(e.detail, "ERROR: retry attempt");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(update_count(&calls), 1);
        assert_eq!(calls_of(&calls).len(), 5);
    }

    #[tokio::test]
    async fn a_failed_self_heal_update_is_reported_with_the_original_error() {
        let (manager, calls, _dir) = manager(vec![
            fail(STALE_EXTRACTOR),
            ok(vec![out("2026.02.03")]),
            fail("ERROR: unable to write to /opt/yt-dlp"),
        ]);

        let result = manager.resolve(VIDEO_URL, &run_options()).await;

        match result {
            Err(YtDlpFailure::Failed(e)) => {
                assert_eq!(e.kind, ErrorKind::Other);
                assert!(e.detail.starts_with(STALE_EXTRACTOR), "{}", e.detail);
                assert!(
                    e.detail.contains("unable to write to /opt/yt-dlp"),
                    "{}",
                    e.detail
                );
            }
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(calls_of(&calls).len(), 3);
    }

    #[tokio::test]
    async fn a_classified_failure_never_updates() {
        let (manager, calls, dir) = manager(vec![fail(
            "ERROR: [youtube] x: Video unavailable. This video has been removed by the uploader",
        )]);

        let result = manager
            .download(
                &request(dir.path(), mp3()),
                CancellationToken::new(),
                recorder().1,
            )
            .await;

        match result {
            Err(YtDlpFailure::Failed(e)) => assert_eq!(e.kind, ErrorKind::Unavailable),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(calls_of(&calls).len(), 1);
    }

    #[tokio::test]
    async fn a_recent_update_check_skips_the_self_heal() {
        let (manager, calls, dir) = manager(vec![fail(STALE_EXTRACTOR)]);
        write_update_check(&manager, 60);

        let result = manager
            .download(
                &request(dir.path(), mp3()),
                CancellationToken::new(),
                recorder().1,
            )
            .await;

        match result {
            Err(YtDlpFailure::Failed(e)) => assert_eq!(e.kind, ErrorKind::Other),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(calls_of(&calls).len(), 1);
    }

    #[tokio::test]
    async fn an_old_update_check_allows_the_self_heal() {
        let (manager, calls, _dir) = manager(vec![
            fail(STALE_EXTRACTOR),
            ok(vec![out("1")]),
            ok(vec![]),
            ok(vec![out("2")]),
            ok(vec![out(SINGLE_JSON)]),
        ]);
        write_update_check(&manager, SELF_HEAL_MIN_INTERVAL.as_secs() + 1);

        manager.resolve(VIDEO_URL, &run_options()).await.unwrap();

        assert_eq!(update_count(&calls), 1);
    }

    #[tokio::test]
    async fn two_concurrent_failures_trigger_exactly_one_update() {
        let output_dir = tempfile::tempdir().unwrap();
        let file = output_dir.path().join("done.mp3");
        fs::write(&file, b"mp3").unwrap();
        let file_line = format!("yaydl-file:{}", file.display());
        let barrier = Arc::new(Barrier::new(2));
        let updated = Arc::new(AtomicBool::new(false));
        let handler_updated = Arc::clone(&updated);
        let (manager, calls, _dir) = manager_with(Box::new(move |args| match args[0].as_str() {
            "--version" => ok(vec![out("2026.09.18")]),
            "--update-to" => {
                handler_updated.store(true, Ordering::SeqCst);
                ok(vec![])
            }
            _ if handler_updated.load(Ordering::SeqCst) => ok(vec![out(&file_line)]),
            _ => FakeStep::Gated(Arc::clone(&barrier), Box::new(fail(STALE_EXTRACTOR))),
        }));
        let request = request(output_dir.path(), mp3());

        let (a, b) = tokio::join!(
            manager.download(&request, CancellationToken::new(), recorder().1),
            manager.download(&request, CancellationToken::new(), recorder().1),
        );

        assert_eq!(a.unwrap().file_path, file);
        assert_eq!(b.unwrap().file_path, file);
        assert_eq!(update_count(&calls), 1, "{:?}", calls_of(&calls));
        assert_eq!(*manager.binary_lock.read().await, 1);
    }

    // --- startup update check

    #[tokio::test]
    async fn startup_without_state_file_updates_and_records_the_check() {
        let (manager, calls, _dir) = manager(vec![
            ok(vec![out("2026.02.03")]),
            ok(vec![]),
            ok(vec![out("2026.09.18.232920")]),
        ]);

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
        assert_eq!(
            calls_of(&calls),
            vec![version_args(), update_args("nightly"), version_args()]
        );
        assert!(manager.state_file().exists());
    }

    #[tokio::test]
    async fn startup_within_the_check_interval_does_nothing() {
        let (manager, calls, _dir) = manager(Vec::new());
        write_update_check(&manager, 60);

        let event = manager
            .maybe_update_on_startup(YtDlpChannel::Nightly)
            .await
            .expect("startup check to succeed");

        assert_eq!(event, None);
        assert!(calls_of(&calls).is_empty());
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
        assert!(calls_of(&calls).is_empty());
    }

    #[tokio::test]
    async fn startup_past_the_check_interval_updates() {
        let (manager, calls, _dir) = manager(vec![
            ok(vec![out("2026.02.03")]),
            ok(vec![]),
            ok(vec![out("2026.02.03")]),
        ]);
        write_update_check(&manager, UPDATE_CHECK_INTERVAL.as_secs() + 1);

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
        assert_eq!(calls_of(&calls).len(), 3);
    }

    #[tokio::test]
    async fn a_failed_update_is_reported_and_keeps_the_generation() {
        let (manager, _calls, _dir) = manager(vec![
            ok(vec![out("2026.02.03")]),
            fail("ERROR: unable to write to /opt/yt-dlp"),
        ]);

        let result = manager.update(YtDlpChannel::Nightly).await;

        assert_eq!(
            result,
            Err(YtDlpError::UpdateFailed {
                channel: YtDlpChannel::Nightly,
                stderr: "ERROR: unable to write to /opt/yt-dlp".to_string(),
            })
        );
        assert_eq!(*manager.binary_lock.read().await, 0);
    }
}
