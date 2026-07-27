// SPDX-License-Identifier: Apache-2.0
//! Progress reporting for long-running commands (§ "pull / build / package の進行状況").
//!
//! A single [`Reporter`] drives every long command (`build`, and later `pull` /
//! `package` / plugin) so their phase, elapsed time, heartbeat and log location
//! read the same way. It renders differently for an interactive terminal and for
//! CI, but the *event model* is identical:
//!
//! - **Human** (a TTY): `[2/4] Configuring CMake`, an idle heartbeat that
//!   reprints the phase with `… elapsed mm:ss`, and a final `completed in mm:ss`.
//! - **Plain** (non-TTY / CI): one machine-greppable line per transition,
//!   `phase=<slug> status=started|completed|failed` with `duration_ms=…`.
//! - **Json** (`--progress json`): one JSON object per line — `phase_started`,
//!   `heartbeat`, `phase_completed`, `phase_failed`, `completed` — for tools that
//!   consume an event stream. Child output is captured to the log only so stdout
//!   stays a clean stream.
//!
//! We never invent a percentage: progress is reported as *phases* plus elapsed
//! time, with a heartbeat so a quiet child process never looks hung. Child
//! stdout/stderr is passed through (or, with `--quiet`/`json`, captured to the
//! log only) and always teed to the per-target log so failures point at a file.
//!
//! With `--notify`, a best-effort OS notification fires on completion (success
//! or failure); it is a no-op over SSH or in CI (see [`crate::notify`]).

use std::collections::VecDeque;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use camino::Utf8Path;
use clap::ValueEnum;

use ost_core::{Error, Result};

use crate::notify;

/// How progress is rendered. `auto` picks Human on a TTY, Plain otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum ProgressMode {
    /// Human on a terminal, plain key=value lines when piped / in CI.
    Auto,
    /// Always emit plain `phase=… status=…` lines (good for CI logs).
    Plain,
    /// Emit one JSON object per line (a stable event stream for tools).
    Json,
}

/// The resolved rendering style (after `auto` detection).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Human,
    Plain,
    Json,
}

/// Idle time with no child output before a heartbeat is emitted.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// Idle time with no child output before the run is reported as *stalled*.
///
/// A heartbeat is chatter and is suppressed under `--quiet`; a stall is not. A
/// managed configure that stops producing output — CMake's compiler-ABI
/// try-compile is the one that does it — is a failure mode with nothing else to
/// look at, and `renderer view --json` runs its nested build quiet, so the
/// heartbeat is exactly the diagnostic that is missing when it is needed.
const STALL: Duration = Duration::from_secs(120);

const OUTPUT_TAIL_BYTES: usize = 4096;

/// Bytes read from the end of a watched diagnostic file for its tail.
const FILE_TAIL_BYTES: u64 = 4096;

/// Lines of a watched diagnostic file reported on a stall or timeout.
const FILE_TAIL_LINES: usize = 3;

struct PhaseState {
    name: String,
    slug: String,
    started: Instant,
}

/// The child process a stall notice is about.
struct ChildRef<'a> {
    pid: u32,
    /// The rendered command line, so a stalled tool can be identified (and
    /// reproduced) without the caller having logged it.
    command: String,
    cwd: &'a Utf8Path,
}

/// How a phase ended, for its terminal transition line.
enum Outcome {
    /// The phase finished cleanly (quiet-suppressible).
    Completed,
    /// The phase failed; `Some(code)` is a child process exit code, `None` an
    /// in-process error (generate/verify). Failures always surface, even quiet.
    Failed(Option<i32>),
}

/// Drives phase/heartbeat/log reporting for one command invocation.
pub struct Reporter {
    style: Style,
    quiet: bool,
    total: usize,
    index: usize,
    started: Instant,
    log: Option<PathBuf>,
    /// Files worth tailing when the current phase stalls or times out — the
    /// CMake logs of the tree being configured, for instance. A stalled child
    /// has told us nothing on its pipe by definition, so the retained log of the
    /// tool that stalled is the only evidence left. Cleared on every phase.
    diagnostics: Vec<PathBuf>,
    current: Option<PhaseState>,
    /// Silence after which a running child is reported as stalled.
    stall_after: Duration,
    /// Fire an OS notification on completion. Already gated on the environment
    /// (false over SSH / in CI even when `--notify` was passed).
    notify: bool,
    /// Short command label for the notification, e.g. `ost build`.
    label: String,
}

impl Reporter {
    /// Create a reporter for a command with `total` phases. `quiet` suppresses
    /// progress chatter and child passthrough, but never failure reporting.
    pub fn new(mode: ProgressMode, total: usize, quiet: bool) -> Reporter {
        let style = match mode {
            ProgressMode::Plain => Style::Plain,
            ProgressMode::Json => Style::Json,
            ProgressMode::Auto => {
                if std::io::stdout().is_terminal() {
                    Style::Human
                } else {
                    Style::Plain
                }
            }
        };
        Reporter {
            style,
            quiet,
            total,
            index: 0,
            started: Instant::now(),
            log: None,
            diagnostics: Vec::new(),
            current: None,
            stall_after: STALL,
            notify: false,
            label: String::new(),
        }
    }

