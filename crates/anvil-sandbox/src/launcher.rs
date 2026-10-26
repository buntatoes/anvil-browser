// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Starting and watching a renderer process.
//!
//! [`RendererProcess`] starts a renderer, talks to it over its stdin and
//! stdout in the [`wire`](crate::wire) format, and makes sure that whatever
//! the renderer does, the browser keeps going:
//!
//! - It starts with an empty environment (only the names you keep), its
//!   working folder set to the filesystem root on Linux, and no inherited
//!   stdin, stdout or stderr.
//! - It has to say hello in the right protocol version within the startup
//!   time, or it is killed.
//! - Every request has a deadline. A renderer that misses it is killed.
//! - On Linux its memory is checked while the browser waits, and a renderer
//!   over the cap is killed. Windows relies on the job object for this.
//! - Any frame that breaks the wire rules, answers the wrong request, or
//!   names another tab or navigation gets the renderer killed.
//! - A renderer that exits or crashes is reported with its exit code or
//!   signal. Its document is never part of the message.
//! - Its stderr is drained so it can't block, and only the first few KiB are
//!   kept.
//! - Once anything has gone wrong the handle is dead, and every later call
//!   says so. Dropping the handle ends the process.
//!
//! This module only uses safe std calls. It limits what a renderer can do
//! to the browser. It does not limit what a renderer can do to the rest of
//! the machine. That needs OS confinement, which is a separate step.

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::wire::{
    self, Direction, FetchReply, FetchRequest, Message, RenderFailure, RenderRequest,
    RenderedDocument, Role, WIRE_VERSION, WireError,
};

/// Limits for one renderer process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererLimits {
    /// How long the renderer has to say hello after it starts.
    pub startup: Duration,
    /// How long the renderer has for each answer. The clock starts again
    /// after each fetch the browser answers.
    pub reply: Duration,
    /// Resident memory cap, in bytes. Checked on Linux while waiting.
    pub memory_cap_bytes: u64,
    /// How often memory and liveness are checked while waiting.
    pub poll: Duration,
    /// How much of the renderer's stderr is kept.
    pub stderr_cap_bytes: usize,
    /// Most fetches one render may ask for.
    pub max_fetches_per_render: usize,
}

impl Default for RendererLimits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(5),
            reply: Duration::from_millis(crate::MANDATORY_WORKER_TIMEOUT_MS),
            memory_cap_bytes: crate::MANDATORY_TAB_MEMORY_LIMIT_BYTES as u64,
            poll: Duration::from_millis(25),
            stderr_cap_bytes: 4 * 1024,
            max_fetches_per_render: 512,
        }
    }
}

/// What to start, and what it gets from the browser's environment.
#[derive(Clone, Debug, Default)]
pub struct RendererCommand {
    program: PathBuf,
    args: Vec<String>,
    keep_env: Vec<String>,
    set_env: Vec<(String, String)>,
    working_dir: Option<PathBuf>,
}

impl RendererCommand {
    /// Run `program` with an empty environment. On Windows `SystemRoot` and
    /// `WINDIR` are kept, because system libraries need the first to load
    /// and the font loader uses the second.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        let keep_env = if cfg!(windows) {
            vec!["SystemRoot".to_string(), "WINDIR".to_string()]
        } else {
            Vec::new()
        };
        let working_dir = if cfg!(unix) {
            Some(PathBuf::from("/"))
        } else {
            None
        };
        Self {
            program: program.into(),
            args: Vec::new(),
            keep_env,
            set_env: Vec::new(),
            working_dir,
        }
    }

    /// Add one argument.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Pass the browser's value of `name` through, when it has one.
    pub fn keep_env(mut self, name: impl Into<String>) -> Self {
        self.keep_env.push(name.into());
        self
    }

    /// Set `name` to `value` in the renderer.
    pub fn env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set_env.push((name.into(), value.into()));
        self
    }

    /// Start the renderer in `dir`.
    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    fn build(&self) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in &self.keep_env {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in &self.set_env {
            command.env(name, value);
        }
        if let Some(dir) = &self.working_dir {
            command.current_dir(dir);
        }
        // Leave the window's job so the renderer can go into its own.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
            command.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
        }
        command
    }
}

