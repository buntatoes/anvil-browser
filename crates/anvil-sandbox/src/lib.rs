// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Process limits for the window and for the per-tab worker processes.
//!
//! `anvil-browser` calls [`enforce_mandatory_sandbox_or_abort`] at startup and
//! holds the [`MandatorySandboxGuard`] for the life of the process. Page loads
//! run inside [`MandatorySandboxGuard::execute_isolated`], which is
//! `catch_unwind` in this same process. This crate never starts a process.
//! `anvil-browser` keeps one `--renderer` process per tab for http(s) pages
//! and local files, starts it through [`launcher`], sends it one [`wire`]
//! request per page, and waits [`MANDATORY_WORKER_TIMEOUT_MS`] for each
//! answer. The window still lays out and paints each document.
//!
//! Windows puts the window process in a Job Object with a 2 GiB process
//! memory cap, kill-on-job-close, and die-on-unhandled-exception. That job
//! does not set a process-count limit and does not set UI restrictions.
//! `--renderer` on Windows uses a second job: 256 MiB, one active process,
//! and `JOB_OBJECT_UILIMIT_ALL`. If that job cannot be applied the worker
//! exits. The window path does not use it.
//!
//! Linux sets `PR_SET_NO_NEW_PRIVS` and refuses to start if that call is
//! rejected. It sets `RLIMIT_CORE` to 0 and tries to cap `RLIMIT_AS` at 2 GiB
//! without raising a limit that is already lower. A failed address-space cap
//! does not abort the window. `--renderer` on Linux applies those same
//! limits and then requires the 256 MiB cap. If that cap is missing or still
//! above 256 MiB, the worker exits. Other operating systems are rejected.
//!
//! There is no GPU work and no script interpreter in this crate.
//!
//! [`wire`] is the binary message format for renderer processes:
//! versioned, size-capped, checked by direction, and fuzz-tested.
//! [`launcher`] starts a renderer process with an empty environment, checks
//! its hello, puts a deadline on every answer, watches its memory on Linux,
//! and kills it on the first bad frame. Neither confines the renderer at the
//! OS level.

pub mod launcher;
pub mod wire;

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Address-space cap for `--renderer` (256 MiB). Not applied to the window process.
pub const MANDATORY_TAB_MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;