    /// Override the silence that counts as a stall. Tests use it to reach the
    /// stall path in milliseconds instead of minutes.
    #[cfg(test)]
    fn stall_after(mut self, after: Duration) -> Reporter {
        self.stall_after = after;
        self
    }

    /// Enable an OS notification on completion, labelled `label` (e.g.
    /// `ost build`). Honours the opt-in `requested` flag but stays off where a
    /// desktop toast has no audience (SSH / CI), per [`notify::enabled`].
    pub fn with_notify(mut self, requested: bool, label: &str) -> Reporter {
        self.notify = requested && notify::enabled();
        self.label = label.to_string();
        self
    }

    /// Tee child output to (and report) this log file. Created on first write;
    /// a failure to open it is non-fatal (logging is best-effort).
    pub fn set_log(&mut self, path: &Utf8Path) {
        self.log = Some(path.as_std_path().to_path_buf());
    }

    /// Name the files to tail if the current phase stalls or times out. They do
    /// not have to exist: a CMake log only appears once configure reaches it,
    /// and the absence of one is itself part of the diagnostic.
    ///
    /// Set per phase, and cleared by the next [`phase`](Self::phase).
    pub fn watch(&mut self, paths: Vec<PathBuf>) {
        self.diagnostics = paths;
    }

    /// Print an incidental human note (e.g. an env summary). Rendered for Human
    /// and Plain, but suppressed under `--quiet` and in Json mode so the JSON
    /// event stream on stdout stays pure.
    pub fn note(&self, msg: &str) {
        if self.quiet || matches!(self.style, Style::Json) {
            return;
        }
        println!("      {msg}");
    }

    /// Begin a new phase, closing the previous one as completed.
    pub fn phase(&mut self, name: &str) {
        self.close_current(Outcome::Completed);
        self.diagnostics.clear();
        self.index += 1;
        let state = PhaseState {
            name: name.to_string(),
            slug: slug(name),
            started: Instant::now(),
        };
        if !self.quiet {
            match self.style {
                Style::Human => println!("[{}/{}] {}", self.index, self.total, name),
                Style::Plain => println!(
                    "timestamp={} phase={} status=started",
                    now_unix(),
                    state.slug
                ),
                Style::Json => emit_json(serde_json::json!({
                    "event": "phase_started",
                    "phase": state.slug,
                    "index": self.index,
                    "total": self.total,
                    "timestamp": now_unix(),
                })),
            }
        }
        self.current = Some(state);
    }

    /// Close the final phase and print the total wall time.
    pub fn done(&mut self) {
        self.close_current(Outcome::Completed);
        let elapsed = self.started.elapsed();
        if !self.quiet {
            match self.style {
                Style::Human => println!("completed in {}", hms(elapsed)),
                Style::Plain => println!(
                    "timestamp={} phase=all status=completed duration_ms={}",
                    now_unix(),
                    elapsed.as_millis()
                ),
                Style::Json => emit_json(serde_json::json!({
                    "event": "completed",
                    "duration_ms": elapsed.as_millis() as u64,
                    "timestamp": now_unix(),
                })),
            }
        }
        if self.notify {
            notify::send(
                &format!("{} ✓", self.label),
                &format!("completed in {}", hms(elapsed)),
            );
        }
    }

    /// Emit the terminal transition for the current phase, if any. This is the
    /// single sink for *every* phase end so a `started` line always has a
    /// matching `completed`/`failed` — whether the phase ended cleanly, a child
    /// process failed (via [`run`](Self::run)), or an in-process phase errored
    /// and the reporter is dropped while unwinding ([`Drop`]).
    fn close_current(&mut self, outcome: Outcome) {
        let Some(state) = self.current.take() else {
            return;
        };
        let dur = state.started.elapsed();
        match outcome {
            // Clean completion is chatter — suppressible under --quiet.
            Outcome::Completed => {
                if self.quiet {
                    return;
                }
                match self.style {
                    Style::Human => {
                        // A short phase needs no echo; only annotate the slow
                        // ones so the log stays terse.
                        if dur >= Duration::from_secs(1) {
                            println!("      done in {}", hms(dur));
                        }
                    }
                    Style::Plain => println!(
                        "timestamp={} phase={} status=completed duration_ms={}",
                        now_unix(),
                        state.slug,
                        dur.as_millis()
                    ),
                    Style::Json => emit_json(serde_json::json!({
                        "event": "phase_completed",
                        "phase": state.slug,
                        "duration_ms": dur.as_millis() as u64,
                        "timestamp": now_unix(),
                    })),
                }
            }
            // Failures always surface (even under --quiet), naming the phase,
            // the exit code (if any) and the log path.
            Outcome::Failed(exit) => {
                let code = exit.map(|c| c.to_string());
                let log = self.log.as_ref().map(|p| p.display().to_string());
                match self.style {
                    Style::Human => {
                        let exit = code.map(|c| format!("exit {c}, ")).unwrap_or_default();
                        eprintln!(
                            "[{}/{}] {} FAILED ({exit}after {})",
                            self.index,
                            self.total,
                            state.name,
                            hms(dur)
                        );
                        if let Some(log) = &log {
                            eprintln!("      log: {log}");
                        }
                    }
                    Style::Plain => {
                        let exit = code.map(|c| format!(" exit_code={c}")).unwrap_or_default();
                        eprintln!(
                            "timestamp={} phase={} status=failed{exit} duration_ms={}",
                            now_unix(),
                            state.slug,
                            dur.as_millis()
                        );
                        if let Some(log) = &log {
                            eprintln!(
                                "timestamp={} phase={} status=failed log={log}",
                                now_unix(),
                                state.slug,
                            );
                        }
                    }
                    Style::Json => emit_json(serde_json::json!({
                        "event": "phase_failed",
                        "phase": state.slug,
                        "exit_code": exit,
                        "duration_ms": dur.as_millis() as u64,
                        "log": log,
                        "timestamp": now_unix(),
                    })),
                }
                if self.notify {
                    notify::send(
                        &format!("{} ✗", self.label),
                        &format!("failed at {}", state.name),
                    );
                }
            }
        }
    }

