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
//!   A page also has a time limit for the whole of its work, fetches
//!   included, so a renderer can't keep a tab busy by asking for one slow
//!   fetch after another.
//! - On Linux its memory is checked while the browser waits, and a renderer
//!   over the cap is killed. Windows relies on the job object for this.
//! - Between requests, [`RendererProcess::heartbeat`] pings it and, on
//!   Linux, checks that it hasn't used the CPU while it had nothing to do.
//!   A renderer that has been taken over can't spin in the background.
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
//! the machine. [`confine`](crate::confine) does that where the system has
//! a tool for it, by starting the renderer through the tool. The launcher
//! then watches the renderer below the tool, and killing the tool ends the
//! renderer too.

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::fmt;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use crate::wire::{
    self, Direction, FetchReply, FetchRequest, Message, RenderFailure, RenderRequest,
    RenderedDocument, RenderedFrame, RendererStatus, Role, WIRE_VERSION, WireError,
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
    /// Longest one page may take, from the request to the answer, with the
    /// browser's time spent on fetches counted too.
    pub page_total: Duration,
    /// Most CPU time the renderer may use, in total, between finishing one
    /// page and being given the next. Pings don't reset it. Checked on Linux
    /// by [`RendererProcess::heartbeat`].
    pub idle_cpu: Duration,
}

impl Default for RendererLimits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(5),
            reply: Duration::from_millis(crate::MANDATORY_WORKER_TIMEOUT_MS),
            memory_cap_bytes: crate::MANDATORY_TAB_MEMORY_LIMIT_BYTES as u64,
            poll: Duration::from_millis(25),
            stderr_cap_bytes: 4 * 1024,
            max_fetches_per_render: wire::MAX_FETCHES_PER_RENDER,
            page_total: Duration::from_secs(60),
            idle_cpu: Duration::from_secs(1),
        }
    }
}

/// What to start, and what it gets from the browser's environment.
#[derive(Clone, Debug, Default)]
pub struct RendererCommand {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    keep_env: Vec<String>,
    set_env: Vec<(String, String)>,
    working_dir: Option<PathBuf>,
    pub(crate) wrapper: Option<Wrapping>,
}

