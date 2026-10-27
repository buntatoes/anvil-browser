// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! Renderer confinement through helper tools the OS already has.
//!
//! The browser has no code of its own that can make a process give up
//! access, because that needs system calls that std doesn't wrap. Linux
//! systems usually have small tools that do it, so on Linux the renderer
//! is started through one of them:
//!
//! - `bwrap` gives it new user, pid, network, IPC, UTS, mount and cgroup
//!   namespaces, a read-only `/usr` and the renderer executable, a private
//!   `/proc`, `/dev` and `/tmp`, and nothing else of the filesystem. It
//!   drops every capability, starts a new session, and ends the renderer
//!   if the browser goes away.
//! - `unshare` is used when `bwrap` is missing or fails. It gives the same
//!   namespaces and a private `/proc`, but the renderer still sees the rest
//!   of the filesystem.
//! - `prlimit`, when it is there too, caps open files at 256, turns core
//!   files off, and sets a file size limit of 0. That limit only stops
//!   writing data into a file: creating empty files and folders,
//!   truncating, renaming and deleting still work. Only `bwrap`'s
//!   read-only view keeps the host's files unchanged.
//!
//! On Windows a copy of the executable is given a low integrity label with
//! `icacls`, and renderers are started from that copy.
//!
//! The tools are looked for in fixed system folders, never on `PATH`, when
//! the browser first needs a renderer. A tool is only used after a
//! self-test: a renderer started through it has to answer, render a small
//! page, and report, from its own `/proc/self`, that it really is in its
//! own namespaces and can't see the browser's processes. If no tool is
//! found, or none passes, renderers start as before, and the reason is
//! kept for the security page. [`ConfineMode::Off`] skips all of this.
//!
//! [`read_own_status`] runs in the renderer. It reads its own entries in
//! `/proc/self` with plain file reads and fills a [`RendererStatus`], which
//! the browser asks for over the wire. Where a value can't be read it stays
//! unknown, so the security page never has to guess.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::launcher::{RenderOutcome, RendererCommand, RendererLimits, RendererProcess, Wrapping};
use crate::wire::{FetchReply, NAMESPACES, RenderRequest, RendererStatus};

/// Whether renderers are started through a helper tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ConfineMode {
    /// Use the first helper tool that is there and passes its self-test.
    #[default]
    Auto,
    /// Start renderers directly, as before.
    Off,
}

impl ConfineMode {
    /// The name stored in the profile.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
        }
    }

    /// The mode for a stored name. Anything unknown is `Auto`.
    pub fn from_slug(slug: &str) -> Self {
        match slug {
            "off" => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// The kinds of helper tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Bwrap,
    Unshare,
    LowIntegrity,
}

impl ToolKind {
    /// Short name for the security page.
    pub fn name(self) -> &'static str {
        match self {
            Self::Bwrap => "bwrap",
            Self::Unshare => "unshare",
            Self::LowIntegrity => "icacls (low integrity copy)",
        }
    }

    /// True when the tool gives the renderer its own namespaces.
    pub fn uses_namespaces(self) -> bool {
        matches!(self, Self::Bwrap | Self::Unshare)
    }
}

/// The folders helper tools are looked for in on Linux. These are fixed,
/// so nothing in the environment can point the browser at another program.
pub const TOOL_DIRS: [&str; 3] = ["/usr/bin", "/bin", "/usr/local/bin"];

/// What `prlimit` sets on the wrapper and the renderer below it: at most
/// 256 open files, no core files, and a file size limit of 0, which only
/// stops writing data into a file, not creating, truncating, renaming or
/// deleting one.
pub const PRLIMIT_ARGS: [&str; 3] = ["--nofile=256:256", "--fsize=0:0", "--core=0:0"];

/// The namespaces a namespace tool must give the renderer, as indexes into
/// [`NAMESPACES`]: pid, user, net, ipc, uts and mnt. The cgroup namespace
/// is asked for too, but some kernels don't have it.
const REQUIRED_NAMESPACES: [usize; 6] = [0, 1, 2, 3, 4, 5];

/// Most processes a confined renderer may see. In its own pid namespace it
/// sees itself and the tool's own helper.
const MAX_VISIBLE_PROCESSES: u32 = 4;

/// The top-level folders that may be symlinks into `/usr` or real folders,
/// depending on the system.
const SYSTEM_TOP_DIRS: [&str; 6] = ["/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32"];

/// One helper tool that was found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Helper {
    pub kind: ToolKind,
    pub path: PathBuf,
    /// `prlimit`, when it was found too. Linux only.
    pub prlimit: Option<PathBuf>,
}

/// `name` in the first of `dirs` that has it as a file anyone may run.
pub fn find_tool_in(dirs: &[&Path], name: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|path| is_runnable(path))
}

fn is_runnable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.is_file() && meta.permissions().mode() & 0o111 == 0o111
    }
    #[cfg(not(unix))]
    {
        meta.is_file()
    }
}

/// The helper tools on this system, best first. Nothing is run here.
pub fn find_helpers() -> Vec<Helper> {
    #[cfg(target_os = "linux")]
    {
        let dirs: Vec<&Path> = TOOL_DIRS.iter().map(Path::new).collect();
        let prlimit = find_tool_in(&dirs, "prlimit");
        [(ToolKind::Bwrap, "bwrap"), (ToolKind::Unshare, "unshare")]
            .into_iter()
            .filter_map(|(kind, name)| {
                Some(Helper {
                    kind,
                    path: find_tool_in(&dirs, name)?,
                    prlimit: prlimit.clone(),
                })
            })
            .collect()
    }
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system = root.join("System32");
        find_tool_in(&[system.as_path()], "icacls.exe")
            .map(|path| Helper {
                kind: ToolKind::LowIntegrity,
                path,
                prlimit: None,
            })
            .into_iter()
            .collect()
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        Vec::new()
    }
}