    /// Emit an idle heartbeat for the running phase (no child output for a while).
    fn heartbeat(&self, idle: Duration, pid: u32, tail: &str) {
        if self.quiet {
            return;
        }
        let Some(state) = &self.current else { return };
        let elapsed = state.started.elapsed();
        match self.style {
            Style::Human => {
                println!(
                    "[{}/{}] {} … elapsed {} (pid {pid}, waiting on output)",
                    self.index,
                    self.total,
                    state.name,
                    hms(elapsed)
                );
                if let Some(log) = &self.log {
                    println!("      log: {}", log.display());
                }
                if !tail.is_empty() {
                    println!("      last output: {}", one_line(tail));
                }
            }
            Style::Plain => println!(
                "timestamp={} phase={} status=running pid={} elapsed_ms={} last_output_ms={} log={} tail={}",
                now_unix(),
                state.slug,
                pid,
                elapsed.as_millis(),
                idle.as_millis(),
                self.log.as_ref().map(|path| path.display().to_string()).unwrap_or_default(),
                serde_json::to_string(tail).unwrap_or_else(|_| "\"\"".into())
            ),
            Style::Json => emit_json(serde_json::json!({
                "event": "heartbeat",
                "phase": state.slug,
                "pid": pid,
                "elapsed_ms": elapsed.as_millis() as u64,
                "last_output_ms": idle.as_millis() as u64,
                "log": self.log.as_ref().map(|path| path.display().to_string()),
                "last_output_tail": tail,
                "timestamp": now_unix(),
            })),
        }
    }

    /// Report that the running child has gone silent past [`STALL`].
    ///
    /// Unlike a heartbeat this is never suppressed: `--quiet` and `--json` are
    /// how a *nested* managed build runs, which is precisely the case where a
    /// stalled configure otherwise prints nothing at all. It names the active
    /// child (pid, command, cwd), the phase, the retained log, what is left of
    /// the timeout budget, and the tail of every watched diagnostic file — the
    /// same evidence the timeout path reports, delivered while the tool is still
    /// stuck rather than after the deadline.
    fn stall(
        &self,
        silent: Duration,
        child: &ChildRef<'_>,
        tail: &str,
        remaining: Option<Duration>,
    ) {
        let Some(state) = &self.current else { return };
        let elapsed = state.started.elapsed();
        let log = self.log.as_ref().map(|path| path.display().to_string());
        let diagnostics = self.diagnostic_tails();
        let budget = remaining
            .map(|left| format!("{}s left of the timeout budget", left.as_secs()))
            .unwrap_or_else(|| "no timeout configured".into());
        match self.style {
            Style::Human => {
                eprintln!(
                    "[{}/{}] {} STALLED — no output for {} (elapsed {}, {budget})",
                    self.index,
                    self.total,
                    state.name,
                    hms(silent),
                    hms(elapsed)
                );
                eprintln!("      pid {} · {}", child.pid, child.command);
                eprintln!("      cwd {}", child.cwd);
                if let Some(log) = &log {
                    eprintln!("      log: {log}");
                }
                if !tail.is_empty() {
                    eprintln!("      last output: {}", one_line(tail));
                }
                for (path, text) in &diagnostics {
                    eprintln!("      {path}: {}", one_line(text));
                }
            }
            Style::Plain => {
                eprintln!(
                    "timestamp={} phase={} status=stalled pid={} silent_ms={} elapsed_ms={} \
                     timeout_remaining_seconds={} cwd={} log={} command={} tail={}",
                    now_unix(),
                    state.slug,
                    child.pid,
                    silent.as_millis(),
                    elapsed.as_millis(),
                    remaining
                        .map(|left| left.as_secs().to_string())
                        .unwrap_or_else(|| "none".into()),
                    child.cwd,
                    log.as_deref().unwrap_or_default(),
                    serde_json::to_string(&child.command).unwrap_or_else(|_| "\"\"".into()),
                    serde_json::to_string(tail).unwrap_or_else(|_| "\"\"".into())
                );
                for (path, text) in &diagnostics {
                    eprintln!(
                        "timestamp={} phase={} status=stalled diagnostic={path} tail={}",
                        now_unix(),
                        state.slug,
                        serde_json::to_string(text).unwrap_or_else(|_| "\"\"".into())
                    );
                }
            }
            Style::Json => emit_json(serde_json::json!({
                "event": "stalled",
                "phase": state.slug,
                "pid": child.pid,
                "command": child.command,
                "cwd": child.cwd,
                "silent_ms": silent.as_millis() as u64,
                "elapsed_ms": elapsed.as_millis() as u64,
                "timeout_remaining_seconds": remaining.map(|left| left.as_secs()),
                "log": log,
                "last_output_tail": tail,
                "diagnostics": diagnostics
                    .iter()
                    .map(|(path, text)| serde_json::json!({ "path": path, "tail": text }))
                    .collect::<Vec<_>>(),
                "timestamp": now_unix(),
            })),
        }
    }

