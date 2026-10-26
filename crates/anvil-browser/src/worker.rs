// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! `--tab-worker` processes.
//!
//! Each tab in the window has its own worker process ([`TabWorkerPool`]).
//! http(s) documents and local files are parsed, scripted, and serialized in
//! that process. The window only lays out and paints the document that comes
//! back. The worker runs under the `anvil-sandbox` worker limits, and every
//! answer is bounded by the timeout. A worker that hangs or crashes is killed
//! and the tab shows an error page. The window keeps running. Closing a tab
//! ends its worker. `anvil://` pages do not use this module.
//!
//! [`exchange_tab_worker`] is the one-document form used by the `render` and
//! `raster` commands. The session cookie jar is never part of a request.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use anvil_json::JsonValue;
use anvil_sandbox::{
    MANDATORY_WORKER_TIMEOUT_MS, TabWorkerRequest, TabWorkerResponse, encode_ipc_frame,
    read_ipc_frame,
};

/// `MANDATORY_WORKER_TIMEOUT_MS` as a `Duration`. The window uses this.
pub fn worker_timeout() -> Duration {
    Duration::from_millis(MANDATORY_WORKER_TIMEOUT_MS)
}

/// Path of the running executable. The window passes this to [`exchange_tab_worker`]
/// with the `--tab-worker` argument.
pub fn current_anvil_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|err| format!("could not find the anvil executable: {err}"))
}

/// Start `program` with `args`, send `request`, and wait up to `timeout`.
///
/// A timeout, a spawn failure, or a frame that is not `ANVILIPC` is `Err`.
/// The child is killed on timeout and on a bad frame. `Ok` is a decoded
/// response, including `ok: false`. The caller must not paint `html` unless
/// `ok` is true.
pub fn exchange_tab_worker(
    program: &Path,
    args: &[String],
    request: &TabWorkerRequest,
    timeout: Duration,
) -> Result<TabWorkerResponse, String> {
    let frame = encode_ipc_frame(&request.to_json()).map_err(|err| err.to_string())?;
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Leave the window job so the child can enter the worker job.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        command.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
    }
    let mut child = command
        .spawn()
        .map_err(|err| format!("could not start the tab worker: {err}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "tab worker stdin was not piped".to_string())?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "tab worker stdout was not piped".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "tab worker stderr was not piped".to_string())?;

    let write_thread = thread::spawn(move || {
        let result = stdin.write_all(&frame).and_then(|_| stdin.flush());
        drop(stdin);
        result
    });
    let read_thread = thread::spawn(move || read_ipc_frame(&mut stdout));
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 512];
        while buf.len() < 2048 {
            match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let room = 2048 - buf.len();
                    buf.extend_from_slice(&chunk[..n.min(room)]);
                }
            }
        }
        String::from_utf8_lossy(&buf).trim().to_string()
    });

    let deadline = Instant::now() + timeout;
    loop {
        if read_thread.is_finished() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = write_thread.join();
            let _ = read_thread.join();
            let _ = stderr_thread.join();
            return Err("tab worker timed out".to_string());
        }
        thread::sleep(Duration::from_millis(10));
    }

    let read_result = read_thread
        .join()
        .map_err(|_| "tab worker reader stopped".to_string())?;
    let _ = child.wait();
    let _ = write_thread.join();
    let stderr_text = stderr_thread.join().unwrap_or_default();
    let json = read_result.map_err(|err| {
        if stderr_text.is_empty() {
            format!("tab worker frame: {err}")
        } else {
            format!("tab worker frame: {err}; {stderr_text}")
        }
    })?;
    TabWorkerResponse::from_json(&json).map_err(|err| err.to_string())
}

/// One long-lived `--tab-worker` process that belongs to one tab.
///
/// The window sends one `ANVILIPC` request per navigation and reads one
/// response. Frames are written and read on helper threads, so a worker that
/// stops reading or stops answering cannot block the window past the timeout.
/// A worker that times out, exits, or writes a bad frame is killed and is not
/// used again. Dropping a `TabProcess` closes its stdin and ends the process.
pub struct TabProcess {
    child: Child,
    frames_out: Option<mpsc::Sender<Vec<u8>>>,
    frames_in: mpsc::Receiver<Result<JsonValue, String>>,
    served: u64,
}

