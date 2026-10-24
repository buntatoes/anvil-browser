//! Process limits for Flux.
//!
//! On Linux the window process sets `PR_SET_NO_NEW_PRIVS`, turns core dumps off,
//! and tries to cap address space at 2 GiB. `--tab-worker` caps address space at
//! 256 MiB. Pages in the window are still drawn in this process. A panic in
//! [`MandatorySandboxGuard::execute_isolated`] is caught here.
//!
//! [`enforce_mandatory_sandbox_or_abort`] quits if `PR_SET_NO_NEW_PRIVS` is rejected.

use flux_json::{JsonValue, parse_json_bounded, to_json_string};
use std::fmt;
use std::io::{Read, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Address-space cap for `--tab-worker` (256 MiB).
pub const MANDATORY_TAB_MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;

/// Address-space cap for the window process (2 GiB).
pub const UI_ADDRESS_SPACE_LIMIT_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Process count limit used by `--tab-worker` on Windows.
pub const MANDATORY_ACTIVE_PROCESS_LIMIT: u32 = 1;

/// Largest `FLUXIPC1` frame body (8 MiB).
pub const MANDATORY_IPC_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Deepest JSON object accepted on the worker pipe.
pub const MANDATORY_IPC_MAX_JSON_DEPTH: usize = 32;

/// Worker time budget, in milliseconds. The window does not enforce this.
pub const MANDATORY_WORKER_TIMEOUT_MS: u64 = 5_000;

/// 8-byte wire magic header for `flux-sandbox` IPC frames.
pub const IPC_MAGIC: &[u8; 8] = b"FLUXIPC1";

const SANDBOX_SEAL_TOKEN: u64 = 0x464C_5558_5342_5838; // "FLUXSBX8"

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxError {
    OsInitializationFailed(String),
    UnverifiedGuard,
    IpcFramingError(&'static str),
    IpcPayloadTooLarge(usize),
    TabIsolatedFault(String),
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OsInitializationFailed(reason) => {
                write!(f, "sandbox setup failed: {reason}")
            }
            Self::UnverifiedGuard => {
                write!(f, "sandbox guard is not active")
            }
            Self::IpcFramingError(msg) => write!(f, "sandbox IPC frame error: {msg}"),
            Self::IpcPayloadTooLarge(len) => {
                write!(
                    f,
                    "sandbox IPC payload ({len} bytes) exceeds {} MiB",
                    MANDATORY_IPC_MAX_BYTES / (1024 * 1024)
                )
            }
            Self::TabIsolatedFault(reason) => {
                write!(f, "tab isolated after fault: {reason}")
            }
        }
    }
}

impl std::error::Error for SandboxError {}

/// What the sandbox actually turned on for this process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxTelemetry {
    pub backend_name: &'static str,
    pub os_enforced: bool,
    pub memory_limit_bytes: usize,
    pub active_process_limit: u32,
    pub ui_restrictions_mask: u32,
    pub kill_on_close: bool,
    pub max_ipc_bytes: usize,
    pub isolated_tab_executions: u64,
    pub contained_tab_faults: u64,
}

/// Guard held by the browser for the life of the process.
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
    #[cfg(target_os = "windows")]
    job_handle: isize,
}

#[cfg(target_os = "windows")]
impl Drop for MandatorySandboxGuard {
    fn drop(&mut self) {
        if self.job_handle != 0 {
            unsafe {
                CloseHandle(self.job_handle);
            }
            self.job_handle = 0;
        }
    }
}

impl MandatorySandboxGuard {
    /// Set up the window-process limits, or return an error.
    pub fn initialize_or_fail() -> Result<Self, SandboxError> {
        verify_ipc_self_test()?;

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

        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(SandboxError::OsInitializationFailed(
                "this operating system is not supported".to_string(),
            ))
        }
    }

    /// Verify that this guard is intact and sealed.
    pub fn verify_active(&self) -> Result<(), SandboxError> {
        if self.seal != SANDBOX_SEAL_TOKEN {
            return Err(SandboxError::UnverifiedGuard);
        }
        #[cfg(target_os = "windows")]
        if self.job_handle == 0 {
            return Err(SandboxError::UnverifiedGuard);
        }
        Ok(())
    }

    /// Run `workload` and count a panic instead of letting it escape.
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

    /// Report the limits that were actually applied.
    pub fn telemetry(&self) -> SandboxTelemetry {
        SandboxTelemetry {
            backend_name: self.backend_name,
            os_enforced: self.verify_active().is_ok(),
            memory_limit_bytes: self.address_space_limit,
            active_process_limit: self.active_process_limit,
            ui_restrictions_mask: self.ui_restrictions_mask,
            kill_on_close: self.kill_on_close,
            max_ipc_bytes: MANDATORY_IPC_MAX_BYTES,
            isolated_tab_executions: self.isolated_executions,
            contained_tab_faults: self.contained_faults,
        }
    }
}

