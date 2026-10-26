// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! What the OS has put a renderer under, read back from the OS.
//!
//! [`read_own_status`] runs in the renderer. It reads its own entries in
//! `/proc/self` with plain file reads and fills a [`RendererStatus`], which
//! the browser asks for over the wire and shows on its security page.
//! Nothing here sets a limit. Where a value can't be read it stays unknown,
//! so the page never has to guess.

#![forbid(unsafe_code)]

use crate::wire::RendererStatus;

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