impl std::fmt::Debug for TabProcess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TabProcess")
            .field("pid", &self.child.id())
            .field("served", &self.served)
            .finish()
    }
}

impl TabProcess {
    /// Start `program` with `args`. The process waits for its first frame.
    pub fn spawn(program: &Path, args: &[String]) -> Result<Self, String> {
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Leave the window job so the child can enter the worker job.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
            command.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
        }
        let mut child = command
            .spawn()
            .map_err(|err| format!("could not start the tab worker: {err}"))?;
        let Some(mut stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("tab worker stdin was not piped".to_string());
        };
        let Some(mut stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("tab worker stdout was not piped".to_string());
        };

        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        thread::spawn(move || {
            for frame in out_rx {
                if stdin.write_all(&frame).and_then(|_| stdin.flush()).is_err() {
                    break;
                }
            }
        });
        let (in_tx, in_rx) = mpsc::channel();
        thread::spawn(move || {
            loop {
                let frame = read_ipc_frame(&mut stdout).map_err(|err| err.to_string());
                let stop = frame.is_err();
                if in_tx.send(frame).is_err() || stop {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            frames_out: Some(out_tx),
            frames_in: in_rx,
            served: 0,
        })
    }

    /// Process id of the worker.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Number of documents this worker has answered.
    pub fn served(&self) -> u64 {
        self.served
    }

    /// True while the process has not exited.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Send `request` and wait up to `timeout` for the answer.
    ///
    /// Any `Err` means the worker is no longer usable. The caller drops it,
    /// which kills the process.
    pub fn exchange(
        &mut self,
        request: &TabWorkerRequest,
        timeout: Duration,
    ) -> Result<TabWorkerResponse, String> {
        let frame = encode_ipc_frame(&request.to_json()).map_err(|err| err.to_string())?;
        let sender = self
            .frames_out
            .as_ref()
            .ok_or_else(|| "tab worker is closed".to_string())?;
        if sender.send(frame).is_err() {
            return Err("the tab worker stopped".to_string());
        }
        let json = match self.frames_in.recv_timeout(timeout) {
            Ok(Ok(json)) => json,
            Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(self.exit_reason());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.kill();
                return Err("tab worker timed out".to_string());
            }
        };
        let resp = TabWorkerResponse::from_json(&json).map_err(|err| err.to_string())?;
        if resp.tab_id != request.tab_id {
            return Err("tab worker returned a different tab id".to_string());
        }
        self.served += 1;
        Ok(resp)
    }