/// A helper program the renderer is started through, such as the one
/// [`confine`](crate::confine) picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Wrapping {
    /// The helper, and what goes before the renderer's own command line.
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    /// The helper reports a renderer killed by signal N as exit code
    /// 128 + N.
    pub(crate) signal_as_exit_code: bool,
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
            wrapper: None,
        }
    }

    /// The same command, wrapper and environment rules included, with
    /// `program` and `args` in place of the renderer's. Tests use this to
    /// run a check the way a renderer would be started. Under `bwrap` only
    /// the renderer and `/usr` are in view, so `program` has to be under
    /// `/usr`.
    pub fn with_program<I, S>(mut self, program: impl Into<PathBuf>, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.program = program.into();
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    /// The program this runs, before any wrapping.
    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

    /// The renderer's arguments, before any wrapping.
    pub fn args(&self) -> &[String] {
        &self.args
    }

    /// True when the renderer is started through a helper program.
    pub fn is_wrapped(&self) -> bool {
        self.wrapper.is_some()
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

    /// The std command this starts, wrapper included. The browser uses
    /// [`RendererProcess::launch`]. This is for tests and tools that want
    /// to run something else the same way.
    pub fn to_command(&self) -> Command {
        self.build()
    }

    fn build(&self) -> Command {
        let mut command = match &self.wrapper {
            Some(wrapper) => {
                let mut command = Command::new(&wrapper.program);
                command.args(&wrapper.args).arg(&self.program);
                command
            }
            None => Command::new(&self.program),
        };
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
    /// One page took longer than [`RendererLimits::page_total`], and it was
    /// killed.
    PageTooSlow,
    /// It used more memory than allowed and was killed.
    OverMemory { resident_bytes: u64 },
    /// It kept the CPU busy between requests and was killed.
    BusyWhileIdle { cpu_ms: u64 },
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
            Self::PageTooSlow => write!(f, "the renderer took too long over this page"),
            Self::OverMemory { resident_bytes } => write!(
                f,
                "the renderer used too much memory ({} MiB)",
                resident_bytes / (1024 * 1024)
            ),
            Self::BusyWhileIdle { cpu_ms } => write!(
                f,
                "the renderer used the CPU while it had nothing to do ({cpu_ms} ms)"
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
    /// The finished document, and the frame the renderer painted for it
    /// when it sent one. The frame is not checked here.
    Done(RenderedDocument, Option<RenderedFrame>),
    /// The renderer could not finish it, but is still healthy.
    Refused(RenderFailure),
}

type Incoming = Result<(u32, Message), WireError>;

/// One running renderer process.
pub struct RendererProcess {
    child: Child,
    /// The renderer itself. Without a wrapper this is the child. With one
    /// it is found below the wrapper once the renderer has said hello.
    renderer_pid: u32,
    signal_as_exit_code: bool,
    to_child: Option<mpsc::Sender<Vec<u8>>>,
    from_child: mpsc::Receiver<Incoming>,
    stderr: Arc<Mutex<Vec<u8>>>,
    limits: RendererLimits,
    next_seq: u32,
    next_nonce: u64,
    /// CPU time after the hello or the last finished page, where the OS
    /// reports it.
    cpu_mark: Option<Duration>,
    dead: bool,
}

impl fmt::Debug for RendererProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RendererProcess")
            .field("pid", &self.renderer_pid)
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
        let spawned = if command.is_wrapped() {
            spawn_from_launcher_thread(command.build())
        } else {
            command.build().spawn()
        };
        let mut child = spawned.map_err(|err| RendererFailure::Start(err.to_string()))?;
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

        let renderer_pid = child.id();
        let mut process = Self {
            child,
            renderer_pid,
            signal_as_exit_code: command
                .wrapper
                .as_ref()
                .is_some_and(|w| w.signal_as_exit_code),
            to_child: Some(to_child),
            from_child,
            stderr,
            limits,
            next_seq: 0,
            next_nonce: 1,
            cpu_mark: None,
            dead: false,
        };
        process.handshake()?;
        if command.wrapper.is_some() {
            // It said hello, so it is running below the wrapper by now.
            process.renderer_pid = innermost_child(process.child.id());
            process.mark_cpu();
        }
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
                self.mark_cpu();
                Ok(())
            }
            (_, Message::Hello { version, .. }) if version != WIRE_VERSION => {
                Err(self.fail(RendererFailure::Handshake("another protocol version")))
            }
            _ => Err(self.fail(RendererFailure::Handshake("a wrong hello"))),
        }
    }

    /// Process id of the renderer, for the task list and for tests. When
    /// it runs through a wrapper this is the renderer, not the wrapper.
    pub fn pid(&self) -> u32 {
        self.renderer_pid
    }

    /// Process id of what was started: the wrapper when there is one,
    /// otherwise the renderer. Killing it ends the renderer too.
    pub fn started_pid(&self) -> u32 {
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

    /// Ask the renderer what the OS has put it under, read from its own
    /// `/proc/self`. It answers within the reply time or is killed.
    ///
    /// The answer comes from the renderer, so it is only worth showing when
    /// the renderer is fresh and has not been given a page yet.
    pub fn status(&mut self) -> Result<RendererStatus, RendererFailure> {
        let seq = self.send(&Message::StatusQuery)?;
        let deadline = Instant::now() + self.limits.reply;
        match self.wait(deadline)? {
            (s, Message::Status(status)) if s == seq => Ok(status),
            _ => Err(self.fail(RendererFailure::Unexpected(
                "a wrong answer to a status query",
            ))),
        }
    }

    /// The idle check, for when the renderer has nothing to do.
    ///
    /// First the CPU check: a renderer that used more than
    /// [`RendererLimits::idle_cpu`] since it said hello or last finished a
    /// page is killed. Answering pings does not reset that, so a renderer
    /// that spins in the background is caught however often it is pinged.
    /// The check needs the OS to report CPU time, which is Linux only for now.
    /// Then a [`ping`](Self::ping), so a renderer that has stopped reading
    /// is found before the next page needs it.
    pub fn heartbeat(&mut self) -> Result<(), RendererFailure> {
        if self.dead {
            return Err(RendererFailure::Closed);
        }
        if let (Some(mark), Some(now)) = (self.cpu_mark, cpu_time(self.renderer_pid)) {
            let used = now.saturating_sub(mark);
            if used > self.limits.idle_cpu {
                let cpu_ms = u64::try_from(used.as_millis()).unwrap_or(u64::MAX);
                return Err(self.fail(RendererFailure::BusyWhileIdle { cpu_ms }));
            }
        }
        self.ping()
    }

    /// Remember the CPU time now, as the start of an idle stretch.
    fn mark_cpu(&mut self) {
        self.cpu_mark = cpu_time(self.renderer_pid);
    }

    /// Send one document and wait for the renderer's answer.
    ///
    /// While it works, the renderer may ask for subresources. Each one goes
    /// to `fetch`, which is where the browser applies its own rules, and the
    /// answer goes back to the renderer. The deadline starts again after
    /// each fetch, but the whole page still has to be done within
    /// [`RendererLimits::page_total`].
    pub fn render(
        &mut self,
        request: &RenderRequest,
        fetch: &mut dyn FnMut(&FetchRequest) -> FetchReply,
    ) -> Result<RenderOutcome, RendererFailure> {
        let seq = self.send(&Message::Render(request.clone()))?;
        let page_deadline = Instant::now() + self.limits.page_total;
        let mut deadline = Instant::now() + self.limits.reply;
        let mut fetches = 0usize;
        let mut frame = None;
        loop {
            let limit = deadline.min(page_deadline);
            let (s, message) = match self.wait(limit) {
                Err(RendererFailure::TimedOut) if limit == page_deadline => {
                    return Err(RendererFailure::PageTooSlow);
                }
                other => other?,
            };
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
                    self.mark_cpu();
                    return Ok(RenderOutcome::Done(doc, frame));
                }
                Message::Frame(painted) => {
                    if painted.tab_id != request.tab_id || painted.nav_id != request.nav_id {
                        return Err(
                            self.fail(RendererFailure::Unexpected("a frame for another page"))
                        );
                    }
                    if frame.is_some() {
                        return Err(self.fail(RendererFailure::Unexpected("a second frame")));
                    }
                    frame = Some(painted);
                }
                Message::Failed(failure) => {
                    if failure.tab_id != request.tab_id || failure.nav_id != request.nav_id {
                        return Err(
                            self.fail(RendererFailure::Unexpected("a failure for another page"))
                        );
                    }
                    self.mark_cpu();
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
            if let Ok(frame) = wire::encode(seq, &Message::Shutdown)
                && let Some(tx) = &self.to_child
            {
                let _ = tx.send(frame);
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
                    if let Some(resident) = resident_bytes(self.renderer_pid)
                        && resident > self.limits.memory_cap_bytes
                    {
                        return Err(self.fail(RendererFailure::OverMemory {
                            resident_bytes: resident,
                        }));
                    }
                    if let Ok(Some(status)) = self.child.try_wait() {
                        // It exited. A last frame may still be in the pipe.
                        if let Ok(Ok(frame)) = self.from_child.recv_timeout(self.limits.poll) {
                            return Ok(frame);
                        }
                        return Err(self.fail(exited(status, self.signal_as_exit_code)));
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
                Ok(Some(status)) => return exited(status, self.signal_as_exit_code),
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

/// A process to start, and where to send the result.
type SpawnJob = (Command, mpsc::Sender<std::io::Result<Child>>);

/// Start a wrapped `command` from one thread that lives as long as the
/// browser.
///
/// A renderer started through `bwrap` or `unshare` is ended when the thread
/// that started it ends, not only when the browser does, because Linux ties
/// the parent-death signal those tools use to the starting thread. The
/// window starts a tab's renderer on the thread that loads the page, which
/// ends with the page, so wrapped renderers are started here instead. Plain
/// renderers don't need it and are started on the caller's thread as before,
/// so this thread, and the memory it keeps, only exists once a tool is in
/// use. If it can't be started, the caller's thread is used.
fn spawn_from_launcher_thread(command: Command) -> std::io::Result<Child> {
    static LAUNCHER: OnceLock<Option<Mutex<mpsc::Sender<SpawnJob>>>> = OnceLock::new();
    let launcher = LAUNCHER.get_or_init(|| {
        let (jobs, incoming) = mpsc::channel::<SpawnJob>();
        thread::Builder::new()
            .name("renderer-launcher".to_string())
            .spawn(move || {
                for (mut command, reply) in incoming {
                    let _ = reply.send(command.spawn());
                }
            })
            .ok()
            .map(|_| Mutex::new(jobs))
    });
    let (reply, result) = mpsc::channel();
    let job = (command, reply);
    let unsent = match launcher.as_ref().map(|jobs| jobs.lock()) {
        Some(Ok(jobs)) => match jobs.send(job) {
            Ok(()) => None,
            Err(mpsc::SendError(job)) => Some(job),
        },
        _ => Some(job),
    };
    match unsent {
        None => result
            .recv()
            .unwrap_or_else(|_| Err(std::io::Error::other("the launcher thread stopped"))),
        Some((mut command, _)) => command.spawn(),
    }
}

fn exited(status: ExitStatus, signal_as_exit_code: bool) -> RendererFailure {
    #[cfg(unix)]
    let signal = {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    };
    #[cfg(not(unix))]
    let signal = None;
    match status.code() {
        Some(code) if signal_as_exit_code && (129..=192).contains(&code) => {
            RendererFailure::Exited {
                code: None,
                signal: Some(code - 128),
            }
        }
        code => RendererFailure::Exited { code, signal },
    }
}

/// Follow `pid` down through processes that have exactly one child, and
/// return the last one. A wrapper and its own helper each have one child,
/// so this ends at the renderer. Where the OS doesn't list children it
/// returns `pid`.
fn innermost_child(pid: u32) -> u32 {
    let mut current = pid;
    for _ in 0..8 {
        match only_child(current) {
            Some(child) => current = child,
            None => break,
        }
    }
    current
}

fn only_child(pid: u32) -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        let list = std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children")).ok()?;
        let mut children = list.split_whitespace();
        let child = children.next()?.parse().ok()?;
        match children.next() {
            None => Some(child),
            Some(_) => None,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
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

/// CPU time process `pid` has used so far, user and system together, where
/// the OS lets us read it with plain file reads. Linux only for now.
pub fn cpu_time(pid: u32) -> Option<Duration> {
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let ticks = parse_cpu_ticks(&stat)?;
        // `/proc` counts in USER_HZ, which the kernel fixes at 100 for
        // every architecture we build for.
        Some(Duration::from_millis(ticks.checked_mul(10)?))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// `utime + stime` from a Linux `/proc/<pid>/stat` line, in clock ticks.
///
/// The process name is the second field, in parentheses, and may hold
/// spaces or parentheses itself, so the fields are counted from the last
/// `)`. After it come the state (field 3) and then fields 4 onwards, so
/// `utime` (field 14) and `stime` (field 15) are the 12th and 13th after
/// the state.
pub fn parse_cpu_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    let mut fields = rest.split_whitespace().skip(11);
    let utime: u64 = fields.next()?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    utime.checked_add(stime)
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
    fn reads_cpu_ticks_from_proc_stat() {
        let stat = "4242 (anvil) S 1 4242 4242 0 -1 4194304 500 0 0 0 \
                    37 5 0 0 20 0 3 0 100 1000000 200 18446744073709551615";
        assert_eq!(parse_cpu_ticks(stat), Some(42));
        // A name with spaces and parentheses does not shift the fields.
        let odd = "7 (a) b (c) R 1 7 7 0 -1 0 0 0 0 0 9 1 0 0 20 0 1 0";
        assert_eq!(parse_cpu_ticks(odd), Some(10));
        assert_eq!(parse_cpu_ticks("7 (short) R 1 2 3"), None);
        assert_eq!(parse_cpu_ticks("no parentheses at all"), None);
        assert_eq!(
            parse_cpu_ticks("1 (x) R 0 0 0 0 0 0 0 0 0 0 18446744073709551615 1"),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_this_process_cpu_time() {
        let start = cpu_time(std::process::id()).expect("cpu time");
        let until = std::time::Instant::now() + Duration::from_millis(100);
        let mut spin = 0u64;
        while std::time::Instant::now() < until {
            spin = std::hint::black_box(spin.wrapping_add(1));
        }
        let after = cpu_time(std::process::id()).expect("cpu time");
        assert!(after >= start);
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
            RendererFailure::BusyWhileIdle { cpu_ms: 1500 },
            RendererFailure::PageTooSlow,
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