impl Helper {
    /// `command`, changed to start through this helper. The renderer's
    /// program has to be an absolute path, and on Linux valid text.
    pub fn wrap(&self, command: &RendererCommand) -> Result<RendererCommand, String> {
        let program = std::fs::canonicalize(&command.program)
            .map_err(|err| format!("the renderer executable can't be found: {err}"))?;
        let mut wrapped = command.clone();
        match self.kind {
            ToolKind::Bwrap | ToolKind::Unshare => {
                let program_text = program
                    .to_str()
                    .ok_or("the renderer path is not valid text")?
                    .to_string();
                let tool = self
                    .path
                    .to_str()
                    .ok_or("the tool path is not valid text")?
                    .to_string();
                let mut args = if self.kind == ToolKind::Bwrap {
                    bwrap_args(&program_text)
                } else {
                    unshare_args()
                };
                args.push("--".to_string());
                let program_of_wrapper = match &self.prlimit {
                    Some(prlimit) => {
                        let mut front: Vec<String> =
                            PRLIMIT_ARGS.iter().map(|arg| arg.to_string()).collect();
                        front.push("--".to_string());
                        front.push(tool);
                        front.append(&mut args);
                        args = front;
                        prlimit.clone()
                    }
                    None => self.path.clone(),
                };
                wrapped.program = program;
                wrapped.wrapper = Some(Wrapping {
                    program: program_of_wrapper,
                    args,
                    signal_as_exit_code: self.kind == ToolKind::Bwrap,
                });
            }
            ToolKind::LowIntegrity => {
                wrapped.program = low_integrity_copy(&self.path, &program)?;
            }
        }
        Ok(wrapped)
    }
}