/// Why a renderer is no longer usable. The text is safe to show on an
/// error page: it never holds the document or anything the renderer wrote,
/// apart from the wire error, which is a fixed message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererFailure {
    /// The process could not be started.
    Start(String),
    /// It did not say hello in time, or said it wrongly.
    Handshake(&'static str),
    /// It missed a deadline and was killed.
    TimedOut,
    /// It used more memory than allowed and was killed.
    OverMemory { resident_bytes: u64 },
    /// It exited or crashed by itself.
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    /// It sent a frame that broke the wire rules, and was killed.
    BadFrame(WireError),
    /// It sent a valid frame at the wrong time or for the wrong page, and
    /// was killed.
    Unexpected(&'static str),
    /// The handle was already dead.
    Closed,
}

impl RendererFailure {
    /// True when the renderer died on its own or was killed for misbehaving,
    /// as opposed to never starting or being used after it was closed.
    pub fn is_crash(&self) -> bool {
        !matches!(self, Self::Start(_) | Self::Closed)
    }
}

impl fmt::Display for RendererFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start(reason) => write!(f, "the renderer could not start: {reason}"),
            Self::Handshake(reason) => write!(f, "the renderer did not start properly: {reason}"),
            Self::TimedOut => write!(f, "the renderer stopped responding"),
            Self::OverMemory { resident_bytes } => write!(
                f,
                "the renderer used too much memory ({} MiB)",
                resident_bytes / (1024 * 1024)
            ),
            Self::Exited {
                code: Some(code), ..
            } => write!(f, "the renderer stopped (exit code {code})"),
            Self::Exited {
                signal: Some(signal),
                ..
            } => write!(f, "the renderer crashed (signal {signal})"),
            Self::Exited { .. } => write!(f, "the renderer stopped"),
            Self::BadFrame(err) => write!(f, "the renderer sent a bad message: {err}"),
            Self::Unexpected(what) => write!(f, "the renderer sent {what}"),
            Self::Closed => write!(f, "the renderer is closed"),
        }
    }
}

impl std::error::Error for RendererFailure {}

/// What a renderer said about one document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderOutcome {
    /// The finished document.
    Done(RenderedDocument),
    /// The renderer could not finish it, but is still healthy.
    Refused(RenderFailure),
}

type Incoming = Result<(u32, Message), WireError>;

/// One running renderer process.
pub struct RendererProcess {
    child: Child,
    to_child: Option<mpsc::Sender<Vec<u8>>>,
    from_child: mpsc::Receiver<Incoming>,
    stderr: Arc<Mutex<Vec<u8>>>,
    limits: RendererLimits,
    next_seq: u32,
    next_nonce: u64,
    dead: bool,
}

impl fmt::Debug for RendererProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RendererProcess")
            .field("pid", &self.child.id())
            .field("dead", &self.dead)
            .finish()
    }
}

impl RendererProcess {
    /// Start a renderer and wait for its hello.
    pub fn launch(
        command: &RendererCommand,
        limits: RendererLimits,
    ) -> Result<Self, RendererFailure> {
        let mut child = command
            .build()
            .spawn()
            .map_err(|err| RendererFailure::Start(err.to_string()))?;
        let (Some(mut stdin), Some(mut stdout), Some(mut stderr_pipe)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RendererFailure::Start("pipes were not set up".to_string()));
        };