/// Process-memory cap requested for the window (2 GiB).
///
/// On Linux this is an `RLIMIT_AS` target and may come back lower. On Windows
/// it is `JOB_OBJECT_LIMIT_PROCESS_MEMORY`.
pub const UI_ADDRESS_SPACE_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` value for `--renderer` on Windows.
/// The window job leaves the active-process limit at 0, which means unset.
pub const MANDATORY_ACTIVE_PROCESS_LIMIT: u32 = 1;

/// Worker time budget, in milliseconds.
///
/// This crate does not start a timer. `anvil-browser` waits this long for each
/// renderer answer, then kills that tab's renderer.
pub const MANDATORY_WORKER_TIMEOUT_MS: u64 = 5_000;

/// Value stored in `MandatorySandboxGuard::seal`. A guard with any other value fails [`MandatorySandboxGuard::verify_active`].
const SANDBOX_SEAL_TOKEN: u64 = 0x414E_5649_4C53_4258; // "ANVILSBX"

/// Why setup, a guard check, or an isolated call failed.
///
/// `OsInitializationFailed` is a rejected OS call or a failed startup self
/// test. `UnverifiedGuard` means the seal or, on Windows, the job handle is
/// missing. `TabIsolatedFault` is a workload `Err` or a panic caught by
/// [`MandatorySandboxGuard::execute_isolated`]. The panic stays in this
/// process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxError {
    OsInitializationFailed(String),
    UnverifiedGuard,
    TabIsolatedFault(String),
}

impl fmt::Display for SandboxError {
    /// One-line message.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OsInitializationFailed(reason) => {
                write!(f, "sandbox setup failed: {reason}")
            }
            Self::UnverifiedGuard => {
                write!(f, "sandbox guard is not active")
            }
            Self::TabIsolatedFault(reason) => {
                write!(f, "tab isolated after fault: {reason}")
            }
        }
    }
}

// `Display` is the error source. No extra fields.
impl std::error::Error for SandboxError {}

/// Limits recorded on the guard after setup. This is a snapshot, not a new OS call,
/// except `os_enforced`, which re-runs [`MandatorySandboxGuard::verify_active`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxTelemetry {
    pub backend_name: &'static str,
    pub os_enforced: bool,
    pub memory_limit_bytes: usize,
    pub active_process_limit: u32,
    pub ui_restrictions_mask: u32,
    pub kill_on_close: bool,
    /// Largest document one renderer frame carries.
    pub max_document_bytes: usize,
    pub isolated_tab_executions: u64,
    pub contained_tab_faults: u64,
}

/// Guard held by `anvil-browser` for the life of the window process.
///
/// `seal` must stay `SANDBOX_SEAL_TOKEN`. `isolated_executions` counts calls
/// to [`execute_isolated`](Self::execute_isolated). `contained_faults` counts
/// workload errors and panics. On Windows `job_handle` is the Job Object for
/// this process and is closed in `Drop`. Linux has no handle. The guard does
/// not move tabs into another process.
#[derive(Debug)]
pub struct MandatorySandboxGuard {
    seal: u64,
    backend_name: &'static str,
    ui_restrictions_mask: u32,
    address_space_limit: usize,
    active_process_limit: u32,
    kill_on_close: bool,
    isolated_executions: u64,
    contained_faults: u64,
    // Windows only. The Job Object for this process. Linux has no handle field.
    #[cfg(target_os = "windows")]
    job_handle: isize,
}

// Windows only. Linux has no job handle to close.
#[cfg(target_os = "windows")]
impl Drop for MandatorySandboxGuard {
    /// Close the window Job Object. A zero handle means it was already closed.
    ///
    /// `KILL_ON_JOB_CLOSE` is cleared first. The process that owns the guard is
    /// in this job, so leaving the flag set would terminate it inside `Drop`
    /// and hide the result of a test. A tab worker breaks away into its own job.
    fn drop(&mut self) {
        if self.job_handle != 0 {
            unsafe {
                disarm_window_job_kill(self.job_handle);
                CloseHandle(self.job_handle);
            }
            self.job_handle = 0;
        }
    }
}

impl MandatorySandboxGuard {
    /// Set up window-process limits, or return an error.
    ///
    /// Runs the wire-format round trip first. Windows assigns the current process to
    /// the UI job. Linux applies `PR_SET_NO_NEW_PRIVS`, a zero core limit, and
    /// the 2 GiB attempt. Any other OS returns `OsInitializationFailed`.
    /// Failure here does not switch the process into `--renderer`.
    pub fn initialize_or_fail() -> Result<Self, SandboxError> {
        verify_wire_self_test()?;

        // Windows window path: memory cap, die-on-unhandled-exception, kill-on-job-close.
        // No UI restriction class and no active-process cap.
        #[cfg(target_os = "windows")]
        {
            let (job_handle, limit) = create_windows_ui_job()?;
            let current = unsafe { GetCurrentProcess() };
            let assigned = unsafe { AssignProcessToJobObject(job_handle, current) };
            if assigned == 0 {
                unsafe {
                    CloseHandle(job_handle);
                }
                return Err(SandboxError::OsInitializationFailed(
                    "AssignProcessToJobObject failed".to_string(),
                ));
            }
            Ok(Self {
                seal: SANDBOX_SEAL_TOKEN,
                backend_name: "Windows job",
                ui_restrictions_mask: 0,
                address_space_limit: limit,
                active_process_limit: 0,
                kill_on_close: true,
                isolated_executions: 0,
                contained_faults: 0,
                job_handle,
            })
        }

        // Linux window path: no-new-privs is mandatory. The address cap is best-effort.
        #[cfg(target_os = "linux")]
        {
            let limit = apply_linux_ui_limits()?;
            Ok(Self {
                seal: SANDBOX_SEAL_TOKEN,
                backend_name: "Linux",
                ui_restrictions_mask: 0,
                address_space_limit: limit,
                active_process_limit: 0,
                kill_on_close: false,
                isolated_executions: 0,
                contained_faults: 0,
            })
        }

        // Not Windows and not Linux. The window refuses to start.
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(SandboxError::OsInitializationFailed(
                "this operating system is not supported".to_string(),
            ))
        }
    }

    /// Reject a guard whose seal was overwritten. On Windows a zero job handle also fails.
    pub fn verify_active(&self) -> Result<(), SandboxError> {
        if self.seal != SANDBOX_SEAL_TOKEN {
            return Err(SandboxError::UnverifiedGuard);
        }
        // Windows: the Job Object must still be open. Linux has no handle field.
        #[cfg(target_os = "windows")]
        if self.job_handle == 0 {
            return Err(SandboxError::UnverifiedGuard);
        }
        Ok(())
    }

    /// Run `workload` in this process and count a panic instead of letting it escape.
    ///
    /// `catch_unwind` is the isolation. There is no child process and no
    /// renderer launch. `Ok(Ok)` returns the value. `Ok(Err)` and a panic
    /// both increment `contained_faults` and return `TabIsolatedFault` with
    /// `tab_label`. The guard must already be sealed.
    pub fn execute_isolated<T, F>(
        &mut self,
        tab_label: &str,
        workload: F,
    ) -> Result<T, SandboxError>
    where
        F: FnOnce() -> Result<T, String>,
    {
        self.verify_active()?;
        self.isolated_executions = self.isolated_executions.saturating_add(1);

        match catch_unwind(AssertUnwindSafe(workload)) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(err)) => {
                self.contained_faults = self.contained_faults.saturating_add(1);
                Err(SandboxError::TabIsolatedFault(format!(
                    "[{tab_label}] {err}"
                )))
            }
            Err(_) => {
                self.contained_faults = self.contained_faults.saturating_add(1);
                Err(SandboxError::TabIsolatedFault(format!(
                    "[{tab_label}] panicked"
                )))
            }
        }
    }

    /// Copy the limits stored on this guard.
    ///
    /// `os_enforced` is the current [`verify_active`](Self::verify_active) result.
    /// `memory_limit_bytes` is whatever setup recorded, which on Linux may be
    /// lower than 2 GiB. `max_document_bytes` is always [`wire::MAX_DOCUMENT_BYTES`].
    pub fn telemetry(&self) -> SandboxTelemetry {
        SandboxTelemetry {
            backend_name: self.backend_name,
            os_enforced: self.verify_active().is_ok(),
            memory_limit_bytes: self.address_space_limit,
            active_process_limit: self.active_process_limit,
            ui_restrictions_mask: self.ui_restrictions_mask,
            kill_on_close: self.kill_on_close,
            max_document_bytes: wire::MAX_DOCUMENT_BYTES,
            isolated_tab_executions: self.isolated_executions,
            contained_tab_faults: self.contained_faults,
        }
    }
}

/// Set up the window limits, or exit with status 101.
///
/// This is what `anvil-browser` calls for normal browsing. It does not start a
/// tab worker. A printed line goes to stderr before the exit.
pub fn enforce_mandatory_sandbox_or_abort() -> MandatorySandboxGuard {
    match MandatorySandboxGuard::initialize_or_fail() {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("anvil: sandbox setup failed: {err}");
            std::process::exit(101);
        }
    }
}

/// Apply the tighter `--renderer` limits, or exit with status 102.
///
/// Called only from the `--renderer` branch in `anvil-browser`, once per
/// renderer process, before the first frame is read. Windows builds
/// the worker job (256 MiB, one process, all UI limits). Linux reapplies the
/// window limits and then requires the 256 MiB address cap.
pub fn lock_current_worker_process_or_abort() {
    if let Err(err) = lock_current_worker_process() {
        eprintln!("anvil: worker sandbox setup failed: {err}");
        std::process::exit(102);
    }
}

/// The body of [`lock_current_worker_process_or_abort`], as a `Result`.
fn lock_current_worker_process() -> Result<(), SandboxError> {
    // Windows worker: memory, process count, and UI limits. The UI mask is not stored.
    #[cfg(target_os = "windows")]
    {
        let (job, _) = create_windows_worker_job()?;
        let current = unsafe { GetCurrentProcess() };
        let assigned = unsafe { AssignProcessToJobObject(job, current) };
        if assigned == 0 {
            unsafe {
                CloseHandle(job);
            }
            return Err(SandboxError::OsInitializationFailed(
                "AssignProcessToJobObject failed for worker".to_string(),
            ));
        }
        Ok(())
    }
    // Linux worker: no-new-privs, core dumps off, then a required 256 MiB cap.
    #[cfg(target_os = "linux")]
    {
        apply_linux_worker_rlimits()
    }
    // Not Windows and not Linux. The worker exits via the error above.
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Err(SandboxError::OsInitializationFailed(
            "unsupported worker OS".to_string(),
        ))
    }
}

/// One page for a tab's renderer, before it is put in a [`wire::RenderRequest`].
///
/// The HTML is the document the browser already fetched. The cookie jar is
/// not in this object. `anvil-browser` makes one of these for each http(s)
/// page or local file that a tab opens. Defining the type does not spawn a
/// renderer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabWorkerRequest {
    pub tab_id: u64,
    pub url: String,
    pub html: String,
    pub scripts_enabled: bool,
    pub csp_header: Option<String>,
    /// CSS pixel width. Zero is treated as 1 by the worker.
    pub viewport_width: u32,
    /// CSS pixel height. Zero is treated as 1 by the worker.
    pub viewport_height: u32,
    /// HTTPS-only is on for this page's subresource requests.
    pub https_only: bool,
    /// Hosts that keep plain `http` while HTTPS-only is on.
    pub http_allowed_hosts: Vec<String>,
}

/// A renderer's answer for one page, taken from its [`wire`] reply.
///
/// `html` is the document after scripts when `ok` is true. The window paints that HTML only after a successful frame. A failed
/// or missing frame does not include the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabWorkerResponse {
    pub tab_id: u64,
    pub ok: bool,
    pub title: String,
    pub dom_nodes: usize,
    pub blocked_csp_resources: usize,
    pub error: Option<String>,
    /// Post-script document. Empty when `ok` is false.
    pub html: String,
}

/// Encode and decode one fixed render request before OS limits are applied.
///
/// A mismatch is `OsInitializationFailed`, so a broken wire format stops
/// startup. This does not talk to another process.
fn verify_wire_self_test() -> Result<(), SandboxError> {
    let request = wire::Message::Render(wire::RenderRequest {
        tab_id: 1,
        nav_id: 1,
        url: "anvil://security".to_string(),
        html: "<p>sandbox self-test</p>".to_string(),
        csp_header: Some("default-src 'self'".to_string()),
        scripts_enabled: true,
        viewport_width: 960,
        viewport_height: 720,
        https_only: true,
        http_allowed_hosts: vec!["intranet.test".to_string()],
    });
    let mismatch = || SandboxError::OsInitializationFailed("wire self-test mismatch".to_string());
    let frame = wire::encode(7, &request).map_err(|_| mismatch())?;
    let (seq, decoded) = wire::read_message(&mut frame.as_slice(), wire::Direction::ToRenderer)
        .map_err(|_| mismatch())?;
    if seq != 7 || decoded != request {
        return Err(mismatch());
    }
    Ok(())
}

// Windows only. kernel32 Job Object for the window process and for `--renderer`.
// The window job does not set UI restrictions. The worker job does.
#[cfg(target_os = "windows")]
/// `JobObjectBasicUIRestrictions` information class (4). Worker job only.
const JOB_OBJECT_BASIC_UI_RESTRICTIONS: i32 = 4;
#[cfg(target_os = "windows")]
/// `JobObjectExtendedLimitInformation` information class (9). Both jobs.
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

#[cfg(target_os = "windows")]
/// Limit flag: cap how many processes the job may hold. Worker job only.
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
#[cfg(target_os = "windows")]
/// Limit flag: `ProcessMemoryLimit` is in bytes. Both jobs.
const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
#[cfg(target_os = "windows")]
/// Limit flag: an unhandled exception ends the process. Both jobs.
const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0000_0400;
#[cfg(target_os = "windows")]
/// Limit flag: closing the last job handle kills assigned processes. Both jobs.
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
#[cfg(target_os = "windows")]
/// Limit flag: a child may pass `CREATE_BREAKAWAY_FROM_JOB`. Window job only.
const JOB_OBJECT_LIMIT_BREAKAWAY_OK: u32 = 0x0000_0800;

#[cfg(target_os = "windows")]
/// All basic UI restriction bits (0xFF). Clipboard, desktop, handles, and exits. Worker only.
const JOB_OBJECT_UILIMIT_ALL: u32 = 0x0000_00FF;

/// `JOBOBJECT_BASIC_LIMIT_INFORMATION`. Only `limit_flags` and `active_process_limit` are set.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobObjectBasicLimitInformation {
    per_process_user_time_limit: i64,
    per_job_user_time_limit: i64,
    limit_flags: u32,
    minimum_working_set_size: usize,
    maximum_working_set_size: usize,
    active_process_limit: u32,
    affinity: usize,
    priority_class: u32,
    scheduling_class: u32,
}

/// `IO_COUNTERS`. Present so the extended struct matches the Windows layout. Left zero.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct IoCounters {
    read_operation_count: u64,
    write_operation_count: u64,
    other_operation_count: u64,
    read_transfer_count: u64,
    write_transfer_count: u64,
    other_transfer_count: u64,
}

/// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION`. `process_memory_limit` is the cap in bytes.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobObjectExtendedLimitInformation {
    basic_limit_information: JobObjectBasicLimitInformation,
    io_info: IoCounters,
    process_memory_limit: usize,
    job_memory_limit: usize,
    peak_process_memory_used: usize,
    peak_job_memory_used: usize,
}