    /// The tail of every watched diagnostic file that exists and has content.
    fn diagnostic_tails(&self) -> Vec<(String, String)> {
        self.diagnostics
            .iter()
            .filter_map(|path| {
                let tail = file_tail(path)?;
                Some((path.display().to_string(), tail))
            })
            .collect()
    }

    fn child_started(
        &self,
        pid: u32,
        program: &Path,
        args: &[String],
        cwd: &Utf8Path,
        timeout: Option<Duration>,
    ) {
        if self.quiet {
            return;
        }
        let Some(state) = &self.current else { return };
        let command = render_command(program, args);
        let timeout_secs = timeout.map(|value| value.as_secs());
        let log = self.log.as_ref().map(|path| path.display().to_string());
        match self.style {
            Style::Human => println!(
                "      pid {pid} · timeout {} · log {}",
                timeout_secs
                    .map(|seconds| format!("{seconds}s"))
                    .unwrap_or_else(|| "disabled".into()),
                log.as_deref().unwrap_or("disabled")
            ),
            Style::Plain => println!(
                "timestamp={} phase={} status=child-started pid={} timeout_seconds={} cwd={} log={} command={}",
                now_unix(),
                state.slug,
                pid,
                timeout_secs.map(|value| value.to_string()).unwrap_or_else(|| "0".into()),
                cwd,
                log.as_deref().unwrap_or_default(),
                serde_json::to_string(&command).unwrap_or_else(|_| "\"\"".into())
            ),
            Style::Json => emit_json(serde_json::json!({
                "event": "child_started",
                "phase": state.slug,
                "pid": pid,
                "timeout_seconds": timeout_secs,
                "cwd": cwd,
                "log": log,
                "command": command,
                "timestamp": now_unix(),
            })),
        }
    }

    /// Close the current phase and preserve an unsuccessful child's exit code.
    ///
    /// Callers that must publish failure evidence before terminating use
    /// [`Reporter::run_status`] and invoke this only after that evidence is
    /// durable. Keeping the exit here preserves phase reporting and the child
    /// status expected by CI.
    pub(crate) fn exit_unsuccessful(&mut self, status: std::process::ExitStatus) -> ! {
        debug_assert!(!status.success());
        self.close_current(Outcome::Failed(status.code()));
        std::process::exit(status.code().unwrap_or(1));
    }

