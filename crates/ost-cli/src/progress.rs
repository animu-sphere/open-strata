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
//!
//! We never invent a percentage: progress is reported as *phases* plus elapsed
//! time, with a heartbeat so a quiet child process never looks hung. Child
//! stdout/stderr is passed through (or, with `--quiet`, captured to the log only)
//! and always teed to the per-target log so failures point at a real file.
//!
//! `--progress json` and OS `--notify` are intentionally not here yet; the event
//! model above is shaped so they slot in without disturbing callers.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use camino::Utf8Path;
use clap::ValueEnum;

use ost_core::{Error, Result};

/// How progress is rendered. `auto` picks Human on a TTY, Plain otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum ProgressMode {
    /// Human on a terminal, plain key=value lines when piped / in CI.
    Auto,
    /// Always emit plain `phase=… status=…` lines (good for CI logs).
    Plain,
}

/// The resolved rendering style (after `auto` detection).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Human,
    Plain,
}

/// Idle time with no child output before a heartbeat is emitted.
const HEARTBEAT: Duration = Duration::from_secs(15);

struct PhaseState {
    name: String,
    slug: String,
    started: Instant,
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
    current: Option<PhaseState>,
}

impl Reporter {
    /// Create a reporter for a command with `total` phases. `quiet` suppresses
    /// progress chatter and child passthrough, but never failure reporting.
    pub fn new(mode: ProgressMode, total: usize, quiet: bool) -> Reporter {
        let style = match mode {
            ProgressMode::Plain => Style::Plain,
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
            current: None,
        }
    }

    /// Tee child output to (and report) this log file. Created on first write;
    /// a failure to open it is non-fatal (logging is best-effort).
    pub fn set_log(&mut self, path: &Utf8Path) {
        self.log = Some(path.as_std_path().to_path_buf());
    }

    /// Begin a new phase, closing the previous one as completed.
    pub fn phase(&mut self, name: &str) {
        self.close_current(Outcome::Completed);
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
            }
        }
        self.current = Some(state);
    }

    /// Close the final phase and print the total wall time.
    pub fn done(&mut self) {
        self.close_current(Outcome::Completed);
        if self.quiet {
            return;
        }
        match self.style {
            Style::Human => println!("completed in {}", hms(self.started.elapsed())),
            Style::Plain => println!(
                "timestamp={} phase=all status=completed duration_ms={}",
                now_unix(),
                self.started.elapsed().as_millis()
            ),
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
                }
            }
            // Failures always surface (even under --quiet), naming the phase,
            // the exit code (if any) and the log path.
            Outcome::Failed(exit) => {
                let code = exit.map(|c| c.to_string());
                match self.style {
                    Style::Human => {
                        let exit = code
                            .map(|c| format!("exit {c}, "))
                            .unwrap_or_default();
                        eprintln!(
                            "[{}/{}] {} FAILED ({exit}after {})",
                            self.index,
                            self.total,
                            state.name,
                            hms(dur)
                        );
                        if let Some(log) = &self.log {
                            eprintln!("      log: {}", log.display());
                        }
                    }
                    Style::Plain => {
                        let exit = code
                            .map(|c| format!(" exit_code={c}"))
                            .unwrap_or_default();
                        eprintln!(
                            "timestamp={} phase={} status=failed{exit} duration_ms={}",
                            now_unix(),
                            state.slug,
                            dur.as_millis()
                        );
                        if let Some(log) = &self.log {
                            eprintln!(
                                "timestamp={} phase={} status=failed log={}",
                                now_unix(),
                                state.slug,
                                log.display()
                            );
                        }
                    }
                }
            }
        }
    }

    /// Emit an idle heartbeat for the running phase (no child output for a while).
    fn heartbeat(&self, idle: Duration) {
        if self.quiet {
            return;
        }
        let Some(state) = &self.current else { return };
        let elapsed = state.started.elapsed();
        match self.style {
            Style::Human => println!(
                "[{}/{}] {} … elapsed {} (waiting on output)",
                self.index,
                self.total,
                state.name,
                hms(elapsed)
            ),
            Style::Plain => println!(
                "timestamp={} phase={} status=running elapsed_ms={} last_output_ms={}",
                now_unix(),
                state.slug,
                elapsed.as_millis(),
                idle.as_millis()
            ),
        }
    }

    /// Run a child process under the current phase: stream its output through
    /// (teeing to the log), emit a heartbeat while it is quiet, and on failure
    /// report the phase + exit code + log path before propagating.
    pub fn run(
        &mut self,
        program: &Path,
        args: &[String],
        cwd: &Utf8Path,
        env: &[(String, String)],
    ) -> Result<()> {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd.as_std_path())
            .envs(env.iter().cloned())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::io(format!("run {}", program.display()), e))?;

        // Shared "last output" clock and an optional log sink, both updated by the
        // reader threads as bytes arrive.
        let last_output = Arc::new(Mutex::new(Instant::now()));
        let log = self.open_log();
        let forward = !self.quiet;

        let out = spawn_reader(child.stdout.take(), Sink::Out, last_output.clone(), log.clone(), forward);
        let err = spawn_reader(child.stderr.take(), Sink::Err, last_output.clone(), log.clone(), forward);

        // Poll for completion; while the child runs, emit a heartbeat whenever it
        // has produced no output for HEARTBEAT.
        let mut last_beat = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    thread::sleep(Duration::from_millis(200));
                    let idle = last_output.lock().map(|t| t.elapsed()).unwrap_or_default();
                    if idle >= HEARTBEAT && last_beat.elapsed() >= HEARTBEAT {
                        self.heartbeat(idle);
                        last_beat = Instant::now();
                    }
                }
                Err(e) => return Err(Error::io(format!("wait {}", program.display()), e)),
            }
        };

        let _ = out.join();
        let _ = err.join();

        if !status.success() {
            self.close_current(Outcome::Failed(status.code()));
            // Preserve the child's exit code for CI rather than collapsing to 1.
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
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
    /// did not pass through [`run`](Reporter::run) (which reports the failure and
    /// exits the process itself) — e.g. a failed `generate`/`verify` phase. Emit
    /// its terminal `failed` line so every `started` has a matching end, even in
    /// plain/CI output. A clean run leaves no open phase (`done`/`phase` close
    /// it), so this is a no-op there.
    fn drop(&mut self) {
        self.close_current(Outcome::Failed(None));
    }
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
}