/// `JOBOBJECT_BASIC_UI_RESTRICTIONS`. One `u32` class mask. Worker job only.
#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobObjectBasicUiRestrictions {
    ui_restrictions_class: u32,
}

// kernel32 job calls. Handles are `isize`. A zero return is failure.
#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    /// Anonymous job. Both pointers are null. There is no job name.
    fn CreateJobObjectW(security_attributes: *const core::ffi::c_void, name: *const u16) -> isize;
    /// Writes either extended limits or basic UI restrictions, selected by `info_class`.
    fn SetInformationJobObject(
        job: isize,
        info_class: i32,
        info: *const core::ffi::c_void,
        info_length: u32,
    ) -> i32;
    /// Puts `process` into `job`. The window and the worker each assign themselves.
    fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
    /// Pseudo-handle of this process. Not closed.
    fn GetCurrentProcess() -> isize;
    /// Closes a job handle. Used on failure and when the guard is dropped.
    fn CloseHandle(handle: isize) -> i32;
}

/// Drop `KILL_ON_JOB_CLOSE` so closing the handle does not terminate this process.
#[cfg(target_os = "windows")]
fn disarm_window_job_kill(job: isize) {
    let mut ext = JobObjectExtendedLimitInformation::default();
    ext.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
        | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    ext.process_memory_limit = UI_ADDRESS_SPACE_LIMIT_BYTES;
    unsafe {
        let _ = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            (&ext as *const JobObjectExtendedLimitInformation).cast(),
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        );
    }
}