    /// Run a child process under the current phase and hand its exit status back.
    ///
    /// Managed build and test commands need this because failure provenance or
    /// test completion evidence must be published before they return or exit.
    pub fn run_status(
        &mut self,
        program: &Path,
        args: &[String],
        cwd: &Utf8Path,
        env: &[(String, String)],
        timeout: Option<Duration>,
    ) -> Result<std::process::ExitStatus> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd.as_std_path())
            .envs(env.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_process_group(&mut cmd);

        // Armed before the spawn so the handler is installed by the time a child
        // can exist. One window remains and cannot be closed without blocking
        // signals around the spawn: between `spawn` returning and the pid being
        // recorded, an interrupt exits without killing the child it could not
        // name. That is the same microseconds every process-spawning tool has,
        // and it is not the failure this exists for — an interrupt during a
        // multi-minute configure lands in the poll loop below, where the pid is
        // recorded.
        interrupt::arm();
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::io(format!("run {}", program.display()), e))?;
        let pid = child.id();
        interrupt::set_active(pid);
        self.child_started(pid, program, args, cwd, timeout);

        // Shared "last output" clock and an optional log sink, both updated by the
        // reader threads as bytes arrive.
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let tail = Arc::new(Mutex::new(VecDeque::with_capacity(OUTPUT_TAIL_BYTES)));
        let log = self.open_log();
        // Pass child output through to our stdout/stderr, except when stdout is a
        // machine stream (Json) or silenced (--quiet) — then it goes to the log
        // only, keeping the event stream clean.
        let forward = !self.quiet && !matches!(self.style, Style::Json);

        let out = spawn_reader(
            child.stdout.take(),
            Sink::Out,
            last_output.clone(),
            tail.clone(),
            log.clone(),
            forward,
        );
        let err = spawn_reader(
            child.stderr.take(),
            Sink::Err,
            last_output.clone(),
            tail.clone(),
            log.clone(),
            forward,
        );

        // Poll for completion; while the child runs, emit a heartbeat whenever it
        // has produced no output for HEARTBEAT, and a louder stall notice once
        // the silence passes STALL.
        let mut last_beat = Instant::now();
        let mut last_stall: Option<Instant> = None;
        let child_started = Instant::now();
        let child_ref = ChildRef {
            pid,
            command: render_command(program, args),
            cwd,
        };
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    thread::sleep(Duration::from_millis(200));
                    if timeout.is_some_and(|limit| child_started.elapsed() >= limit) {
                        let phase = self
                            .current
                            .as_ref()
                            .map(|state| state.slug.clone())
                            .unwrap_or_else(|| "external-tool".into());
                        let diagnostics = self.diagnostic_tails();
                        let cleanup = terminate_process_tree(&mut child);
                        interrupt::clear();
                        // Never make a timeout unbounded again by joining a
                        // reader whose pipe is still held by an escaped
                        // descendant. Dropping JoinHandle detaches the reader;
                        // normal cleanup closes the pipe immediately, and a
                        // failed tree cleanup is already named in the error.
                        drop(out);
                        drop(err);
                        let tail = output_tail(&tail);
                        let log = self
                            .log
                            .as_ref()
                            .map(|path| path.display().to_string())
                            .unwrap_or_else(|| "<disabled>".into());
                        // The child said nothing on its pipe, so the retained log
                        // of the tool that hung is the evidence that remains.
                        let retained = diagnostics
                            .iter()
                            .map(|(path, text)| format!(", {path}: {}", one_line(text)))
                            .collect::<String>();
                        self.close_current(Outcome::Failed(None));
                        return Err(Error::external_tool(format!(
                            "command timed out after {}s: {} (phase '{phase}', pid {pid}, cwd '{cwd}', log '{log}', cleanup: {cleanup}, last output: {}{retained})",
                            timeout.unwrap_or_default().as_secs(),
                            child_ref.command,
                            if tail.is_empty() { "<none>".into() } else { one_line(&tail) }
                        ))
                        .with_phase(phase));
                    }
                    let idle = last_output.lock().map(|t| t.elapsed()).unwrap_or_default();
                    // A stall supersedes the heartbeat for this tick: they
                    // describe the same silence, and only one of them is worth
                    // interrupting a quiet run for.
                    if idle >= self.stall_after
                        && last_stall.is_none_or(|at| at.elapsed() >= self.stall_after)
                    {
                        let remaining =
                            timeout.map(|limit| limit.saturating_sub(child_started.elapsed()));
                        self.stall(idle, &child_ref, &output_tail(&tail), remaining);
                        last_stall = Some(Instant::now());
                        last_beat = Instant::now();
                    } else if idle >= HEARTBEAT && last_beat.elapsed() >= HEARTBEAT {
                        self.heartbeat(idle, pid, &output_tail(&tail));
                        last_beat = Instant::now();
                    }
                }
                Err(e) => {
                    interrupt::clear();
                    return Err(Error::io(format!("wait {}", program.display()), e));
                }
            }
        };
        // The pid is free to be reused the moment the child is reaped, so stop
        // pointing an interrupt at it.
        interrupt::clear();

        let _ = out.join();
        let _ = err.join();

        Ok(status)
    }

    /// Open (append) the log file once per `run`, best-effort.
    fn open_log(&self) -> Option<Arc<Mutex<std::fs::File>>> {
        let path = self.log.as_ref()?;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
            .map(|f| Arc::new(Mutex::new(f)))
    }
}

impl Drop for Reporter {
    /// A phase still open at drop time means we are unwinding on an error that
    /// did not close it explicitly — e.g. a failed `generate`/`verify` phase.
    /// Emit its terminal `failed` line so every `started` has a matching end,
    /// even in plain/CI output. A clean run leaves no open phase (`done`/`phase`
    /// close it), so this is a no-op there.
    fn drop(&mut self) {
        self.close_current(Outcome::Failed(None));
    }
}

/// Terminating a managed child when `ost` itself is interrupted.
///
/// Managed children are spawned into their own process group (so a console
/// Ctrl-C never races OpenStrata to them, and a timeout can reap the whole
/// tree). The cost of that isolation is that an interrupted `ost` leaves cmake,
/// ninja and their compilers running: nothing else is listening. So `ost`
/// listens, and kills the tree it started before it goes.
///
/// The target lease is deliberately *not* released here. A configure killed
/// mid-write leaves a build tree shaped by a run that never finished, which is
/// exactly what takeover evidence exists to announce — only a timeout OpenStrata
/// handled to completion clears its own lease.
mod interrupt {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Once;

    /// The pid of the managed child currently running, or 0 for none. Also the
    /// group id: children are spawned as their own group leader.
    static ACTIVE: AtomicU32 = AtomicU32::new(0);
    static ARMED: Once = Once::new();

    /// Install the interrupt handler once per process.
    pub(super) fn arm() {
        ARMED.call_once(install);
    }

    /// Record the child an interrupt should terminate.
    pub(super) fn set_active(pid: u32) {
        ACTIVE.store(pid, Ordering::SeqCst);
    }

    /// Forget the child: it has exited (or been reaped) and its pid may be
    /// reused by an unrelated process.
    pub(super) fn clear() {
        ACTIVE.store(0, Ordering::SeqCst);
    }

    fn active() -> u32 {
        ACTIVE.load(Ordering::SeqCst)
    }