        // Writer: one frame at a time, so a renderer that stops reading
        // can block this thread but never the caller.
        let (to_child, outgoing) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            use std::io::Write;
            for frame in outgoing {
                if stdin
                    .write_all(&frame)
                    .and_then(|()| stdin.flush())
                    .is_err()
                {
                    break;
                }
            }
        });

        // Reader: every frame is checked as untrusted. The first bad one
        // ends the thread.
        let (incoming, from_child) = mpsc::channel::<Incoming>();
        thread::spawn(move || {
            loop {
                let frame = wire::read_message(&mut stdout, Direction::ToBrowser);
                let stop = frame.is_err();
                if incoming.send(frame).is_err() || stop {
                    break;
                }
            }
        });

        // Stderr: drained to the end so the renderer can't block on it,
        // and only the first `stderr_cap_bytes` are kept.
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let kept = Arc::clone(&stderr);
        let cap = limits.stderr_cap_bytes;
        thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match stderr_pipe.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut buf) = kept.lock() {
                            let room = cap.saturating_sub(buf.len());
                            buf.extend_from_slice(&chunk[..n.min(room)]);
                        }
                    }
                }
            }
        });

        let mut process = Self {
            child,
            to_child: Some(to_child),
            from_child,
            stderr,
            limits,
            next_seq: 0,
            next_nonce: 1,
            dead: false,
        };
        process.handshake()?;
        Ok(process)
    }

    fn handshake(&mut self) -> Result<(), RendererFailure> {
        let seq = self.send(&Message::Hello {
            role: Role::Browser,
            version: WIRE_VERSION,
        })?;
        let deadline = Instant::now() + self.limits.startup;
        let reply = match self.wait(deadline) {
            Ok(reply) => reply,
            Err(RendererFailure::TimedOut) => {
                return Err(RendererFailure::Handshake("no hello in time"));
            }
            Err(other) => return Err(other),
        };
        match reply {
            (s, Message::Hello { role, version })
                if s == seq && role == Role::Renderer && version == WIRE_VERSION =>
            {
                Ok(())
            }
            (_, Message::Hello { version, .. }) if version != WIRE_VERSION => {
                Err(self.fail(RendererFailure::Handshake("another protocol version")))
            }
            _ => Err(self.fail(RendererFailure::Handshake("a wrong hello"))),
        }
    }

    /// Process id, for the task list and for tests.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// True until something has gone wrong or the process has exited.
    pub fn is_alive(&mut self) -> bool {
        !self.dead && matches!(self.child.try_wait(), Ok(None))
    }

    /// The first part of what the renderer wrote to stderr. This is for
    /// logs only. It comes from the renderer, so don't show it on a page.
    pub fn stderr_text(&self) -> String {
        self.stderr
            .lock()
            .map(|buf| String::from_utf8_lossy(&buf).into_owned())
            .unwrap_or_default()
    }

    /// Check that the renderer still answers within the reply time.
    pub fn ping(&mut self) -> Result<(), RendererFailure> {
        let nonce = self.next_nonce;
        self.next_nonce = self.next_nonce.wrapping_add(1);
        let seq = self.send(&Message::Ping { nonce })?;
        let deadline = Instant::now() + self.limits.reply;
        match self.wait(deadline)? {
            (s, Message::Pong { nonce: n }) if s == seq && n == nonce => Ok(()),
            _ => Err(self.fail(RendererFailure::Unexpected("a wrong answer to a ping"))),
        }
    }

    /// Send one document and wait for the renderer's answer.
    ///
    /// While it works, the renderer may ask for subresources. Each one goes
    /// to `fetch`, which is where the browser applies its own rules, and the
    /// answer goes back to the renderer. The deadline starts again after
    /// each fetch.
    pub fn render(
        &mut self,
        request: &RenderRequest,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
    ) -> Result<RenderOutcome, RendererFailure> {
        let seq = self.send(&Message::Render(request.clone()))?;
        let mut deadline = Instant::now() + self.limits.reply;
        let mut fetches = 0usize;
        loop {
            let (s, message) = self.wait(deadline)?;
            if s != seq {
                return Err(self.fail(RendererFailure::Unexpected("an answer to another request")));
            }
            match message {
                Message::Rendered(doc) => {
                    if doc.tab_id != request.tab_id {
                        return Err(
                            self.fail(RendererFailure::Unexpected("a page for another tab"))
                        );
                    }
                    if doc.nav_id != request.nav_id {
                        return Err(
                            self.fail(RendererFailure::Unexpected("a page for another navigation"))
                        );
                    }
                    return Ok(RenderOutcome::Done(doc));
                }
                Message::Failed(failure) => {
                    if failure.tab_id != request.tab_id || failure.nav_id != request.nav_id {
                        return Err(
                            self.fail(RendererFailure::Unexpected("a failure for another page"))
                        );
                    }
                    return Ok(RenderOutcome::Refused(failure));
                }
                Message::Fetch(want) => {
                    if want.nav_id != request.nav_id {
                        return Err(self.fail(RendererFailure::Unexpected(
                            "a fetch for another navigation",
                        )));
                    }
                    fetches += 1;
                    if fetches > self.limits.max_fetches_per_render {
                        return Err(self.fail(RendererFailure::Unexpected("too many fetches")));
                    }
                    let mut reply = fetch(&want);
                    reply.nav_id = want.nav_id;
                    let frame = wire::encode(seq, &Message::FetchReply(reply)).or_else(|_| {
                        // The browser's own answer broke a cap, for example a
                        // body over 8 MiB. Send a failed fetch instead.
                        let empty = FetchReply {
                            nav_id: want.nav_id,
                            status: 0,
                            content_type: String::new(),
                            body: Vec::new(),
                        };
                        wire::encode(seq, &Message::FetchReply(empty))
                    });
                    let frame = frame.map_err(RendererFailure::BadFrame)?;
                    self.send_frame(frame)?;
                    deadline = Instant::now() + self.limits.reply;
                }
                _ => {
                    return Err(self.fail(RendererFailure::Unexpected("a message out of turn")));
                }
            }
        }
    }

    /// Ask the renderer to exit, wait briefly, then make sure it is gone.
    pub fn shutdown(mut self) {
        self.close(Duration::from_millis(500));
    }

    fn close(&mut self, grace: Duration) {
        if !self.dead {
            let seq = self.next_seq;
            if let Ok(frame) = wire::encode(seq, &Message::Shutdown) {
                if let Some(tx) = &self.to_child {
                    let _ = tx.send(frame);
                }
            }
        }
        // Dropping the sender closes stdin, which also ends a renderer that
        // missed the shutdown frame.
        self.to_child = None;
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                self.dead = true;
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.kill();
    }

    fn send(&mut self, message: &Message) -> Result<u32, RendererFailure> {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.send_with_seq(seq, message)?;
        Ok(seq)
    }

    fn send_with_seq(&mut self, seq: u32, message: &Message) -> Result<(), RendererFailure> {
        let frame = wire::encode(seq, message).map_err(RendererFailure::BadFrame)?;
        self.send_frame(frame)
    }

    fn send_frame(&mut self, frame: Vec<u8>) -> Result<(), RendererFailure> {
        if self.dead {
            return Err(RendererFailure::Closed);
        }
        let sent = self
            .to_child
            .as_ref()
            .is_some_and(|tx| tx.send(frame).is_ok());
        if sent {
            Ok(())
        } else {
            let failure = self.exit_failure();
            Err(self.fail(failure))
        }
    }

    /// Wait for the next frame until `deadline`, checking memory and
    /// liveness on the way.
    fn wait(&mut self, deadline: Instant) -> Result<(u32, Message), RendererFailure> {
        if self.dead {
            return Err(RendererFailure::Closed);
        }
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(self.fail(RendererFailure::TimedOut));
            }
            let step = (deadline - now).min(self.limits.poll);
            match self.from_child.recv_timeout(step) {
                Ok(Ok(frame)) => return Ok(frame),
                Ok(Err(WireError::Io(_) | WireError::Truncated))
                | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // The pipe closed. Most likely the process is gone.
                    let failure = self.exit_failure();
                    return Err(self.fail(failure));
                }
                Ok(Err(err)) => return Err(self.fail(RendererFailure::BadFrame(err))),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if let Some(resident) = resident_bytes(self.child.id()) {
                        if resident > self.limits.memory_cap_bytes {
                            return Err(self.fail(RendererFailure::OverMemory {
                                resident_bytes: resident,
                            }));
                        }
                    }
                    if let Ok(Some(status)) = self.child.try_wait() {
                        // It exited. A last frame may still be in the pipe.
                        if let Ok(Ok(frame)) = self.from_child.recv_timeout(self.limits.poll) {
                            return Ok(frame);
                        }
                        return Err(self.fail(exited(status)));
                    }
                }
            }
        }
    }

    /// How the process ended, waiting briefly for it to finish exiting.
    fn exit_failure(&mut self) -> RendererFailure {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return exited(status),
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
                _ => return RendererFailure::Unexpected("a closed pipe"),
            }
        }
    }

    /// Kill the process, mark the handle dead, and hand back `failure`.
    fn fail(&mut self, failure: RendererFailure) -> RendererFailure {
        self.kill();
        failure
    }

    fn kill(&mut self) {
        self.dead = true;
        self.to_child = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for RendererProcess {
    fn drop(&mut self) {
        self.close(Duration::from_millis(200));
    }
}

fn exited(status: ExitStatus) -> RendererFailure {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    RendererFailure::Exited {
        code: status.code(),
        signal,
    }
}

/// Resident memory of process `pid`, in bytes, where the OS lets us read
/// it with plain file reads. Linux only for now.
pub fn resident_bytes(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        parse_vm_rss(&status)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// Read the `VmRSS:` line of a Linux `/proc/<pid>/status` file, in bytes.
pub fn parse_vm_rss(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let mut parts = line["VmRSS:".len()..].split_whitespace();
    let value: u64 = parts.next()?.parse().ok()?;
    match parts.next() {
        Some("kB") => value.checked_mul(1024),
        _ => None,
    }
}

/// Stops a renderer that keeps crashing from being started over and over.
///
/// Record each crash with [`RelaunchBudget::record_crash`]. Once `max`
/// crashes fall inside `window`, it says no until the oldest one ages out,
/// and the tab should show its error page instead of trying again.
#[derive(Clone, Debug)]
pub struct RelaunchBudget {
    max: usize,
    window: Duration,
    crashes: VecDeque<Instant>,
}

impl RelaunchBudget {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            crashes: VecDeque::new(),
        }
    }

    /// Note a crash at `now` and say whether starting again is allowed.
    pub fn record_crash(&mut self, now: Instant) -> bool {
        self.forget_old(now);
        self.crashes.push_back(now);
        self.crashes.len() < self.max
    }

    /// True when a new start is allowed at `now`.
    pub fn may_launch(&mut self, now: Instant) -> bool {
        self.forget_old(now);
        self.crashes.len() < self.max
    }

    fn forget_old(&mut self, now: Instant) {
        while let Some(first) = self.crashes.front() {
            if now.saturating_duration_since(*first) >= self.window {
                self.crashes.pop_front();
            } else {
                break;
            }
        }
    }
}