/// Apply extended limits. On failure the job handle is closed and an error is returned.
#[cfg(target_os = "windows")]
fn set_job_limits(job: isize, ext: &JobObjectExtendedLimitInformation) -> Result<(), SandboxError> {
    let ok = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            (ext as *const JobObjectExtendedLimitInformation).cast(),
            std::mem::size_of::<JobObjectExtendedLimitInformation>() as u32,
        )
    };
    if ok == 0 {
        unsafe {
            CloseHandle(job);
        }
        return Err(SandboxError::OsInitializationFailed(
            "SetInformationJobObject failed".to_string(),
        ));
    }
    Ok(())
}

/// Window job: 2 GiB process memory, die on unhandled exception, kill when the job closes.
///
/// `JOB_OBJECT_LIMIT_BREAKAWAY_OK` lets a child started with
/// `CREATE_BREAKAWAY_FROM_JOB` leave this job. The tab worker uses that so it
/// can enter its own job. No active-process limit and no UI restriction class.
/// Returns the handle and the byte cap.
#[cfg(target_os = "windows")]
fn create_windows_ui_job() -> Result<(isize, usize), SandboxError> {
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job == 0 {
        return Err(SandboxError::OsInitializationFailed(
            "CreateJobObjectW returned NULL".to_string(),
        ));
    }
    let mut ext = JobObjectExtendedLimitInformation::default();
    ext.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
    ext.process_memory_limit = UI_ADDRESS_SPACE_LIMIT_BYTES;
    set_job_limits(job, &ext)?;
    Ok((job, UI_ADDRESS_SPACE_LIMIT_BYTES))
}