/// The `bwrap` arguments for a renderer at `program`, without the final
/// `--`.
pub fn bwrap_args(program: &str) -> Vec<String> {
    let mut args: Vec<String> = [
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--ro-bind",
        "/usr",
        "/usr",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect();
    for dir in SYSTEM_TOP_DIRS {
        let Ok(meta) = std::fs::symlink_metadata(dir) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            if let Some(target) = std::fs::read_link(dir)
                .ok()
                .and_then(|target| target.to_str().map(str::to_string))
            {
                args.extend(["--symlink".to_string(), target, dir.to_string()]);
            }
        } else if meta.is_dir() {
            args.extend(["--ro-bind".to_string(), dir.to_string(), dir.to_string()]);
        }
    }
    args.extend(
        ["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]
            .iter()
            .map(|arg| arg.to_string()),
    );
    // Bound last: a later mount over a folder above it, such as the empty
    // /tmp, would hide it and the renderer would not start.
    if !program.starts_with("/usr/") {
        args.extend([
            "--ro-bind".to_string(),
            program.to_string(),
            program.to_string(),
        ]);
    }
    args.extend(["--chdir".to_string(), "/".to_string()]);
    args
}

/// The `unshare` arguments, without the final `--`. The renderer keeps
/// its own user id inside, and is killed if `unshare` goes away.
pub fn unshare_args() -> Vec<String> {
    [
        "--user",
        "--map-current-user",
        "--pid",
        "--fork",
        "--kill-child",
        "--mount",
        "--mount-proc",
        "--net",
        "--ipc",
        "--uts",
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect()
}

/// Copy `program` next to itself and give the copy a low integrity label
/// with `icacls`. Windows starts a process from such a file at low
/// integrity.
fn low_integrity_copy(icacls: &Path, program: &Path) -> Result<PathBuf, String> {
    let dir = program
        .parent()
        .ok_or("the renderer executable has no folder")?;
    let copy = dir.join("anvil-renderer-low.exe");
    std::fs::copy(program, &copy).map_err(|err| format!("could not copy the renderer: {err}"))?;
    let status = std::process::Command::new(icacls)
        .arg(&copy)
        .args(["/setintegritylevel", "low"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|err| format!("icacls did not start: {err}"))?;
    if status.success() {
        Ok(copy)
    } else {
        Err(format!("icacls failed ({status})"))
    }
}

/// What the browser decided about confinement, and why.
#[derive(Clone, Debug, Default)]
pub struct Confinement {
    pub mode: ConfineMode,
    /// Every helper tool that was found.
    pub found: Vec<Helper>,
    /// The helper renderers are started through, if any.
    pub in_use: Option<Helper>,
    /// One line for each tool that was tried and not used, and why, or why
    /// none was tried.
    pub notes: Vec<String>,
    /// What the self-test renderer read back about itself, when a helper
    /// passed.
    pub self_test: Option<RendererStatus>,
    /// The renderer executable, as found on disk.
    pub program: PathBuf,
    command: Option<RendererCommand>,
}

impl Confinement {
    /// A decision that could not be made, with the reason. Renderers start
    /// plainly.
    pub fn unavailable(mode: ConfineMode, reason: impl Into<String>) -> Self {
        Self {
            mode,
            notes: vec![reason.into()],
            ..Self::default()
        }
    }

    /// The command to start renderers with, in place of `plain`.
    pub fn command_for(&self, plain: &RendererCommand) -> RendererCommand {
        self.command.clone().unwrap_or_else(|| plain.clone())
    }
}

/// Decide how to start renderers that would otherwise run as `plain`.
///
/// With [`ConfineMode::Auto`] each helper that was found is tried in turn
/// with a self-test, and the first that passes is used. `limits` are the
/// limits the self-test renderer gets.
pub fn decide(mode: ConfineMode, plain: &RendererCommand, limits: &RendererLimits) -> Confinement {
    decide_with(mode, plain, limits, find_helpers())
}

/// [`decide`] with the given helpers in place of the ones on the system.
/// Tests use this to check what happens when a tool fails.
pub fn decide_with(
    mode: ConfineMode,
    plain: &RendererCommand,
    limits: &RendererLimits,
    found: Vec<Helper>,
) -> Confinement {
    let mut decision = Confinement {
        mode,
        found: found.clone(),
        program: std::fs::canonicalize(&plain.program).unwrap_or_else(|_| plain.program.clone()),
        ..Confinement::default()
    };
    if mode == ConfineMode::Off {
        decision
            .notes
            .push("confinement is turned off in the settings".to_string());
        return decision;
    }
    if found.is_empty() {
        decision.notes.push(if cfg!(windows) {
            "icacls was not found in the system folder".to_string()
        } else if cfg!(target_os = "linux") {
            format!(
                "neither bwrap nor unshare was found in {}",
                TOOL_DIRS.join(", ")
            )
        } else {
            "this system has no supported confinement tool".to_string()
        });
        return decision;
    }
    let ours = own_namespaces();
    for helper in found {
        let name = helper.kind.name();
        let attempt = helper
            .wrap(plain)
            .and_then(|command| Ok((self_test(&command, limits, helper.kind, &ours)?, command)));
        match attempt {
            Ok((status, command)) => {
                decision.self_test = Some(status);
                decision.command = Some(command);
                decision.in_use = Some(helper);
                break;
            }
            Err(reason) => decision.notes.push(format!(
                "{name} was found but failed its self-test: {reason}"
            )),
        }
    }
    decision
}

/// Start one renderer with `command`, have it render a small page, and
/// check what it reports about itself.
pub fn self_test(
    command: &RendererCommand,
    limits: &RendererLimits,
    kind: ToolKind,
    browser_namespaces: &[u64; 7],
) -> Result<RendererStatus, String> {
    let limits = RendererLimits {
        startup: limits.startup.min(Duration::from_secs(10)),
        reply: limits.reply.min(Duration::from_secs(10)),
        ..limits.clone()
    };
    let mut process = RendererProcess::launch(command, limits).map_err(|err| err.to_string())?;
    let status = process.status().map_err(|err| err.to_string())?;
    let page = RenderRequest {
        tab_id: 0,
        nav_id: 1,
        url: "http://self-test.invalid/".to_string(),
        html: "<!DOCTYPE html><p id=t>self test</p><script>document.title='ok'</script>"
            .to_string(),
        csp_header: None,
        scripts_enabled: true,
        viewport_width: 320,
        viewport_height: 240,
        https_only: false,
        http_allowed_hosts: Vec::new(),
    };
    let mut refuse = |want: &crate::wire::FetchRequest| FetchReply {
        nav_id: want.nav_id,
        status: 0,
        content_type: String::new(),
        body: Vec::new(),
    };
    match process.render(&page, &mut refuse) {
        Ok(RenderOutcome::Done(..)) => {}
        Ok(RenderOutcome::Refused(failure)) => {
            return Err(format!("the test page was refused ({:?})", failure.code));
        }
        Err(err) => return Err(err.to_string()),
    }
    process.shutdown();
    check_status(kind, &status, browser_namespaces)?;
    Ok(status)
}

/// Check that `status`, from a renderer started through a tool of `kind`,
/// shows the confinement that tool is meant to give.
pub fn check_status(
    kind: ToolKind,
    status: &RendererStatus,
    browser_namespaces: &[u64; 7],
) -> Result<(), String> {
    if status.no_new_privs != Some(true) {
        return Err("it does not have NoNewPrivs set".to_string());
    }
    if !kind.uses_namespaces() {
        return Ok(());
    }
    for index in REQUIRED_NAMESPACES {
        let name = NAMESPACES[index];
        let theirs = status.namespaces[index];
        if theirs == 0 || browser_namespaces[index] == 0 {
            return Err(format!("its {name} namespace could not be read"));
        }
        if theirs == browser_namespaces[index] {
            return Err(format!("it shares the browser's {name} namespace"));
        }
    }
    if status.visible_processes > MAX_VISIBLE_PROCESSES {
        return Err(format!("it can see {} processes", status.visible_processes));
    }
    Ok(())
}

/// What this process can read about itself. Outside Linux every field is
/// unknown.
pub fn read_own_status() -> RendererStatus {
    #[cfg(target_os = "linux")]
    {
        linux_own_status()
    }
    #[cfg(not(target_os = "linux"))]
    {
        RendererStatus::default()
    }
}

/// Whether one protection is in place, as far as the OS says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Active,
    /// In place, but it stops less than its name might suggest. The
    /// detail says what it leaves open.
    Limited,
    NotActive,
    /// The OS didn't say, so the page must not claim it.
    Unknown,
}

/// Writable mounts that don't reach the host's files: the private root,
/// `/tmp`, `/dev` and `/proc` that `bwrap` makes, and its device nodes.
const PRIVATE_WRITABLE_MOUNTS: [&str; 11] = [
    "/:tmpfs",
    "/tmp:tmpfs",
    "/dev:tmpfs",
    "/proc:proc",
    "/dev/pts:devpts",
    "/dev/null:devtmpfs",
    "/dev/zero:devtmpfs",
    "/dev/full:devtmpfs",
    "/dev/random:devtmpfs",
    "/dev/urandom:devtmpfs",
    "/dev/tty:devtmpfs",
];

/// Whether a renderer's `writable_mounts` list shows it can't change any
/// of the host's files: every writable mount is one of
/// [`PRIVATE_WRITABLE_MOUNTS`], and the list wasn't cut short.
fn only_private_mounts_writable(writable_mounts: &str) -> bool {
    writable_mounts == "none"
        || (!writable_mounts.is_empty()
            && !writable_mounts.ends_with(" ...")
            && writable_mounts
                .split(' ')
                .all(|mount| PRIVATE_WRITABLE_MOUNTS.contains(&mount)))
}

/// One line of the security page's confinement table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Protection {
    pub name: &'static str,
    pub state: State,
    pub detail: String,
}

fn row(name: &'static str, state: State, detail: impl Into<String>) -> Protection {
    Protection {
        name,
        state,
        detail: detail.into(),
    }
}

fn known(value: Option<bool>) -> State {
    match value {
        Some(true) => State::Active,
        Some(false) => State::NotActive,
        None => State::Unknown,
    }
}

fn limit_text(value: Option<u64>) -> String {
    match value {
        None => "not readable".to_string(),
        Some(u64::MAX) => "unlimited".to_string(),
        Some(value) => value.to_string(),
    }
}

/// The confinement table for the security page.
///
/// `decision` says which tool renderers are started through. `live` is
/// what a fresh renderer, started the same way, read about itself just
/// now, or why none could be asked. Every line about the renderer comes
/// from `live` and is compared with `browser`, this process's own
/// namespaces. Nothing is marked active unless the renderer's own reading
/// shows it.
pub fn protections(
    decision: &Confinement,
    live: Result<&RendererStatus, &str>,
    browser: &[u64; 7],
) -> Vec<Protection> {
    let mut rows = Vec::new();
    let tool_detail = match &decision.in_use {
        Some(helper) => {
            let mut text = format!("{} at {}", helper.kind.name(), helper.path.display());
            if let Some(prlimit) = &helper.prlimit {
                text.push_str(&format!(", with prlimit at {}", prlimit.display()));
            }
            text
        }
        None => decision.notes.join("; "),
    };
    rows.push(row(
        "Started through a confinement tool",
        if decision.in_use.is_some() {
            State::Active
        } else {
            State::NotActive
        },
        tool_detail,
    ));
    let status = match live {
        Ok(status) => status,
        Err(reason) => {
            rows.push(row(
                "Read back from a renderer",
                State::Unknown,
                format!("no renderer could be asked: {reason}"),
            ));
            return rows;
        }
    };
    for (index, name) in [
        (1, "Own user namespace"),
        (0, "Own pid namespace"),
        (2, "Own network namespace"),
        (3, "Own IPC namespace"),
        (4, "Own UTS namespace"),
        (5, "Own mount namespace"),
    ] {
        let theirs = status.namespaces[index];
        let ours = browser[index];
        let state = if theirs == 0 || ours == 0 {
            State::Unknown
        } else if theirs != ours {
            State::Active
        } else {
            State::NotActive
        };
        let detail = if state == State::Unknown {
            "not readable".to_string()
        } else {
            format!("renderer {theirs}, window {ours}")
        };
        rows.push(row(name, state, detail));
    }
    let separate_pid = status.namespaces[0] != 0 && status.namespaces[0] != browser[0];
    rows.push(row(
        "Can't see the window's process",
        if status.visible_processes == 0 {
            State::Unknown
        } else {
            known(Some(
                separate_pid && status.visible_processes <= MAX_VISIBLE_PROCESSES,
            ))
        },
        format!(
            "it sees {} processes in /proc, and its own pid is {}",
            status.visible_processes, status.pid
        ),
    ));
    rows.push(row(
        "User id map",
        if status.uid_map.is_empty() {
            State::Unknown
        } else {
            known(Some(
                status.namespaces[1] != 0 && status.namespaces[1] != browser[1],
            ))
        },
        if status.uid_map.is_empty() {
            "not readable".to_string()
        } else {
            status.uid_map.clone()
        },
    ));
    // The view may hold the system folders, a private /proc, /dev and /tmp,
    // and the folder the renderer executable is bound in from.
    let exe_top = decision
        .program
        .components()
        .find_map(|part| match part {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .unwrap_or("usr");
    let small_view = !status.root_entries.is_empty()
        && status.root_entries.split(' ').all(|name| {
            name == exe_top
                || ["proc", "dev", "tmp", "usr"].contains(&name)
                || SYSTEM_TOP_DIRS.iter().any(|dir| dir[1..] == *name)
        });
    rows.push(row(
        "Small filesystem view",
        if status.root_entries.is_empty() {
            State::Unknown
        } else {
            known(Some(small_view))
        },
        if status.root_entries.is_empty() {
            "not readable".to_string()
        } else {
            format!("it sees / as: {}", status.root_entries)
        },
    ));
    // Read-only only counts when the view is small too, so a host whose own
    // root happens to be a tmpfs doesn't pass.
    let host_read_only = small_view && only_private_mounts_writable(&status.writable_mounts);
    rows.push(row(
        "Host files read-only",
        if status.writable_mounts.is_empty() {
            State::Unknown
        } else {
            known(Some(host_read_only))
        },
        if status.writable_mounts.is_empty() {
            "not readable".to_string()
        } else if host_read_only {
            format!(
                "it can only write to its own private mounts: {}",
                status.writable_mounts
            )
        } else {
            format!("it can write to: {}", status.writable_mounts)
        },
    ));
    rows.push(row(
        "No new privileges",
        known(status.no_new_privs),
        match status.no_new_privs {
            Some(true) => "NoNewPrivs 1",
            Some(false) => "NoNewPrivs 0",
            None => "not readable",
        },
    ));
    rows.push(row(
        "System call filter (seccomp)",
        match status.seccomp {
            Some(2) => State::Active,
            Some(_) => State::NotActive,
            None => State::Unknown,
        },
        match status.seccomp {
            Some(mode) => format!("Seccomp mode {mode}"),
            None => "not readable".to_string(),
        },
    ));
    let [address, core, files, file_size, processes] = status.limits;
    rows.push(row(
        "Address space limit",
        address.map_or(State::Unknown, |v| known(Some(v != u64::MAX))),
        limit_text(address),
    ));
    rows.push(row(
        "No core files",
        core.map_or(State::Unknown, |v| known(Some(v == 0))),
        limit_text(core),
    ));
    rows.push(row(
        "Open file limit",
        files.map_or(State::Unknown, |v| known(Some(v <= 256))),
        limit_text(files),
    ));
    rows.push(row(
        "File size limit",
        match file_size {
            None => State::Unknown,
            Some(0) => State::Limited,
            Some(_) => State::NotActive,
        },
        match file_size {
            Some(0) => "0: writing data into a file fails, but it can still create empty files \
                        and folders, and truncate, rename and delete files it is allowed to \
                        change. Only a read-only view stops that."
                .to_string(),
            other => limit_text(other),
        },
    ));
    rows.push(row(
        "Process limit",
        State::NotActive,
        format!(
            "{} (counted for the whole user, so it is left alone)",
            limit_text(processes)
        ),
    ));
    if decision
        .in_use
        .as_ref()
        .is_some_and(|helper| helper.kind == ToolKind::LowIntegrity)
    {
        rows.push(row(
            "Low integrity level",
            State::Unknown,
            "renderers start from a copy with a low integrity label, but the level of the \
             running renderer can't be read back here",
        ));
    }
    rows
}

#[cfg(target_os = "linux")]
fn linux_own_status() -> RendererStatus {
    use crate::wire::{LIMITS, MAX_SHORT_TEXT_BYTES, MAX_UID_MAP_BYTES, NAMESPACES};
    use std::fs;

    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let mut namespaces = [0u64; 7];
    for (id, name) in namespaces.iter_mut().zip(NAMESPACES) {
        *id = fs::read_link(format!("/proc/self/ns/{name}"))
            .ok()
            .and_then(|link| parse_namespace_link(&link.to_string_lossy()))
            .unwrap_or(0);
    }
    let uid_map = fs::read_to_string("/proc/self/uid_map")
        .map(|map| first_line_squeezed(&map, MAX_UID_MAP_BYTES))
        .unwrap_or_default();
    let pid = fs::read_link("/proc/self")
        .ok()
        .and_then(|link| link.to_string_lossy().parse().ok())
        .unwrap_or(0);
    let visible_processes = fs::read_dir("/proc")
        .map(|dir| {
            dir.filter_map(Result::ok)
                .filter(|entry| is_pid_name(&entry.file_name().to_string_lossy()))
                .count()
        })
        .map(|count| u32::try_from(count).unwrap_or(u32::MAX))
        .unwrap_or(0);
    let mut names: Vec<String> = fs::read_dir("/")
        .map(|dir| {
            dir.filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let writable_mounts = fs::read_to_string("/proc/self/mountinfo")
        .map(|text| writable_mounts(&text, MAX_SHORT_TEXT_BYTES))
        .unwrap_or_default();
    let limits_text = fs::read_to_string("/proc/self/limits").unwrap_or_default();
    let mut limits = [None; 5];
    for (limit, name) in limits.iter_mut().zip(LIMITS) {
        *limit = parse_soft_limit(&limits_text, name);
    }
    RendererStatus {
        no_new_privs: status_field(&status, "NoNewPrivs:").and_then(|v| match v {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }),
        seccomp: status_field(&status, "Seccomp:")
            .and_then(|v| u8::try_from(v).ok())
            .filter(|mode| *mode <= 2),
        namespaces,
        uid_map,
        pid,
        visible_processes,
        root_entries: join_capped(&names, MAX_SHORT_TEXT_BYTES),
        writable_mounts,
        limits,
    }
}

/// The ids of this process's own namespaces, in [`NAMESPACES`](crate::wire::NAMESPACES) order, 0
/// where one can't be read. The browser compares a renderer's with these.
pub fn own_namespaces() -> [u64; 7] {
    read_own_status().namespaces
}

/// The number after `name` in a `/proc/<pid>/status` text.
pub fn status_field(status: &str, name: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with(name))?;
    line[name.len()..].trim().parse().ok()
}

/// The id in a namespace link such as `pid:[4026531836]`.
pub fn parse_namespace_link(link: &str) -> Option<u64> {
    let open = link.find(":[")?;
    let inner = link[open + 2..].strip_suffix(']')?;
    inner.parse().ok()
}

/// The soft value of limit `name` in a `/proc/<pid>/limits` text, with
/// `u64::MAX` for unlimited.
pub fn parse_soft_limit(limits: &str, name: &str) -> Option<u64> {
    let line = limits.lines().find(|line| {
        line.strip_prefix(name)
            .is_some_and(|rest| rest.starts_with(' '))
    })?;
    match line[name.len()..].split_whitespace().next()? {
        "unlimited" => Some(u64::MAX),
        value => value.parse().ok(),
    }
}

/// The mounts in a `/proc/<pid>/mountinfo` text that are mounted
/// read-write, as `<mount point>:<filesystem type>` separated by spaces,
/// each only once, in the kernel's order. `none` if every mount is
/// read-only. A list that doesn't fit in `cap` bytes is cut and ends in
/// ` ...`, so it can't be mistaken for a whole one.
pub fn writable_mounts(mountinfo: &str, cap: usize) -> String {
    let mut seen: Vec<String> = Vec::new();
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let (Some(point), Some(options)) = (fields.get(4), fields.get(5)) else {
            continue;
        };
        if !options.split(',').any(|option| option == "rw") {
            continue;
        }
        let kind = fields
            .iter()
            .position(|field| *field == "-")
            .and_then(|at| fields.get(at + 1))
            .unwrap_or(&"?");
        let entry = format!("{point}:{kind}");
        if !seen.contains(&entry) {
            seen.push(entry);
        }
    }
    if seen.is_empty() {
        return "none".to_string();
    }
    let whole = seen.join(" ");
    if whole.len() <= cap {
        return whole;
    }
    const MORE: &str = " ...";
    let mut out = join_capped(&seen, cap.saturating_sub(MORE.len()));
    out.push_str(MORE);
    out
}

#[cfg(target_os = "linux")]
fn is_pid_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(any(target_os = "linux", test))]
/// The first line of `text` with runs of spaces squeezed to one, cut to
/// `cap` bytes.
fn first_line_squeezed(text: &str, cap: usize) -> String {
    let line = text.lines().next().unwrap_or("");
    let squeezed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    cut(&squeezed, cap)
}

/// `names` joined by spaces, stopping before the text would pass `cap`.
fn join_capped(names: &[String], cap: usize) -> String {
    let mut out = String::new();
    for name in names {
        let extra = name.len() + usize::from(!out.is_empty());
        if out.len() + extra > cap {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(name);
    }
    out
}

#[cfg(any(target_os = "linux", test))]
fn cut(text: &str, cap: usize) -> String {
    let mut end = text.len().min(cap);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::MAX_SHORT_TEXT_BYTES;

    #[test]
    fn reads_status_fields() {
        let status = "Name:\tanvil\nNoNewPrivs:\t1\nSeccomp:\t0\nSeccomp_filters:\t0\n";
        assert_eq!(status_field(status, "NoNewPrivs:"), Some(1));
        assert_eq!(status_field(status, "Seccomp:"), Some(0));
        assert_eq!(status_field(status, "Missing:"), None);
    }

    #[test]
    fn reads_namespace_links() {
        assert_eq!(
            parse_namespace_link("pid:[4026531836]"),
            Some(4_026_531_836)
        );
        assert_eq!(parse_namespace_link("pid:4026531836"), None);
        assert_eq!(parse_namespace_link("pid:[x]"), None);
    }

    #[test]
    fn reads_soft_limits() {
        let limits = "Limit                     Soft Limit           Hard Limit           Units     \n\
                      Max file size             0                    unlimited            bytes     \n\
                      Max processes             unlimited            unlimited            processes \n\
                      Max open files            64                   64                   files     \n\
                      Max address space         268435456            unlimited            bytes     \n";
        assert_eq!(parse_soft_limit(limits, "Max file size"), Some(0));
        assert_eq!(parse_soft_limit(limits, "Max processes"), Some(u64::MAX));
        assert_eq!(parse_soft_limit(limits, "Max open files"), Some(64));
        assert_eq!(
            parse_soft_limit(limits, "Max address space"),
            Some(256 << 20)
        );
        assert_eq!(parse_soft_limit(limits, "Max core file size"), None);
    }

    #[test]
    fn long_lists_are_cut_to_fit() {
        let names: Vec<String> = (0..1000).map(|n| format!("entry{n}")).collect();
        let joined = join_capped(&names, MAX_SHORT_TEXT_BYTES);
        assert!(joined.len() <= MAX_SHORT_TEXT_BYTES);
        assert!(joined.starts_with("entry0 entry1 "));
        assert_eq!(
            first_line_squeezed("  0   1000   1\nnext\n", 256),
            "0 1000 1"
        );
        assert_eq!(cut("héllo", 2), "h");
    }

    #[test]
    fn lists_only_writable_mounts() {
        // As a renderer sees it through bwrap: a private root, /usr
        // read-only.
        let confined = "\
1 0 0:50 / / rw,nosuid,nodev,relatime - tmpfs tmpfs rw,uid=1000\n\
2 1 253:16 /usr /usr ro,nosuid,nodev,relatime - ext4 /dev/vdb rw\n\
3 1 0:51 / /proc rw,nosuid,nodev,noexec,relatime - proc proc rw\n\
4 1 0:52 / /tmp rw,nosuid,nodev,relatime shared:5 - tmpfs tmpfs rw\n\
5 1 0:53 / /tmp rw,nosuid,nodev,relatime - tmpfs tmpfs rw\n";
        assert_eq!(
            writable_mounts(confined, MAX_SHORT_TEXT_BYTES),
            "/:tmpfs /proc:proc /tmp:tmpfs"
        );
        // Every mount read-only.
        let read_only = "1 0 253:16 / / ro,relatime - ext4 /dev/vda ro\n";
        assert_eq!(writable_mounts(read_only, MAX_SHORT_TEXT_BYTES), "none");
        // Lines that aren't mountinfo are skipped.
        assert_eq!(writable_mounts("junk\n\n", MAX_SHORT_TEXT_BYTES), "none");
        // A long list is cut and says so.
        let mut many = String::new();
        for n in 0..500 {
            many.push_str(&format!("{n} 1 0:{n} / /m{n} rw - ext4 /dev/x rw\n"));
        }
        let cut_list = writable_mounts(&many, MAX_SHORT_TEXT_BYTES);
        assert!(cut_list.len() <= MAX_SHORT_TEXT_BYTES);
        assert!(cut_list.starts_with("/m0:ext4 /m1:ext4 "));
        assert!(cut_list.ends_with(" ..."));
    }

    #[test]
    fn modes_round_trip_through_their_names() {
        for mode in [ConfineMode::Auto, ConfineMode::Off] {
            assert_eq!(ConfineMode::from_slug(mode.slug()), mode);
        }
        assert_eq!(ConfineMode::from_slug("something"), ConfineMode::Auto);
        assert_eq!(ConfineMode::default(), ConfineMode::Auto);
    }

    #[cfg(unix)]
    #[test]
    fn tools_are_only_found_as_runnable_files_in_the_given_folders() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("anvil-confine-{}", std::process::id()));
        let first = base.join("first");
        let second = base.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        // Not runnable in the first folder, runnable in the second.
        std::fs::write(first.join("tool"), b"").unwrap();
        std::fs::write(second.join("tool"), b"").unwrap();
        let mode = |path: &Path, bits| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(bits)).unwrap()
        };
        mode(&first.join("tool"), 0o644);
        mode(&second.join("tool"), 0o755);
        // A folder with the name is not a tool.
        std::fs::create_dir_all(first.join("dir-tool")).unwrap();
        let dirs = [first.as_path(), second.as_path()];
        assert_eq!(find_tool_in(&dirs, "tool"), Some(second.join("tool")));
        assert_eq!(find_tool_in(&dirs, "dir-tool"), None);
        assert_eq!(find_tool_in(&dirs, "missing"), None);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn bwrap_gets_a_small_read_only_view() {
        let args = bwrap_args("/opt/anvil/anvil");
        let text = args.join(" ");
        for wanted in [
            "--unshare-all",
            "--die-with-parent",
            "--new-session",
            "--cap-drop ALL",
            "--ro-bind /usr /usr",
            "--ro-bind /opt/anvil/anvil /opt/anvil/anvil",
            "--proc /proc",
            "--dev /dev",
            "--tmpfs /tmp",
            "--chdir /",
        ] {
            assert!(text.contains(wanted), "{wanted} missing from {text}");
        }
        // Nothing is bound writable, and nothing from home or /etc.
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--bind" || arg == "--dev-bind")
        );
        assert!(!text.contains("/home") && !text.contains("/etc"));
        // A renderer under /usr is already in view.
        let under_usr = bwrap_args("/usr/bin/anvil").join(" ");
        assert!(!under_usr.contains("--ro-bind /usr/bin/anvil"));
    }

    #[test]
    fn the_renderer_is_bound_after_every_other_mount() {
        for program in ["/tmp/build/anvil", "/opt/anvil/anvil", "/dev/shm/anvil"] {
            let args = bwrap_args(program);
            let bound = args
                .windows(3)
                .position(|w| w == ["--ro-bind", program, program])
                .expect("the renderer is bound");
            let last_mount = args
                .iter()
                .rposition(|arg| {
                    matches!(
                        arg.as_str(),
                        "--ro-bind" | "--symlink" | "--proc" | "--dev" | "--tmpfs"
                    )
                })
                .unwrap();
            assert_eq!(bound, last_mount, "{program}: {}", args.join(" "));
        }
    }

    #[test]
    fn unshare_gets_every_namespace_and_a_private_proc() {
        let text = unshare_args().join(" ");
        for wanted in [
            "--user",
            "--map-current-user",
            "--pid",
            "--fork",
            "--kill-child",
            "--mount-proc",
            "--net",
            "--ipc",
            "--uts",
        ] {
            assert!(text.contains(wanted), "{wanted} missing from {text}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn wrapping_puts_prlimit_then_the_tool_in_front() {
        let exe = std::env::current_exe().unwrap();
        let plain = RendererCommand::new(&exe).arg("--renderer");
        let helper = Helper {
            kind: ToolKind::Bwrap,
            path: PathBuf::from("/usr/bin/bwrap"),
            prlimit: Some(PathBuf::from("/usr/bin/prlimit")),
        };
        let wrapped = helper.wrap(&plain).unwrap();
        assert!(wrapped.is_wrapped());
        assert_eq!(wrapped.args(), ["--renderer"]);
        let wrapping = wrapped.wrapper.as_ref().unwrap();
        assert_eq!(wrapping.program, PathBuf::from("/usr/bin/prlimit"));
        assert_eq!(&wrapping.args[..3], &PRLIMIT_ARGS);
        assert_eq!(wrapping.args[3], "--");
        assert_eq!(wrapping.args[4], "/usr/bin/bwrap");
        assert_eq!(wrapping.args.last().map(String::as_str), Some("--"));
        assert!(wrapping.signal_as_exit_code);
        // Without prlimit the tool itself is started.
        let bare = Helper {
            prlimit: None,
            kind: ToolKind::Unshare,
            path: PathBuf::from("/usr/bin/unshare"),
        };
        let wrapped = bare.wrap(&plain).unwrap();
        let wrapping = wrapped.wrapper.as_ref().unwrap();
        assert_eq!(wrapping.program, PathBuf::from("/usr/bin/unshare"));
        assert!(!wrapping.signal_as_exit_code);
        // A renderer that isn't there can't be wrapped.
        let missing = RendererCommand::new("/no/such/anvil");
        assert!(helper.wrap(&missing).is_err());
    }

    fn confined_status() -> RendererStatus {
        RendererStatus {
            no_new_privs: Some(true),
            seccomp: Some(0),
            namespaces: [11, 12, 13, 14, 15, 16, 17],
            uid_map: "1000 1000 1".to_string(),
            pid: 2,
            visible_processes: 2,
            root_entries: "bin dev lib lib64 proc tmp usr".to_string(),
            writable_mounts: "/:tmpfs /proc:proc /dev:tmpfs /tmp:tmpfs".to_string(),
            limits: [Some(256 << 20), Some(0), Some(256), Some(0), None],
        }
    }

    #[test]
    fn the_self_test_check_wants_every_namespace_separate() {
        let browser = [1, 2, 3, 4, 5, 6, 7];
        let good = confined_status();
        assert_eq!(check_status(ToolKind::Bwrap, &good, &browser), Ok(()));
        // Sharing any required namespace fails, naming it.
        for (index, name) in NAMESPACES.iter().enumerate().take(6) {
            let mut shared = good.clone();
            shared.namespaces[index] = browser[index];
            let err = check_status(ToolKind::Bwrap, &shared, &browser).unwrap_err();
            assert!(err.contains(name), "{err}");
        }
        // The cgroup namespace is not required.
        let mut cgroup = good.clone();
        cgroup.namespaces[6] = browser[6];
        assert_eq!(check_status(ToolKind::Unshare, &cgroup, &browser), Ok(()));
        // Unknown counts as failed.
        let mut unknown = good.clone();
        unknown.namespaces[0] = 0;
        assert!(check_status(ToolKind::Bwrap, &unknown, &browser).is_err());
        // Seeing the host's processes fails.
        let mut sees = good.clone();
        sees.visible_processes = 300;
        assert!(check_status(ToolKind::Bwrap, &sees, &browser).is_err());
        // NoNewPrivs is always wanted.
        let mut privs = good.clone();
        privs.no_new_privs = None;
        assert!(check_status(ToolKind::LowIntegrity, &privs, &browser).is_err());
        assert_eq!(
            check_status(ToolKind::LowIntegrity, &good, &browser),
            Ok(())
        );
    }

    #[test]
    fn turned_off_means_plain_renderers_and_says_why() {
        let plain = RendererCommand::new("/no/such/anvil").arg("--renderer");
        let decision = decide(ConfineMode::Off, &plain, &RendererLimits::default());
        assert!(decision.in_use.is_none());
        assert!(!decision.command_for(&plain).is_wrapped());
        assert!(
            decision.notes[0].contains("turned off"),
            "{:?}",
            decision.notes
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_tool_that_fails_its_self_test_is_not_used_and_says_why() {
        // There is no renderer at this path, so every tool fails.
        let plain = RendererCommand::new("/no/such/anvil").arg("--renderer");
        let decision = decide(ConfineMode::Auto, &plain, &RendererLimits::default());
        assert!(decision.in_use.is_none());
        assert!(!decision.command_for(&plain).is_wrapped());
        assert!(!decision.notes.is_empty());
        if !decision.found.is_empty() {
            assert!(
                decision.notes[0].contains("failed its self-test"),
                "{:?}",
                decision.notes
            );
        }
    }

    fn state_of(rows: &[Protection], name: &str) -> State {
        rows.iter()
            .find(|row| row.name == name)
            .map(|row| row.state)
            .unwrap_or_else(|| panic!("no row {name}"))
    }

    #[test]
    fn the_table_shows_what_the_renderer_read() {
        let browser = [1, 2, 3, 4, 5, 6, 7];
        let decision = Confinement {
            in_use: Some(Helper {
                kind: ToolKind::Bwrap,
                path: PathBuf::from("/usr/bin/bwrap"),
                prlimit: Some(PathBuf::from("/usr/bin/prlimit")),
            }),
            program: PathBuf::from("/home/someone/.local/bin/anvil"),
            ..Confinement::default()
        };
        // The executable's own top folder is part of a small view.
        let status = RendererStatus {
            root_entries: "bin dev home lib lib64 proc sbin tmp usr".to_string(),
            ..confined_status()
        };
        let rows = protections(&decision, Ok(&status), &browser);
        for name in [
            "Started through a confinement tool",
            "Own user namespace",
            "Own pid namespace",
            "Own network namespace",
            "Can't see the window's process",
            "Small filesystem view",
            "Host files read-only",
            "No new privileges",
        ] {
            assert_eq!(state_of(&rows, name), State::Active, "{name}");
        }
        // A file size limit of 0 is never shown as stopping every write.
        assert_eq!(state_of(&rows, "File size limit"), State::Limited);
        // There is no seccomp filter, and the page says so.
        assert_eq!(
            state_of(&rows, "System call filter (seccomp)"),
            State::NotActive
        );
        assert_eq!(state_of(&rows, "Process limit"), State::NotActive);
    }

    #[test]
    fn file_writes_are_only_stopped_by_a_read_only_view() {
        let browser = [1, 2, 3, 4, 5, 6, 7];
        let tool = |kind| Confinement {
            in_use: Some(Helper {
                kind,
                path: PathBuf::from("/usr/bin/tool"),
                prlimit: Some(PathBuf::from("/usr/bin/prlimit")),
            }),
            program: PathBuf::from("/usr/bin/anvil"),
            ..Confinement::default()
        };
        let line = |rows: &[Protection], name: &str| {
            rows.iter()
                .find(|row| row.name == name)
                .cloned()
                .unwrap_or_else(|| panic!("no row {name}"))
        };

        // bwrap: only private mounts are writable, so host files are
        // read-only. The size limit alone is still only Limited.
        let rows = protections(&tool(ToolKind::Bwrap), Ok(&confined_status()), &browser);
        assert_eq!(state_of(&rows, "Host files read-only"), State::Active);
        assert_eq!(state_of(&rows, "File size limit"), State::Limited);
        let size = line(&rows, "File size limit");
        assert!(
            size.detail.contains("truncate, rename and delete"),
            "{size:?}"
        );

        // unshare: own namespaces, but the host's files are all there and
        // writable. A file size limit of 0 must not read as stopping writes.
        let unshared = RendererStatus {
            root_entries: "bin boot etc home lib proc tmp usr var".to_string(),
            writable_mounts: "/:ext4 /proc:proc /dev:devtmpfs /tmp:tmpfs".to_string(),
            ..confined_status()
        };
        let rows = protections(&tool(ToolKind::Unshare), Ok(&unshared), &browser);
        assert_eq!(state_of(&rows, "Host files read-only"), State::NotActive);
        assert!(
            line(&rows, "Host files read-only")
                .detail
                .contains("/:ext4")
        );
        assert_eq!(state_of(&rows, "File size limit"), State::Limited);
        // No line about writing files reads Active.
        for name in ["Host files read-only", "File size limit"] {
            assert_ne!(state_of(&rows, name), State::Active, "{name}");
        }

        // Off: no size limit and the host's files writable.
        let plain = Confinement {
            notes: vec!["confinement is turned off in the settings".to_string()],
            ..Confinement::default()
        };
        let shared = RendererStatus {
            namespaces: browser,
            limits: [Some(256 << 20), Some(0), Some(1024), Some(u64::MAX), None],
            ..unshared.clone()
        };
        let rows = protections(&plain, Ok(&shared), &browser);
        assert_eq!(state_of(&rows, "Host files read-only"), State::NotActive);
        assert_eq!(state_of(&rows, "File size limit"), State::NotActive);

        // Not claimed when the list was cut, when only the root is a tmpfs
        // but the view is the host's, or when it couldn't be read.
        for (root_entries, writable_mounts, want) in [
            (
                "bin dev lib proc tmp usr",
                "/:tmpfs /proc:proc ...",
                State::NotActive,
            ),
            (
                "bin etc home proc usr var",
                "/:tmpfs /proc:proc",
                State::NotActive,
            ),
            ("bin dev lib proc tmp usr", "", State::Unknown),
            ("bin dev lib proc tmp usr", "none", State::Active),
        ] {
            let status = RendererStatus {
                root_entries: root_entries.to_string(),
                writable_mounts: writable_mounts.to_string(),
                ..confined_status()
            };
            let rows = protections(&tool(ToolKind::Bwrap), Ok(&status), &browser);
            assert_eq!(
                state_of(&rows, "Host files read-only"),
                want,
                "{root_entries} / {writable_mounts}"
            );
        }
    }

    #[test]
    fn the_table_never_claims_what_it_could_not_read() {
        let browser = [1, 2, 3, 4, 5, 6, 7];
        let plain = Confinement {
            notes: vec!["neither bwrap nor unshare was found".to_string()],
            ..Confinement::default()
        };
        // A plain renderer shares everything with the window.
        let shared = RendererStatus {
            namespaces: browser,
            visible_processes: 300,
            root_entries: "bin etc home proc usr".to_string(),
            writable_mounts: "/:ext4 /proc:proc /dev:devtmpfs /dev/shm:tmpfs".to_string(),
            limits: [Some(256 << 20), Some(0), Some(1024), Some(u64::MAX), None],
            ..confined_status()
        };
        let rows = protections(&plain, Ok(&shared), &browser);
        assert_eq!(
            state_of(&rows, "Started through a confinement tool"),
            State::NotActive
        );
        assert!(rows[0].detail.contains("neither bwrap"));
        for name in [
            "Own user namespace",
            "Own pid namespace",
            "Can't see the window's process",
            "Small filesystem view",
            "Host files read-only",
            "Open file limit",
            "File size limit",
        ] {
            assert_eq!(state_of(&rows, name), State::NotActive, "{name}");
        }
        // The worker limits the renderer sets on itself still show.
        assert_eq!(state_of(&rows, "Address space limit"), State::Active);
        // Nothing readable: every renderer line is unknown, none active.
        let rows = protections(&plain, Ok(&RendererStatus::default()), &browser);
        assert!(
            rows.iter().skip(1).all(|row| row.state != State::Active),
            "{rows:?}"
        );
        // No renderer at all: one unknown line with the reason.
        let rows = protections(&plain, Err("it did not start"), &browser);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].state, State::Unknown);
        assert!(rows[1].detail.contains("it did not start"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_this_process() {
        let status = read_own_status();
        assert!(status.no_new_privs.is_some());
        assert!(status.seccomp.is_some());
        assert!(status.namespaces.iter().take(6).all(|id| *id != 0));
        assert!(!status.uid_map.is_empty());
        assert_eq!(status.pid, std::process::id());
        assert!(status.visible_processes >= 1);
        assert!(status.root_entries.split(' ').any(|name| name == "proc"));
        assert!(!status.writable_mounts.is_empty());
        assert!(status.limits[0].is_some());
        // The status always fits on the wire.
        assert!(crate::wire::encode(0, &crate::wire::Message::Status(status)).is_ok());
    }
}