    /// Text for a worker that closed its pipe instead of answering.
    fn exit_reason(&mut self) -> String {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    return match status.code() {
                        Some(code) => format!("the tab worker stopped (exit code {code})"),
                        None => "the tab worker stopped".to_string(),
                    };
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                _ => {
                    self.kill();
                    return "the tab worker sent a bad frame".to_string();
                }
            }
        }
    }

    /// Kill the process and reap it.
    fn kill(&mut self) {
        self.frames_out = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for TabProcess {
    /// Close stdin so the worker leaves its loop, then make sure it is gone.
    fn drop(&mut self) {
        self.frames_out = None;
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        self.kill();
    }
}

/// The program and arguments used to start a tab worker.
#[derive(Clone, Debug)]
struct WorkerCommand {
    program: Option<PathBuf>,
    args: Vec<String>,
    timeout: Duration,
}

/// One slot per tab. The inner lock keeps one navigation per tab at a time.
type TabSlot = Arc<Mutex<Option<TabProcess>>>;

/// Tab workers for one window, keyed by tab id.
///
/// A tab gets its process on its first http(s) or file navigation and keeps it
/// for later navigations. [`TabWorkerPool::close`] ends that process when the
/// tab closes. A worker that fails is dropped, and the next navigation in that
/// tab starts a fresh one. `anvil://` pages never start a worker.
#[derive(Debug)]
pub struct TabWorkerPool {
    command: Mutex<WorkerCommand>,
    slots: Mutex<HashMap<u64, TabSlot>>,
}

impl Default for TabWorkerPool {
    fn default() -> Self {
        Self::new()
    }
}

impl TabWorkerPool {
    /// Workers run this executable with `--tab-worker` and the standard timeout.
    pub fn new() -> Self {
        Self {
            command: Mutex::new(WorkerCommand {
                program: None,
                args: vec!["--tab-worker".to_string()],
                timeout: worker_timeout(),
            }),
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Use `program` and `args` for workers started from now on.
    /// Tests use this to point at the `anvil` binary or at a stand-in process.
    pub fn set_command(&self, program: &Path, args: &[String], timeout: Duration) {
        if let Ok(mut command) = self.command.lock() {
            *command = WorkerCommand {
                program: Some(program.to_path_buf()),
                args: args.to_vec(),
                timeout,
            };
        }
    }

    /// Send `request` to the worker for `request.tab_id`, starting it if needed.
    ///
    /// A failed exchange ends that worker. The error text never contains the
    /// document that was sent.
    pub fn exchange(&self, request: &TabWorkerRequest) -> Result<TabWorkerResponse, String> {
        let command = self
            .command
            .lock()
            .map_err(|_| "tab worker settings are unavailable".to_string())?
            .clone();
        let slot = self.slot(request.tab_id)?;
        let mut guard = slot
            .lock()
            .map_err(|_| "tab worker is unavailable".to_string())?;
        if guard.as_mut().is_some_and(|process| !process.is_running()) {
            *guard = None;
        }
        if guard.is_none() {
            let program = match &command.program {
                Some(program) => program.clone(),
                None => current_anvil_exe()?,
            };
            *guard = Some(TabProcess::spawn(&program, &command.args)?);
        }
        let result = match guard.as_mut() {
            Some(process) => process.exchange(request, command.timeout),
            None => Err("tab worker did not start".to_string()),
        };
        if result.is_err() {
            *guard = None;
        }
        drop(guard);
        // The tab may have closed while this page was loading.
        let still_open = self
            .slots
            .lock()
            .map(|slots| {
                slots
                    .get(&request.tab_id)
                    .is_some_and(|current| Arc::ptr_eq(current, &slot))
            })
            .unwrap_or(false);
        if !still_open {
            if let Ok(mut guard) = slot.lock() {
                *guard = None;
            }
        }
        result
    }

    /// End the worker for `tab_id`, if it has one.
    pub fn close(&self, tab_id: u64) {
        let slot = self
            .slots
            .lock()
            .ok()
            .and_then(|mut slots| slots.remove(&tab_id));
        if let Some(slot) = slot {
            // A load in progress holds the lock. It drops the worker when it finishes.
            if let Ok(mut guard) = slot.try_lock() {
                *guard = None;
            }
        }
    }

    /// End every worker.
    pub fn close_all(&self) {
        let ids: Vec<u64> = self
            .slots
            .lock()
            .map(|slots| slots.keys().copied().collect())
            .unwrap_or_default();
        for id in ids {
            self.close(id);
        }
    }

    /// Process id of the running worker for `tab_id`.
    pub fn worker_pid(&self, tab_id: u64) -> Option<u32> {
        let slot = self.slots.lock().ok()?.get(&tab_id).cloned()?;
        let mut guard = slot.try_lock().ok()?;
        let process = guard.as_mut()?;
        if process.is_running() {
            Some(process.pid())
        } else {
            None
        }
    }

    /// Number of tabs with a running worker.
    pub fn running_workers(&self) -> usize {
        let ids: Vec<u64> = self
            .slots
            .lock()
            .map(|slots| slots.keys().copied().collect())
            .unwrap_or_default();
        ids.into_iter()
            .filter(|id| self.worker_pid(*id).is_some())
            .count()
    }

    fn slot(&self, tab_id: u64) -> Result<TabSlot, String> {
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| "tab worker table is unavailable".to_string())?;
        Ok(Arc::clone(slots.entry(tab_id).or_default()))
    }
}

impl Drop for TabWorkerPool {
    fn drop(&mut self) {
        self.close_all();
    }
}