    #[cfg(unix)]
    fn install() {
        // SAFETY: `on_signal` is async-signal-safe — it calls only `kill`,
        // `write` and `_exit`.
        unsafe {
            signal(SIGINT, on_signal);
            signal(SIGTERM, on_signal);
        }
    }

    #[cfg(unix)]
    const SIGINT: std::os::raw::c_int = 2;
    #[cfg(unix)]
    const SIGTERM: std::os::raw::c_int = 15;
    #[cfg(unix)]
    const SIGKILL: std::os::raw::c_int = 9;

    #[cfg(unix)]
    unsafe extern "C" {
        fn signal(
            signum: std::os::raw::c_int,
            handler: extern "C" fn(std::os::raw::c_int),
        ) -> usize;
        fn kill(pid: std::os::raw::c_int, sig: std::os::raw::c_int) -> std::os::raw::c_int;
        fn write(fd: std::os::raw::c_int, buf: *const std::os::raw::c_void, count: usize) -> isize;
        fn _exit(code: std::os::raw::c_int) -> !;
    }

    #[cfg(unix)]
    extern "C" fn on_signal(sig: std::os::raw::c_int) {
        const MESSAGE: &[u8] = b"\ninterrupted: terminating the managed process tree\n";
        let pid = active();
        if pid != 0 {
            if let Ok(pid) = std::os::raw::c_int::try_from(pid) {
                // SAFETY: async-signal-safe; a negative pid signals the group.
                unsafe {
                    write(2, MESSAGE.as_ptr().cast(), MESSAGE.len());
                    kill(-pid, SIGKILL);
                }
            }
        }
        // SAFETY: the handler must not return — the default disposition was
        // replaced, so returning would resume a run whose child is now dead.
        unsafe { _exit(128 + sig) }
    }

    #[cfg(windows)]
    fn install() {
        // SAFETY: a plain FFI registration; the handler is a valid `extern
        // "system"` fn for the lifetime of the process.
        unsafe {
            SetConsoleCtrlHandler(Some(on_console_ctrl), 1);
        }
    }