/// Set up sandbox limits, or exit the process.
pub fn enforce_mandatory_sandbox_or_abort() -> MandatorySandboxGuard {
    match MandatorySandboxGuard::initialize_or_fail() {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("flux: sandbox setup failed: {err}");
            std::process::exit(101);
        }
    }
}

/// Apply the tighter worker limits. Used by `--tab-worker`.
pub fn lock_current_worker_process_or_abort() {
    let res: Result<(), SandboxError> = (|| {
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
        #[cfg(target_os = "linux")]
        {
            apply_linux_worker_rlimits()
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(SandboxError::OsInitializationFailed(
                "unsupported worker OS".to_string(),
            ))
        }
    })();

    if let Err(err) = res {
        eprintln!("flux: worker sandbox setup failed: {err}");
        std::process::exit(102);
    }
}

/// Request message sent over a framed `FLUXIPC1` channel to an isolated tab worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabWorkerRequest {
    pub tab_id: u64,
    pub url: String,
    pub html: String,
    pub scripts_enabled: bool,
    pub csp_header: Option<String>,
}

/// Response message returned by an isolated tab worker over a framed `FLUXIPC1` channel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabWorkerResponse {
    pub tab_id: u64,
    pub ok: bool,
    pub title: String,
    pub dom_nodes: usize,
    pub blocked_csp_resources: usize,
    pub error: Option<String>,
}

impl TabWorkerRequest {
    pub fn to_json(&self) -> JsonValue {
        let mut obj = JsonValue::empty_object();
        obj.insert("tab_id", JsonValue::Number(self.tab_id as f64));
        obj.insert("url", JsonValue::String(self.url.clone()));
        obj.insert("html", JsonValue::String(self.html.clone()));
        obj.insert("scripts_enabled", JsonValue::Bool(self.scripts_enabled));
        match &self.csp_header {
            Some(csp) => obj.insert("csp_header", JsonValue::String(csp.clone())),
            None => obj.insert("csp_header", JsonValue::Null),
        }
        obj
    }

    pub fn from_json(val: &JsonValue) -> Result<Self, SandboxError> {
        let tab_id = val.get_u64("tab_id").ok_or(SandboxError::IpcFramingError(
            "missing tab_id in worker request",
        ))?;
        let url = val
            .get_str("url")
            .ok_or(SandboxError::IpcFramingError(
                "missing url in worker request",
            ))?
            .to_string();
        let html = val
            .get_str("html")
            .ok_or(SandboxError::IpcFramingError(
                "missing html in worker request",
            ))?
            .to_string();
        let scripts_enabled = val.get_bool("scripts_enabled").unwrap_or(true);
        let csp_header = val.get_str("csp_header").map(ToString::to_string);
        Ok(Self {
            tab_id,
            url,
            html,
            scripts_enabled,
            csp_header,
        })
    }
}

impl TabWorkerResponse {
    pub fn to_json(&self) -> JsonValue {
        let mut obj = JsonValue::empty_object();
        obj.insert("tab_id", JsonValue::Number(self.tab_id as f64));
        obj.insert("ok", JsonValue::Bool(self.ok));
        obj.insert("title", JsonValue::String(self.title.clone()));
        obj.insert("dom_nodes", JsonValue::Number(self.dom_nodes as f64));
        obj.insert(
            "blocked_csp_resources",
            JsonValue::Number(self.blocked_csp_resources as f64),
        );
        match &self.error {
            Some(err) => obj.insert("error", JsonValue::String(err.clone())),
            None => obj.insert("error", JsonValue::Null),
        }
        obj
    }

    pub fn from_json(val: &JsonValue) -> Result<Self, SandboxError> {
        let tab_id = val.get_u64("tab_id").ok_or(SandboxError::IpcFramingError(
            "missing tab_id in worker response",
        ))?;
        let ok = val.get_bool("ok").ok_or(SandboxError::IpcFramingError(
            "missing ok in worker response",
        ))?;
        let title = val.get_str("title").unwrap_or("Untitled").to_string();
        let dom_nodes = val.get_u64("dom_nodes").unwrap_or(0) as usize;
        let blocked_csp_resources = val.get_u64("blocked_csp_resources").unwrap_or(0) as usize;
        let error = val.get_str("error").map(ToString::to_string);
        Ok(Self {
            tab_id,
            ok,
            title,
            dom_nodes,
            blocked_csp_resources,
            error,
        })
    }
}