/// Worker job: one active process, 256 MiB, the same kill flags, then all UI limits.
///
/// Returns the handle and `JOB_OBJECT_UILIMIT_ALL`. The caller discards the mask.
/// A failed UI call closes the job. Only tab worker processes use this job.
#[cfg(target_os = "windows")]
fn create_windows_worker_job() -> Result<(isize, u32), SandboxError> {
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job == 0 {
        return Err(SandboxError::OsInitializationFailed(
            "CreateJobObjectW returned NULL".to_string(),
        ));
    }
    let mut ext = JobObjectExtendedLimitInformation::default();
    ext.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    ext.basic_limit_information.active_process_limit = MANDATORY_ACTIVE_PROCESS_LIMIT;
    ext.process_memory_limit = MANDATORY_TAB_MEMORY_LIMIT_BYTES;
    set_job_limits(job, &ext)?;

    let ui = JobObjectBasicUiRestrictions {
        ui_restrictions_class: JOB_OBJECT_UILIMIT_ALL,
    };
    let ok_ui = unsafe {
        SetInformationJobObject(
            job,
            JOB_OBJECT_BASIC_UI_RESTRICTIONS,
            (&ui as *const JobObjectBasicUiRestrictions).cast(),
            std::mem::size_of::<JobObjectBasicUiRestrictions>() as u32,
        )
    };
    if ok_ui == 0 {
        unsafe {
            CloseHandle(job);
        }
        return Err(SandboxError::OsInitializationFailed(
            "SetInformationJobObject failed".to_string(),
        ));
    }
    Ok((job, JOB_OBJECT_UILIMIT_ALL))
}