impl Default for RelaunchBudget {
    /// Three crashes in a minute.
    fn default() -> Self {
        Self::new(3, Duration::from_secs(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_resident_memory_from_proc_status() {
        let status = "Name:\tanvil\nVmPeak:\t  999 kB\nVmRSS:\t   2048 kB\nThreads:\t4\n";
        assert_eq!(parse_vm_rss(status), Some(2048 * 1024));
        assert_eq!(parse_vm_rss("VmRSS:\t12 MB\n"), None);
        assert_eq!(parse_vm_rss("VmRSS:\tlots kB\n"), None);
        assert_eq!(parse_vm_rss("Name:\tx\n"), None);
        assert_eq!(parse_vm_rss("VmRSS:\t18446744073709551615 kB\n"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_this_process_memory() {
        let resident = resident_bytes(std::process::id()).expect("VmRSS");
        assert!(resident > 0);
    }

    #[test]
    fn relaunch_budget_stops_a_crash_loop() {
        let start = Instant::now();
        let mut budget = RelaunchBudget::new(3, Duration::from_secs(60));
        assert!(budget.may_launch(start));
        assert!(budget.record_crash(start));
        assert!(budget.record_crash(start + Duration::from_secs(1)));
        assert!(!budget.record_crash(start + Duration::from_secs(2)));
        assert!(!budget.may_launch(start + Duration::from_secs(30)));
        // The first crash ages out after a minute.
        assert!(budget.may_launch(start + Duration::from_secs(60)));
    }

    #[test]
    fn failure_text_never_needs_the_document() {
        let failures = [
            RendererFailure::TimedOut,
            RendererFailure::OverMemory {
                resident_bytes: 300 * 1024 * 1024,
            },
            RendererFailure::Exited {
                code: Some(7),
                signal: None,
            },
            RendererFailure::Exited {
                code: None,
                signal: Some(11),
            },
            RendererFailure::BadFrame(WireError::BadMagic),
            RendererFailure::Unexpected("a page for another tab"),
        ];
        for failure in failures {
            assert!(failure.is_crash());
            assert!(failure.to_string().starts_with("the renderer"));
        }
        assert!(!RendererFailure::Closed.is_crash());
        assert_eq!(
            RendererFailure::OverMemory {
                resident_bytes: 300 * 1024 * 1024
            }
            .to_string(),
            "the renderer used too much memory (300 MiB)"
        );
    }

    #[test]
    fn the_command_starts_with_an_empty_environment() {
        let command = RendererCommand::new("renderer")
            .arg("--renderer")
            .env("ANVIL_MODE", "test");
        let built = command.build();
        let envs: Vec<_> = built.get_envs().collect();
        assert!(
            envs.iter()
                .any(|(k, v)| *k == "ANVIL_MODE" && v.is_some_and(|v| v == "test"))
        );
        #[cfg(unix)]
        assert_eq!(built.get_current_dir(), Some(std::path::Path::new("/")));
    }
}