/// Encode a `JsonValue` into a length-prefixed `FLUXIPC1` wire frame bounded by
/// [`MANDATORY_IPC_MAX_BYTES`].
pub fn encode_ipc_frame(payload: &JsonValue) -> Result<Vec<u8>, SandboxError> {
    let serialized = to_json_string(payload);
    let bytes = serialized.as_bytes();
    if bytes.len() > MANDATORY_IPC_MAX_BYTES {
        return Err(SandboxError::IpcPayloadTooLarge(bytes.len()));
    }
    let mut frame = Vec::with_capacity(12 + bytes.len());
    frame.extend_from_slice(IPC_MAGIC);
    frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    frame.extend_from_slice(bytes);
    Ok(frame)
}

/// Decode and validate a length-prefixed `FLUXIPC1` wire frame using bounded JSON parsing.
pub fn decode_ipc_frame(frame: &[u8]) -> Result<JsonValue, SandboxError> {
    if frame.len() < 12 {
        return Err(SandboxError::IpcFramingError(
            "frame shorter than 12-byte header",
        ));
    }
    if &frame[..8] != IPC_MAGIC {
        return Err(SandboxError::IpcFramingError(
            "invalid FLUXIPC1 magic header",
        ));
    }
    let declared_len = u32::from_le_bytes([frame[8], frame[9], frame[10], frame[11]]) as usize;
    if declared_len > MANDATORY_IPC_MAX_BYTES {
        return Err(SandboxError::IpcPayloadTooLarge(declared_len));
    }
    if frame.len() != 12 + declared_len {
        return Err(SandboxError::IpcFramingError("frame length mismatch"));
    }
    let text = std::str::from_utf8(&frame[12..])
        .map_err(|_| SandboxError::IpcFramingError("non-UTF8 IPC payload"))?;
    parse_json_bounded(text, MANDATORY_IPC_MAX_JSON_DEPTH, MANDATORY_IPC_MAX_BYTES)
        .map_err(|_| SandboxError::IpcFramingError("malformed or over-nested IPC JSON"))
}

/// Write an IPC frame to a stream (such as a child worker's `stdin` or `stdout` pipe).
pub fn write_ipc_frame<W: Write>(writer: &mut W, payload: &JsonValue) -> Result<(), SandboxError> {
    let frame = encode_ipc_frame(payload)?;
    writer
        .write_all(&frame)
        .and_then(|_| writer.flush())
        .map_err(|_| SandboxError::IpcFramingError("failed to write IPC frame"))
}

/// Read an IPC frame from a stream with strict header and length validation before allocation.
pub fn read_ipc_frame<R: Read>(reader: &mut R) -> Result<JsonValue, SandboxError> {
    let mut header = [0_u8; 12];
    reader
        .read_exact(&mut header)
        .map_err(|_| SandboxError::IpcFramingError("failed to read 12-byte IPC header"))?;
    if &header[..8] != IPC_MAGIC {
        return Err(SandboxError::IpcFramingError(
            "invalid FLUXIPC1 magic header",
        ));
    }
    let declared_len = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    if declared_len > MANDATORY_IPC_MAX_BYTES {
        return Err(SandboxError::IpcPayloadTooLarge(declared_len));
    }
    let mut frame = Vec::with_capacity(12 + declared_len);
    frame.extend_from_slice(&header);
    frame.resize(12 + declared_len, 0);
    reader
        .read_exact(&mut frame[12..])
        .map_err(|_| SandboxError::IpcFramingError("truncated IPC frame body"))?;
    decode_ipc_frame(&frame)
}

fn verify_ipc_self_test() -> Result<(), SandboxError> {
    let req = TabWorkerRequest {
        tab_id: 1,
        url: "flux://security".to_string(),
        html: "<p>sandbox self-test</p>".to_string(),
        scripts_enabled: true,
        csp_header: Some("default-src 'self'".to_string()),
    };
    let frame = encode_ipc_frame(&req.to_json())?;
    let decoded_json = decode_ipc_frame(&frame)?;
    let decoded_req = TabWorkerRequest::from_json(&decoded_json)?;
    if decoded_req != req {
        return Err(SandboxError::OsInitializationFailed(
            "IPC round-trip self-test mismatch".to_string(),
        ));
    }
    Ok(())
}