// Linux only. prctl and setrlimit. Windows uses the Job Object above instead.
#[cfg(target_os = "linux")]
/// `prctl` option. Arg2 = 1 locks out new privileges for this process.
const PR_SET_NO_NEW_PRIVS: i32 = 38;
#[cfg(target_os = "linux")]
/// Resource id for core dumps. Set to zero so a crash does not write a core.
const RLIMIT_CORE: i32 = 4;
#[cfg(target_os = "linux")]
/// Resource id for address space. The cap is in bytes.
const RLIMIT_AS: i32 = 9;

/// `rlimit`. `rlim_cur` is the soft limit. `rlim_max` is the hard ceiling.
#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy)]
struct RLimit {
    rlim_cur: usize,
    rlim_max: usize,
}

// libc calls. A non-zero return is failure. errno is not read.
#[cfg(target_os = "linux")]
#[link(name = "c")]
unsafe extern "C" {
    /// Used only as `prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)`.
    fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    /// Reads the current soft and hard limit for one resource.
    fn getrlimit(resource: i32, rlim: *mut RLimit) -> i32;
    /// Writes a soft and hard limit. The hard limit is not raised.
    fn setrlimit(resource: i32, rlim: *const RLimit) -> i32;
}

/// Try to lower `RLIMIT_AS` to `bytes` without raising either side.
///
/// Returns the soft limit that is in effect afterward. `getrlimit` failure
/// returns 0. `setrlimit` failure returns the previous soft limit. `RLIM_INFINITY`
/// is `usize::MAX` and is replaced rather than kept when it is the hard limit.
#[cfg(target_os = "linux")]
fn cap_address_space(bytes: usize) -> usize {
    /// `RLIM_INFINITY` on this platform.
    const INFINITY: usize = usize::MAX;
    let mut current = RLimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { getrlimit(RLIMIT_AS, &mut current) } != 0 {
        return 0;
    }
    let mut target = if current.rlim_max == INFINITY {
        bytes
    } else {
        bytes.min(current.rlim_max)
    };
    if current.rlim_cur != INFINITY {
        target = target.min(current.rlim_cur);
    }
    if target == current.rlim_cur && current.rlim_cur != INFINITY {
        return current.rlim_cur;
    }
    let mem_limit = RLimit {
        rlim_cur: target,
        rlim_max: current.rlim_max,
    };
    if unsafe { setrlimit(RLIMIT_AS, &mem_limit) } == 0 {
        target
    } else {
        current.rlim_cur
    }
}