    #[cfg(windows)]
    unsafe extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    /// Windows runs console control handlers on a dedicated thread rather than
    /// in a signal context, so this may allocate and spawn. Returning 0 lets the
    /// default handler terminate `ost` — after the tree is gone.
    #[cfg(windows)]
    unsafe extern "system" fn on_console_ctrl(_event: u32) -> i32 {
        let pid = active();
        if pid != 0 {
            eprintln!("\ninterrupted: terminating the managed process tree (pid {pid})");
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        0
    }

    #[cfg(not(any(unix, windows)))]
    fn install() {}
}

/// Which standard stream a reader forwards to.
#[derive(Clone, Copy)]
enum Sink {
    Out,
    Err,
}

/// Forward a child stream to our stdout/stderr and the log, bumping the
/// `last_output` clock on every chunk so the heartbeat knows the child is alive.
fn spawn_reader<R: Read + Send + 'static>(
    src: Option<R>,
    sink: Sink,
    last_output: Arc<Mutex<Instant>>,
    tail: Arc<Mutex<VecDeque<u8>>>,
    log: Option<Arc<Mutex<std::fs::File>>>,
    forward: bool,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let Some(mut src) = src else { return };
        let mut buf = [0u8; 8192];
        loop {
            match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut t) = last_output.lock() {
                        *t = Instant::now();
                    }
                    let chunk = &buf[..n];
                    if let Ok(mut tail) = tail.lock() {
                        for byte in chunk {
                            if tail.len() == OUTPUT_TAIL_BYTES {
                                tail.pop_front();
                            }
                            tail.push_back(*byte);
                        }
                    }
                    if forward {
                        match sink {
                            Sink::Out => {
                                let mut o = std::io::stdout();
                                let _ = o.write_all(chunk);
                                let _ = o.flush();
                            }
                            Sink::Err => {
                                let mut e = std::io::stderr();
                                let _ = e.write_all(chunk);
                                let _ = e.flush();
                            }
                        }
                    }
                    if let Some(log) = &log {
                        if let Ok(mut f) = log.lock() {
                            let _ = f.write_all(chunk);
                        }
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// The last few non-empty lines of a file, read from its end.
///
/// `None` when the file is absent or has nothing to say — a CMake log that has
/// not been created yet is not an error, it locates the stall before that log.
fn file_tail(path: &Path) -> Option<String> {
    use std::io::{Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(FILE_TAIL_BYTES);
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buf = Vec::with_capacity(FILE_TAIL_BYTES as usize);
    file.read_to_end(&mut buf).ok()?;

    let text = String::from_utf8_lossy(&buf);
    let mut lines: Vec<&str> = text
        .lines()
        // A partial first line from mid-file truncation is not worth showing.
        .skip(usize::from(from > 0))
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    let start = lines.len().saturating_sub(FILE_TAIL_LINES);
    lines.drain(..start);
    Some(lines.join("\n"))
}

fn output_tail(tail: &Arc<Mutex<VecDeque<u8>>>) -> String {
    tail.lock()
        .map(|bytes| String::from_utf8_lossy(&bytes.iter().copied().collect::<Vec<_>>()).into())
        .unwrap_or_default()
}

fn one_line(value: &str) -> String {
    value
        .replace('\r', "")
        .replace('\n', " ⏎ ")
        .trim()
        .to_string()
}

fn render_command(program: &Path, args: &[String]) -> String {
    std::iter::once(program.display().to_string())
        .chain(args.iter().cloned())
        .map(|part| {
            if part.contains(' ') {
                format!("\"{part}\"")
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(unix)]
fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn configure_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}

#[cfg(not(any(unix, windows)))]
fn configure_process_group(_command: &mut Command) {}

#[cfg(windows)]
fn terminate_process_tree(child: &mut std::process::Child) -> String {
    let pid = child.id().to_string();
    let result = Command::new("taskkill")
        .args(["/PID", &pid, "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    if !matches!(&result, Ok(status) if status.success()) {
        let _ = child.kill();
    }
    let _ = child.wait();
    match result {
        Ok(status) if status.success() => "process tree terminated".into(),
        Ok(status) => format!("taskkill exited {}", status.code().unwrap_or(-1)),
        Err(error) => format!("taskkill failed: {error}"),
    }
}

#[cfg(unix)]
fn terminate_process_tree(child: &mut std::process::Child) -> String {
    let Some(group) = child_process_group(child) else {
        return terminate_child(child);
    };
    let term = signal_process_group(group, SIGTERM);
    for _ in 0..20 {
        if child.try_wait().ok().flatten().is_some() {
            return "process group terminated".into();
        }
        thread::sleep(Duration::from_millis(50));
    }
    let kill = signal_process_group(group, SIGKILL);
    if kill.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    match (term, kill) {
        (_, Ok(())) => "process group killed".into(),
        (Ok(()), _) => "process group terminated".into(),
        (_, Err(error)) => format!("process-group cleanup failed: {error}"),
    }
}

#[cfg(unix)]
const SIGTERM: std::os::raw::c_int = 15;

#[cfg(unix)]
const SIGKILL: std::os::raw::c_int = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn getpgrp() -> std::os::raw::c_int;
    fn getpgid(pid: std::os::raw::c_int) -> std::os::raw::c_int;
    fn kill(pid: std::os::raw::c_int, sig: std::os::raw::c_int) -> std::os::raw::c_int;
}

#[cfg(unix)]
fn child_process_group(child: &std::process::Child) -> Option<std::os::raw::c_int> {
    let pid: std::os::raw::c_int = child.id().try_into().ok()?;
    let group = unsafe { getpgid(pid) };
    let current_group = unsafe { getpgrp() };
    (group > 0 && group == pid && group != current_group).then_some(group)
}

#[cfg(unix)]
fn signal_process_group(
    group: std::os::raw::c_int,
    signal: std::os::raw::c_int,
) -> std::io::Result<()> {
    if unsafe { kill(-group, signal) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn terminate_child(child: &mut std::process::Child) -> String {
    match child.kill() {
        Ok(()) => {
            let _ = child.wait();
            "child terminated".into()
        }
        Err(error) => format!("child cleanup failed: {error}"),
    }
}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree(child: &mut std::process::Child) -> String {
    match child.kill() {
        Ok(()) => {
            let _ = child.wait();
            "child terminated".into()
        }
        Err(error) => format!("child cleanup failed: {error}"),
    }
}

/// Emit one JSON event as a single line on stdout (JSON Lines).
fn emit_json(value: serde_json::Value) {
    if let Ok(line) = serde_json::to_string(&value) {
        println!("{line}");
    }
}

/// Seconds since the Unix epoch, for plain-mode timestamps.
fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Render a duration as `mm:ss` (or `h:mm:ss` past an hour).
fn hms(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// A stable, greppable slug for a phase name: lowercase, non-alnum runs → `-`.
fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_lowercase_and_dashed() {
        assert_eq!(slug("Configuring CMake"), "configuring-cmake");
        assert_eq!(slug("Building targets"), "building-targets");
        assert_eq!(slug("  Verify / outputs  "), "verify-outputs");
    }

    #[test]
    fn hms_formats_minutes_and_hours() {
        assert_eq!(hms(Duration::from_secs(142)), "02:22");
        assert_eq!(hms(Duration::from_secs(5)), "00:05");
        assert_eq!(hms(Duration::from_secs(3661)), "1:01:01");
    }

    #[test]
    fn plain_mode_is_forced_regardless_of_tty() {
        let r = Reporter::new(ProgressMode::Plain, 3, false);
        assert!(matches!(r.style, Style::Plain));
    }

    #[test]
    fn json_mode_selects_the_stream_style() {
        let r = Reporter::new(ProgressMode::Json, 3, false);
        assert!(matches!(r.style, Style::Json));
    }

    #[test]
    fn notify_stays_off_until_requested() {
        // Default: no notification even if the environment would allow one.
        let r = Reporter::new(ProgressMode::Auto, 1, false);
        assert!(!r.notify);
        // Requested but environment-gated: never on under SSH / CI.
        let gated = Reporter::new(ProgressMode::Auto, 1, false).with_notify(true, "ost build");
        assert_eq!(gated.notify, notify::enabled());
    }

    /// A child that produces no output at all: the stall case, in miniature.
    fn silent_child() -> (PathBuf, Vec<String>) {
        if cfg!(windows) {
            (
                PathBuf::from("cmd"),
                vec!["/c".into(), "ping -n 4 127.0.0.1 > nul".into()],
            )
        } else {
            (PathBuf::from("/bin/sleep"), vec!["3".into()])
        }
    }

    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ost-progress-{tag}-{}-{}",
            std::process::id(),
            now_unix()
        ))
    }

    #[test]
    fn file_tail_reports_the_last_lines_only() {
        let path = temp_path("tail");
        std::fs::write(&path, "one\ntwo\nthree\nfour\nfive\n").unwrap();
        assert_eq!(file_tail(&path).unwrap(), "three\nfour\nfive");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_tail_skips_a_line_truncated_by_the_read_window() {
        let path = temp_path("tail-window");
        // Longer than FILE_TAIL_BYTES, so the read starts mid-line.
        let mut body = "x".repeat(FILE_TAIL_BYTES as usize);
        body.push_str("\nlast line\n");
        std::fs::write(&path, body).unwrap();
        assert_eq!(file_tail(&path).unwrap(), "last line");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn file_tail_is_none_when_absent_or_empty() {
        assert!(file_tail(&temp_path("absent")).is_none());
        let path = temp_path("empty");
        std::fs::write(&path, "\n\n  \n").unwrap();
        assert!(file_tail(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_new_phase_forgets_the_previous_phase_diagnostics() {
        let mut reporter = Reporter::new(ProgressMode::Plain, 2, true);
        reporter.phase("Configuring CMake");
        reporter.watch(vec![PathBuf::from("CMakeFiles/CMakeError.log")]);
        assert_eq!(reporter.diagnostics.len(), 1);
        reporter.phase("Building targets");
        assert!(
            reporter.diagnostics.is_empty(),
            "a build stall must not tail the configure logs"
        );
    }

    /// hdMerlin report 10: a managed configure that stalls has told us nothing
    /// on its pipe, so the retained CMake log is the evidence that remains.
    #[test]
    fn timeout_error_names_the_watched_diagnostic_files() {
        let cwd = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap();
        let (program, args) = silent_child();
        let cmake_log = temp_path("cmake-error-log");
        std::fs::write(
            &cmake_log,
            "Detecting CXX compiler ABI info\nchecking whether the CXX compiler works\n",
        )
        .unwrap();

        let mut reporter = Reporter::new(ProgressMode::Plain, 1, true);
        reporter.phase("Configuring CMake");
        reporter.watch(vec![cmake_log.clone()]);
        let error = reporter
            .run_status(&program, &args, &cwd, &[], Some(Duration::from_millis(100)))
            .unwrap_err();

        let message = error.to_string();
        assert!(
            message.contains("checking whether the CXX compiler works"),
            "the timeout must carry the retained log tail:\n{message}"
        );
        assert!(
            message.contains(&cmake_log.display().to_string()),
            "and name the file it came from:\n{message}"
        );
        let _ = std::fs::remove_file(&cmake_log);
    }

    /// A silent child must be reported as stalled and still be allowed to
    /// finish: a stall is a diagnostic, not a deadline.
    #[test]
    fn a_stalled_child_is_reported_and_still_completes() {
        let cwd = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap();
        let (program, args) = silent_child();
        let mut reporter =
            Reporter::new(ProgressMode::Plain, 1, true).stall_after(Duration::from_millis(200));
        reporter.phase("Configuring CMake");
        let status = reporter
            .run_status(&program, &args, &cwd, &[], None)
            .expect("a stall must not fail the run");
        assert!(
            status.success(),
            "the child should exit cleanly: {status:?}"
        );
    }

    #[test]
    fn timeout_returns_attributed_external_tool_error() {
        let cwd = camino::Utf8PathBuf::from_path_buf(std::env::current_dir().unwrap()).unwrap();
        let (program, args): (PathBuf, Vec<String>) = if cfg!(windows) {
            (
                std::env::var_os("SystemRoot")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
                    .join("System32")
                    .join("ping.exe"),
                vec!["-n".into(), "30".into(), "127.0.0.1".into()],
            )
        } else {
            (PathBuf::from("/bin/sleep"), vec!["30".into()])
        };
        let mut reporter = Reporter::new(ProgressMode::Plain, 1, true);
        reporter.phase("Timeout fixture");
        let log = camino::Utf8PathBuf::from_path_buf(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "ost-timeout-fixture-{}-{}.log",
                std::process::id(),
                now_unix()
            ));
        reporter.set_log(&log);
        let started = Instant::now();
        let error = reporter
            .run_status(&program, &args, &cwd, &[], Some(Duration::from_millis(100)))
            .unwrap_err();
        assert_eq!(error.category(), ost_core::Category::ExternalTool);
        assert_eq!(error.phase(), Some("timeout-fixture"));
        let message = error.to_string();
        assert!(message.contains("timed out"));
        assert!(message.contains("phase 'timeout-fixture'"), "{message}");
        assert!(message.contains("pid "), "{message}");
        assert!(message.contains("log '"), "{message}");
        assert!(message.contains("cleanup:"), "{message}");
        assert!(message.contains("last output:"), "{message}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_file(log);
    }
}