// ============================================================================
// Windows Native Job Object & UI Restriction Sandbox (kernel32.dll)
// ============================================================================
#[cfg(target_os = "windows")]
const JOB_OBJECT_BASIC_UI_RESTRICTIONS: i32 = 4;
#[cfg(target_os = "windows")]
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

#[cfg(target_os = "windows")]
const JOB_OBJECT_LIMIT_ACTIVE_PROCESS: u32 = 0x0000_0008;
#[cfg(target_os = "windows")]
const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x0000_0100;
#[cfg(target_os = "windows")]
const JOB_OBJECT_LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0000_0400;
#[cfg(target_os = "windows")]
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[cfg(target_os = "windows")]
const JOB_OBJECT_UILIMIT_ALL: u32 = 0x0000_00FF;

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

#[cfg(target_os = "windows")]
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct JobObjectBasicUiRestrictions {
    ui_restrictions_class: u32,
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn CreateJobObjectW(security_attributes: *const core::ffi::c_void, name: *const u16) -> isize;
    fn SetInformationJobObject(
        job: isize,
        info_class: i32,
        info: *const core::ffi::c_void,
        info_length: u32,
    ) -> i32;
    fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
    fn GetCurrentProcess() -> isize;
    fn CloseHandle(handle: isize) -> i32;
}

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
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    ext.process_memory_limit = UI_ADDRESS_SPACE_LIMIT_BYTES;
    set_job_limits(job, &ext)?;
    Ok((job, UI_ADDRESS_SPACE_LIMIT_BYTES))
}

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

// ============================================================================
// Linux Native prctl(PR_SET_NO_NEW_PRIVS) & setrlimit Sandbox (libc)
// ============================================================================
#[cfg(target_os = "linux")]
const PR_SET_NO_NEW_PRIVS: i32 = 38;
#[cfg(target_os = "linux")]
const RLIMIT_CORE: i32 = 4;
#[cfg(target_os = "linux")]
const RLIMIT_AS: i32 = 9;

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy)]
struct RLimit {
    rlim_cur: usize,
    rlim_max: usize,
}

#[cfg(target_os = "linux")]
#[link(name = "c")]
unsafe extern "C" {
    fn prctl(option: i32, arg2: usize, arg3: usize, arg4: usize, arg5: usize) -> i32;
    fn getrlimit(resource: i32, rlim: *mut RLimit) -> i32;
    fn setrlimit(resource: i32, rlim: *const RLimit) -> i32;
}

#[cfg(target_os = "linux")]
fn cap_address_space(bytes: usize) -> usize {
    let mut current = RLimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    if unsafe { getrlimit(RLIMIT_AS, &mut current) } != 0 {
        return 0;
    }
    let cap = bytes.min(current.rlim_max);
    let mem_limit = RLimit {
        rlim_cur: cap,
        rlim_max: current.rlim_max,
    };
    if unsafe { setrlimit(RLIMIT_AS, &mem_limit) } == 0 {
        cap
    } else {
        0
    }
}

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

#[cfg(target_os = "linux")]
fn apply_linux_worker_rlimits() -> Result<(), SandboxError> {
    apply_linux_ui_limits()?;
    let _ = cap_address_space(MANDATORY_TAB_MEMORY_LIMIT_BYTES);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn initializes_mandatory_os_sandbox_and_seals_guard() {
        let mut guard = MandatorySandboxGuard::initialize_or_fail().unwrap();
        assert!(guard.verify_active().is_ok());

        let telemetry = guard.telemetry();
        assert!(telemetry.os_enforced);
        assert_eq!(telemetry.active_process_limit, 0);
        assert_eq!(telemetry.ui_restrictions_mask, 0);
        #[cfg(target_os = "linux")]
        assert!(!telemetry.kill_on_close);
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

    #[test]
    fn frames_and_validates_ipc_messages_with_bounded_limits() {
        let resp = TabWorkerResponse {
            tab_id: 7,
            ok: true,
            title: "Flux Sandbox Worker".to_string(),
            dom_nodes: 128,
            blocked_csp_resources: 2,
            error: None,
        };
        let mut pipe = Vec::new();
        write_ipc_frame(&mut pipe, &resp.to_json()).unwrap();

        let mut cursor = Cursor::new(pipe);
        let decoded_val = read_ipc_frame(&mut cursor).unwrap();
        let decoded = TabWorkerResponse::from_json(&decoded_val).unwrap();
        assert_eq!(decoded, resp);

        assert!(matches!(
            decode_ipc_frame(b"BADMAGIC\x02\x00\x00\x00{}"),
            Err(SandboxError::IpcFramingError(_))
        ));
    }
}