/// Window limits. `PR_SET_NO_NEW_PRIVS` must succeed.
///
/// Core dumps are set to zero and a failure there is ignored. The 2 GiB
/// address cap is best-effort: the returned value may be 0 or the old soft limit.
#[cfg(target_os = "linux")]
fn apply_linux_ui_limits() -> Result<usize, SandboxError> {
    let rc = unsafe { prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if rc != 0 {
        return Err(SandboxError::OsInitializationFailed(
            "prctl(PR_SET_NO_NEW_PRIVS) was rejected".to_string(),
        ));
    }
    let core_zero = RLimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let _ = unsafe { setrlimit(RLIMIT_CORE, &core_zero) };
    Ok(cap_address_space(UI_ADDRESS_SPACE_LIMIT_BYTES))
}

/// Worker limits. Runs the window limits, then requires a soft cap at or under 256 MiB.
///
/// `applied == 0` or a cap still above 256 MiB is a hard error. The caller exits.
/// Only tab worker processes call this. The window does not.
#[cfg(target_os = "linux")]
fn apply_linux_worker_rlimits() -> Result<(), SandboxError> {
    apply_linux_ui_limits()?;
    let applied = cap_address_space(MANDATORY_TAB_MEMORY_LIMIT_BYTES);
    if applied == 0 || applied > MANDATORY_TAB_MEMORY_LIMIT_BYTES {
        return Err(SandboxError::OsInitializationFailed(
            "address space cap was not applied".to_string(),
        ));
    }
    Ok(())
}

/// Window-guard startup, panic accounting, and the wire self-test. No child process.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initializes_mandatory_os_sandbox_and_seals_guard() {
        let mut guard = MandatorySandboxGuard::initialize_or_fail().unwrap();
        assert!(guard.verify_active().is_ok());

        let telemetry = guard.telemetry();
        assert!(telemetry.os_enforced);
        assert_eq!(telemetry.active_process_limit, 0);
        assert_eq!(telemetry.ui_restrictions_mask, 0);
        // Linux window job is not kill-on-close. There is no job handle.
        #[cfg(target_os = "linux")]
        assert!(!telemetry.kill_on_close);
        // Windows window job sets JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.
        #[cfg(target_os = "windows")]
        assert!(telemetry.kill_on_close);

        let ok_res = guard
            .execute_isolated("tab-1", || Ok::<_, String>(42))
            .unwrap();
        assert_eq!(ok_res, 42);
        assert_eq!(guard.telemetry().isolated_tab_executions, 1);
        assert_eq!(guard.telemetry().contained_tab_faults, 0);

        let fault_res: Result<(), SandboxError> =
            guard.execute_isolated("tab-2", || Err("script exceeded recursion cap".to_string()));
        assert!(matches!(fault_res, Err(SandboxError::TabIsolatedFault(_))));
        assert_eq!(guard.telemetry().contained_tab_faults, 1);
    }
}
