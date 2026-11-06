// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 Buntato
//! Unified diagnostic error catalog (`ERROR ID-1` through `ERROR ID-31`) for Anvil Browser.
//!
//! # Architecture Overview
//! Every lifecycle exit, crash, sandbox containment event, renderer failure, network
//! transport error, and HTTP status error in Anvil Browser maps to a deterministic,
//! human-readable `ERROR ID-<N>` identifier.
//!
//! - **`ERROR ID-1`** is reserved for normal, user-initiated browser closure (closing
//!   the window, clicking the close caption button, or pressing `Alt+F4` / `Ctrl+Q`).
//! - **`ERROR ID-2`** is reserved for an unexpected fatal crash or panic in the main
//!   browser process.
//! - **`ERROR ID-3` .. `ERROR ID-13`** cover OS sandbox initialization, `--renderer`
//!   worker confinement, signals (such as `SIGSEGV` or `SIGXFSZ`), and timeouts.
//! - **`ERROR ID-14` .. `ERROR ID-25`** cover DNS, TCP, TLS 1.3 / PQC handshakes,
//!   HTTPS-Only policy, Shield blocks, HTTP 4xx/5xx responses, and local file I/O.
//! - **`ERROR ID-26` .. `ERROR ID-30`** cover desktop windowing (Wayland/X11/Win32),
//!   GPU context fallback, encrypted profile storage, media decoding, and setup packaging.
//! - **`ERROR ID-31`** covers the sealed saved-logins file (`logins.anvil`).
//!
//! Whenever a navigation fails or a process exits, [`classify_error_reason`] inspects
//! the underlying failure string and attaches the matching [`ErrorId`] so the user can
//! look up the exact cause and fix in `Anvil-Browser-Error-Codes-Manual.txt` or at
//! `anvil://errors`.

use std::fmt;

// ============================================================================
// Harmless Lore & Easter Egg Constants (Silent Hill 2 & One Piece)
// ============================================================================

/// Silent Hill 2 Easter Egg:
///
/// > *"In my restless dreams, I see that town... Silent Hill.*
/// > *You promised you'd take me there again someday, but you never did.*
/// > *Well, I'm alone there now... In our 'special place'... Waiting for you..."*
///
/// And the infamous chalk message on the boarded-up window of Neely's Bar on
/// Lindsey Street in South Vale:
///
/// > *"There was a HOLE here. It's gone now."*
///
/// This constant is completely inert during normal browsing and is surfaced
/// only when inspecting the easter egg or visiting `anvil://restless-dreams`
/// (or `anvil://room-312`, Lakeview Hotel).
pub const SILENT_HILL_2_RESTLESS_DREAMS: &str = "In my restless dreams, I see that town... Silent Hill. \
     Waiting for you in Room 312 of the Lakeview Hotel. \
     (Neely's Bar notice: \"There was a HOLE here. It's gone now.\")";

/// One Piece Easter Egg:
///
/// > *"Wealth, fame, power. The man who had acquired everything in this world,*
/// > *the Pirate King, Gol D. Roger. The final words he said at his execution*
/// > *sent people to the seas:*
/// > *'My treasure? If you want it, I'll let you have it! Go look for it!*
/// > *I left all of it at that place!'"*
///
/// And the Poneglyph inscription on Laugh Tale at the end of the New World:
///
/// > *"Inherited Will, the Destiny of the Age, and the Dreams of the People.*
/// > *As long as people continue to pursue the meaning of Freedom, these things*
/// > *will never cease to be!"*
///
/// This constant is completely inert during normal browsing and is surfaced
/// only when inspecting the easter egg or visiting `anvil://laugh-tale`
/// (or `anvil://grand-line`).
pub const ONE_PIECE_LAUGH_TALE_LOG_POSE: &str = "Log Pose locked onto Laugh Tale at the end of the Grand Line! \
     \"Inherited Will, the Destiny of the Age, and the Dreams of the People \
     will never cease to be!\" — Thousand Sunny navigation log.";

// ============================================================================
// Error Code Enumeration (`ERROR ID-1` .. `ERROR ID-31`)
// ============================================================================

/// Numeric and symbolic identifier for every Anvil Browser status or failure condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorId {
    /// `ERROR ID-1`: The user closed the browser normally (window close button, `Alt+F4`, or `Ctrl+Q`).
    UserClosedBrowser = 1,
    /// `ERROR ID-2`: The main browser process crashed due to an unhandled panic or fatal exception.
    BrowserCrashed = 2,
    /// `ERROR ID-3`: Mandatory OS sandbox guard initialization failed at startup (exit code 101).
    SandboxInitFailed = 3,
    /// `ERROR ID-4`: The `--renderer` worker process failed to lock down its 256 MiB sandbox limits (exit code 102).
    WorkerSandboxLockFailed = 4,
    /// `ERROR ID-5`: The browser could not launch the `--renderer` child process or confinement helper.
    RendererStartFailed = 5,
    /// `ERROR ID-6`: The `--renderer` worker process exited unexpectedly with a non-zero exit code.
    RendererStopped = 6,
    /// `ERROR ID-7`: The `--renderer` worker process crashed due to a fatal OS signal (e.g. `SIGSEGV`, `SIGABRT`, `SIGXFSZ`).
    RendererCrashedSignal = 7,
    /// `ERROR ID-8`: The `--renderer` worker process stopped responding to IPC or heartbeat checks.
    RendererTimedOut = 8,
    /// `ERROR ID-9`: The `--renderer` worker took too long to finish rendering the current page.
    RendererPageTooSlow = 9,
    /// `ERROR ID-10`: The tab's `--renderer` worker crashed repeatedly and is in crash-loop cooldown.
    RendererCrashCooldown = 10,
    /// `ERROR ID-11`: The `--renderer` worker refused the page request (document or URL exceeded wire caps).
    RendererRefusedPage = 11,
    /// `ERROR ID-12`: The `--renderer` worker returned a malformed display-list frame that failed verification.
    RendererBadFrame = 12,
    /// `ERROR ID-13`: An in-process tab workload panicked or faulted and was contained by the sandbox guard.
    TabSandboxIsolatedFault = 13,
    /// `ERROR ID-14`: DNS lookup failed; the domain name could not be resolved to an IP address.
    DnsResolutionFailed = 14,
    /// `ERROR ID-15`: TCP socket connection was refused or the destination host/port is unreachable.
    TcpConnectionFailed = 15,
    /// `ERROR ID-16`: Network socket read or write timed out waiting for the remote server.
    NetworkTimeout = 16,
    /// `ERROR ID-17`: TLS 1.3 or hybrid `X25519MLKEM768` handshake or certificate verification failed.
    TlsHandshakeFailed = 17,
    /// `ERROR ID-18`: HTTPS-Only mode blocked a plaintext `http://` navigation that could not be upgraded.
    HttpsOnlyBlocked = 18,
    /// `ERROR ID-19`: Navigation or resource was blocked by Anvil Shield (`adblock-rust`), CSP, CORP, or Same-Origin Policy.
    ShieldOrPolicyBlocked = 19,
    /// `ERROR ID-20`: The target URL is malformed or uses an unsupported URI scheme.
    InvalidUrlOrScheme = 20,
    /// `ERROR ID-21`: The server redirected too many times (redirect loop detected).
    TooManyRedirects = 21,
    /// `ERROR ID-22`: The remote server returned an HTTP `4xx` client error (such as `403 Forbidden` or `404 Not Found`).
    HttpClientError4xx = 22,
    /// `ERROR ID-23`: The remote server returned an HTTP `5xx` server error (such as `500`, `502`, `503`, or `504`).
    HttpServerError5xx = 23,
    /// `ERROR ID-24`: The server returned a malformed HTTP response or a non-UTF-8 binary body that is not a page.
    InvalidHttpResponse = 24,
    /// `ERROR ID-25`: A local file (`file://` or disk path) could not be found or read.
    LocalFileReadFailed = 25,
    /// `ERROR ID-26`: Desktop windowing (Wayland, X11, or Win32) failed to connect or create a surface.
    DisplayServerUnavailable = 26,
    /// `ERROR ID-27`: Hardware GPU context creation failed; Anvil switched to the multi-threaded CPU SIMD rasterizer.
    GpuFallbackActive = 27,
    /// `ERROR ID-28`: The encrypted `profile.anvil` vault could not be decrypted or saved.
    ProfileVaultError = 28,
    /// `ERROR ID-29`: An image, font, or video stream exceeded safety caps or contained corrupt data.
    MediaDecodeError = 29,
    /// `ERROR ID-30`: The setup installer failed to extract or write the browser executable.
    InstallerError = 30,
    /// `ERROR ID-31`: The sealed saved-logins file could not be opened or written.
    SavedLoginsError = 31,
}

/// Complete static list of all 31 defined [`ErrorId`] variants in numeric order.
pub const ALL_ERROR_IDS: [ErrorId; 31] = [
    ErrorId::UserClosedBrowser,
    ErrorId::BrowserCrashed,
    ErrorId::SandboxInitFailed,
    ErrorId::WorkerSandboxLockFailed,
    ErrorId::RendererStartFailed,
    ErrorId::RendererStopped,
    ErrorId::RendererCrashedSignal,
    ErrorId::RendererTimedOut,
    ErrorId::RendererPageTooSlow,
    ErrorId::RendererCrashCooldown,
    ErrorId::RendererRefusedPage,
    ErrorId::RendererBadFrame,
    ErrorId::TabSandboxIsolatedFault,
    ErrorId::DnsResolutionFailed,
    ErrorId::TcpConnectionFailed,
    ErrorId::NetworkTimeout,
    ErrorId::TlsHandshakeFailed,
    ErrorId::HttpsOnlyBlocked,
    ErrorId::ShieldOrPolicyBlocked,
    ErrorId::InvalidUrlOrScheme,
    ErrorId::TooManyRedirects,
    ErrorId::HttpClientError4xx,
    ErrorId::HttpServerError5xx,
    ErrorId::InvalidHttpResponse,
    ErrorId::LocalFileReadFailed,
    ErrorId::DisplayServerUnavailable,
    ErrorId::GpuFallbackActive,
    ErrorId::ProfileVaultError,
    ErrorId::MediaDecodeError,
    ErrorId::InstallerError,
    ErrorId::SavedLoginsError,
];

impl ErrorId {
    /// Numeric ID (`1` through `31`).
    #[inline]
    pub const fn number(self) -> u16 {
        self as u16
    }

    /// Canonical code string formatted as `"ERROR ID-<N>"` (for example, `"ERROR ID-1"`).
    pub const fn code(self) -> &'static str {
        match self {
            Self::UserClosedBrowser => "ERROR ID-1",
            Self::BrowserCrashed => "ERROR ID-2",
            Self::SandboxInitFailed => "ERROR ID-3",
            Self::WorkerSandboxLockFailed => "ERROR ID-4",
            Self::RendererStartFailed => "ERROR ID-5",
            Self::RendererStopped => "ERROR ID-6",
            Self::RendererCrashedSignal => "ERROR ID-7",
            Self::RendererTimedOut => "ERROR ID-8",
            Self::RendererPageTooSlow => "ERROR ID-9",
            Self::RendererCrashCooldown => "ERROR ID-10",
            Self::RendererRefusedPage => "ERROR ID-11",
            Self::RendererBadFrame => "ERROR ID-12",
            Self::TabSandboxIsolatedFault => "ERROR ID-13",
            Self::DnsResolutionFailed => "ERROR ID-14",
            Self::TcpConnectionFailed => "ERROR ID-15",
            Self::NetworkTimeout => "ERROR ID-16",
            Self::TlsHandshakeFailed => "ERROR ID-17",
            Self::HttpsOnlyBlocked => "ERROR ID-18",
            Self::ShieldOrPolicyBlocked => "ERROR ID-19",
            Self::InvalidUrlOrScheme => "ERROR ID-20",
            Self::TooManyRedirects => "ERROR ID-21",
            Self::HttpClientError4xx => "ERROR ID-22",
            Self::HttpServerError5xx => "ERROR ID-23",
            Self::InvalidHttpResponse => "ERROR ID-24",
            Self::LocalFileReadFailed => "ERROR ID-25",
            Self::DisplayServerUnavailable => "ERROR ID-26",
            Self::GpuFallbackActive => "ERROR ID-27",
            Self::ProfileVaultError => "ERROR ID-28",
            Self::MediaDecodeError => "ERROR ID-29",
            Self::InstallerError => "ERROR ID-30",
            Self::SavedLoginsError => "ERROR ID-31",
        }
    }

    /// Short, human-friendly title summarizing the condition.
    pub const fn title(self) -> &'static str {
        match self {
            Self::UserClosedBrowser => "User Closed the Browser",
            Self::BrowserCrashed => "Browser Process Crashed",
            Self::SandboxInitFailed => "Mandatory OS Sandbox Initialization Failed",
            Self::WorkerSandboxLockFailed => "Tab Worker Sandbox Lockdown Failed",
            Self::RendererStartFailed => "Renderer Process Failed to Start",
            Self::RendererStopped => "Renderer Process Stopped Unexpectedly",
            Self::RendererCrashedSignal => "Renderer Process Crashed (OS Signal)",
            Self::RendererTimedOut => "Renderer Process Stopped Responding",
            Self::RendererPageTooSlow => "Renderer Page Time Budget Exceeded",
            Self::RendererCrashCooldown => "Renderer Crash-Loop Cooldown Active",
            Self::RendererRefusedPage => "Renderer Refused Oversized or Invalid Page",
            Self::RendererBadFrame => "Renderer Sent Invalid Display Frame",
            Self::TabSandboxIsolatedFault => "Tab Fault Contained by Mandatory Sandbox",
            Self::DnsResolutionFailed => "DNS Hostname Resolution Failed",
            Self::TcpConnectionFailed => "TCP Connection Refused or Unreachable",
            Self::NetworkTimeout => "Network Socket Timed Out",
            Self::TlsHandshakeFailed => "TLS 1.3 / PQC Handshake or Certificate Failed",
            Self::HttpsOnlyBlocked => "HTTPS-Only Mode Blocked Plaintext HTTP",
            Self::ShieldOrPolicyBlocked => "Blocked by Anvil Shield or Security Policy",
            Self::InvalidUrlOrScheme => "Invalid or Unsupported URL",
            Self::TooManyRedirects => "Too Many HTTP Redirects",
            Self::HttpClientError4xx => "HTTP 4xx Client Error",
            Self::HttpServerError5xx => "HTTP 5xx Server Error",
            Self::InvalidHttpResponse => "Malformed HTTP Response or Non-Text Body",
            Self::LocalFileReadFailed => "Local File Could Not Be Read",
            Self::DisplayServerUnavailable => "Desktop Display Server Unavailable",
            Self::GpuFallbackActive => "GPU Context Unavailable (CPU SIMD Fallback)",
            Self::ProfileVaultError => "Encrypted Profile Vault Error",
            Self::MediaDecodeError => "Image, Font, or Media Decode Error",
            Self::InstallerError => "Setup / Installer Package Error",
            Self::SavedLoginsError => "Saved Logins Could Not Be Opened",
        }
    }

    /// Detailed plain-English explanation of what caused this error ID and how to resolve it.
    pub const fn explanation(self) -> &'static str {
        match self {
            Self::UserClosedBrowser => {
                "The user closed the browser window normally (window close button, Alt+F4, or Ctrl+Q). No fault occurred."
            }
            Self::BrowserCrashed => {
                "The main browser process encountered an unhandled panic or fatal exception and terminated."
            }
            Self::SandboxInitFailed => {
                "The main process could not apply its mandatory OS sandbox guard (Windows Job Object or Linux PR_SET_NO_NEW_PRIVS) and exited with status 101."
            }
            Self::WorkerSandboxLockFailed => {
                "A --renderer child process could not apply its 256 MiB address-space cap or single-process Job Object restriction and exited with status 102."
            }
            Self::RendererStartFailed => {
                "The browser process could not spawn the --renderer binary or its bwrap/unshare confinement wrapper."
            }
            Self::RendererStopped => {
                "The --renderer child process exited with a non-zero exit status while rendering the page."
            }
            Self::RendererCrashedSignal => {
                "The --renderer child process was killed by an OS signal (such as SIGSEGV=11, SIGABRT=6, SIGKILL=9, or SIGXFSZ=25) without affecting the main browser window."
            }
            Self::RendererTimedOut => {
                "The --renderer child process did not complete its startup handshake or stopped replying to IPC messages within the timeout window."
            }
            Self::RendererPageTooSlow => {
                "Rendering the document exceeded the per-navigation wall-clock budget; the slow renderer was terminated."
            }
            Self::RendererCrashCooldown => {
                "This tab's renderer crashed multiple times in a short window; Anvil paused automatic relaunches briefly to prevent a crash loop."
            }
            Self::RendererRefusedPage => {
                "The HTML document, URL, or subresource count exceeded wire protocol safety caps, so the renderer refused to process it."
            }
            Self::RendererBadFrame => {
                "The display-list frame returned by the renderer failed bounds, layer-depth, or link verification in the browser process."
            }
            Self::TabSandboxIsolatedFault => {
                "An in-process tab operation panicked or returned an error and was isolated by MandatorySandboxGuard::execute_isolated."
            }
            Self::DnsResolutionFailed => {
                "The operating system resolver could not find an IPv4 or IPv6 address for the target domain name."
            }
            Self::TcpConnectionFailed => {
                "The TCP connection to the remote host was refused, reset, or could not reach the destination network."
            }
            Self::NetworkTimeout => {
                "The network socket timed out while connecting, sending the HTTP request, or waiting for response bytes."
            }
            Self::TlsHandshakeFailed => {
                "The TLS 1.3 (X25519MLKEM768 / X25519) handshake failed, or the server presented an untrusted, expired, or mismatched X.509 certificate."
            }
            Self::HttpsOnlyBlocked => {
                "HTTPS-Only mode prevented loading a plaintext http:// page because the server did not accept an encrypted https:// upgrade."
            }
            Self::ShieldOrPolicyBlocked => {
                "The request was blocked by Anvil Shield (adblock-rust), Content-Security-Policy (CSP), Cross-Origin-Resource-Policy (CORP), or Same-Origin Policy."
            }
            Self::InvalidUrlOrScheme => {
                "The address entered could not be parsed as a valid http://, https://, file://, or anvil:// URL."
            }
            Self::TooManyRedirects => {
                "The server sent a chain of HTTP 3xx redirects exceeding the maximum hop limit."
            }
            Self::HttpClientError4xx => {
                "The remote web server returned an HTTP 4xx client error status (e.g. 400 Bad Request, 403 Forbidden, 404 Not Found, or 429 Too Many Requests)."
            }
            Self::HttpServerError5xx => {
                "The remote web server returned an HTTP 5xx server error status (e.g. 500 Internal Server Error, 502 Bad Gateway, 503 Service Unavailable, or 504 Gateway Timeout)."
            }
            Self::InvalidHttpResponse => {
                "The server sent malformed HTTP headers, broken chunked transfer encoding, or a non-UTF-8 binary payload that is not a web document."
            }
            Self::LocalFileReadFailed => {
                "The requested local file path does not exist or the current user lacks permission to read it."
            }
            Self::DisplayServerUnavailable => {
                "No Wayland compositor socket, X11 display, or Win32 desktop session was available to open the browser window."
            }
            Self::GpuFallbackActive => {
                "Hardware EGL/OpenGL or Direct2D initialization was unavailable; rendering continues seamlessly on the multi-threaded CPU SIMD rasterizer."
            }
            Self::ProfileVaultError => {
                "The AES-256-GCM encrypted profile vault (profile.anvil) could not be read, authenticated, or written."
            }
            Self::MediaDecodeError => {
                "An image, SFNT font, or video container exceeded dimension limits or contained invalid bitstream bytes."
            }
            Self::InstallerError => {
                "The setup binary could not locate its embedded payload or failed to write the installed executable."
            }
            Self::SavedLoginsError => {
                "The sealed logins.anvil file next to your profile could not be opened, failed its seal check, or could not be written. Nothing from it was shown, and no login was written anywhere else."
            }
        }
    }
}

impl fmt::Display for ErrorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.code(), self.title())
    }
}

/// Inspect a human-readable navigation, renderer, sandbox, or network failure
/// string and map it deterministically to its corresponding [`ErrorId`].
///
/// This function is used by [`crate::navigation_error_page`] and diagnostic logs
/// so every error presented to the user carries an easy-to-reference `ERROR ID-<N>`.
pub fn classify_error_reason(reason: &str) -> ErrorId {
    let lower = reason.to_ascii_lowercase();

    if lower.contains("saved logins") {
        return ErrorId::SavedLoginsError;
    }

    // 1. Renderer process & confinement errors (ERROR ID-5 .. ERROR ID-12)
    if lower.contains("crashed (signal") || lower.contains("killed by signal") {
        return ErrorId::RendererCrashedSignal;
    }
    if lower.contains("stopped responding") || lower.contains("renderer timed out") {
        return ErrorId::RendererTimedOut;
    }
    if lower.contains("took too long over this page") {
        return ErrorId::RendererPageTooSlow;
    }
    if lower.contains("crashed too many times") {
        return ErrorId::RendererCrashCooldown;
    }
    if lower.contains("could not check") || lower.contains("bad frame") {
        return ErrorId::RendererBadFrame;
    }
    if lower.contains("could not finish the page")
        || lower.contains("too large for the renderer")
        || lower.contains("address is too long")
        || lower.contains("header is too long")
    {
        return ErrorId::RendererRefusedPage;
    }
    if lower.contains("could not start") || lower.contains("failed to spawn") {
        return ErrorId::RendererStartFailed;
    }
    if lower.contains("the renderer stopped")
        || lower.contains("tab worker rejected")
        || lower.contains("renderer table is unavailable")
    {
        return ErrorId::RendererStopped;
    }

    // 2. Sandbox containment & initialization (ERROR ID-3, 4, 13)
    if lower.contains("worker sandbox setup failed") || lower.contains("exit code 102") {
        return ErrorId::WorkerSandboxLockFailed;
    }
    if lower.contains("sandbox setup failed") || lower.contains("exit code 101") {
        return ErrorId::SandboxInitFailed;
    }
    if lower.contains("tab isolated") || lower.contains("panicked") {
        return ErrorId::TabSandboxIsolatedFault;
    }

    // 3. HTTP 4xx and 5xx status codes (ERROR ID-22, 23)
    if let Some(rest) = lower.strip_prefix("the server returned ")
        && let Ok(code) = rest.trim().parse::<u16>()
    {
        if (400..500).contains(&code) {
            return ErrorId::HttpClientError4xx;
        }
        if (500..600).contains(&code) {
            return ErrorId::HttpServerError5xx;
        }
        return ErrorId::InvalidHttpResponse;
    }

    // 4. Local file read errors (ERROR ID-25)
    if lower.starts_with("could not read ") || lower.contains("no such file") {
        return ErrorId::LocalFileReadFailed;
    }

    // 5. Network, DNS, TLS, Shield, Redirects, URL (ERROR ID-14 .. ERROR ID-21, 24)
    if lower.contains("https-only") {
        return ErrorId::HttpsOnlyBlocked;
    }
    if lower.contains("blocked by")
        || lower.contains("shield")
        || lower.contains("same-origin")
        || lower.contains("phishing")
        || lower.contains("cross-origin")
    {
        return ErrorId::ShieldOrPolicyBlocked;
    }
    if lower.contains("redirect") {
        return ErrorId::TooManyRedirects;
    }
    if lower.contains("certificate")
        || lower.contains("tls")
        || lower.contains("handshake")
        || lower.contains("key exchange")
    {
        return ErrorId::TlsHandshakeFailed;
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return ErrorId::NetworkTimeout;
    }
    if lower.contains("resolve")
        || lower.contains("dns")
        || lower.contains("unknown host")
        || lower.contains("name or service not known")
    {
        return ErrorId::DnsResolutionFailed;
    }
    if lower.contains("connect")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("unreachable")
    {
        return ErrorId::TcpConnectionFailed;
    }
    if lower.contains("scheme") || lower.contains("invalid url") || lower.contains("malformed url")
    {
        return ErrorId::InvalidUrlOrScheme;
    }
    if lower.contains("was not text") || lower.contains("utf-8") || lower.contains("http") {
        return ErrorId::InvalidHttpResponse;
    }

    ErrorId::RendererStopped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_ids_are_numbered_sequentially_from_1_to_31() {
        for (idx, id) in ALL_ERROR_IDS.iter().enumerate() {
            let expected_num = (idx + 1) as u16;
            assert_eq!(id.number(), expected_num);
            assert_eq!(id.code(), format!("ERROR ID-{expected_num}"));
            assert!(!id.title().is_empty());
            assert!(!id.explanation().is_empty());
        }
        assert_eq!(ErrorId::UserClosedBrowser.code(), "ERROR ID-1");
        assert_eq!(ErrorId::BrowserCrashed.code(), "ERROR ID-2");
        assert_eq!(ErrorId::SavedLoginsError.code(), "ERROR ID-31");
        for err in [
            crate::logins::LoginsError::Unreadable,
            crate::logins::LoginsError::Damaged,
            crate::logins::LoginsError::Io,
            crate::logins::LoginsError::NoPlace,
        ] {
            assert_eq!(
                classify_error_reason(&err.to_string()),
                ErrorId::SavedLoginsError
            );
        }
        // The new code's words stay apart from the profile and downloads one.
        let text = format!(
            "{} {}",
            ErrorId::SavedLoginsError.title(),
            ErrorId::SavedLoginsError.explanation()
        )
        .to_ascii_lowercase();
        assert!(!text.contains("vault"));
        assert!(!text.contains("profile.anvil"));
    }

    #[test]
    fn classifies_renderer_network_and_http_errors_accurately() {
        assert_eq!(
            classify_error_reason("the renderer crashed (signal 25)"),
            ErrorId::RendererCrashedSignal
        );
        assert_eq!(
            classify_error_reason("the renderer stopped (exit code 3)"),
            ErrorId::RendererStopped
        );
        assert_eq!(
            classify_error_reason("the renderer stopped responding"),
            ErrorId::RendererTimedOut
        );
        assert_eq!(
            classify_error_reason("the renderer took too long over this page"),
            ErrorId::RendererPageTooSlow
        );
        assert_eq!(
            classify_error_reason("the renderer took too long over this page (20510 ms of CPU)"),
            ErrorId::RendererPageTooSlow
        );
        assert_eq!(
            classify_error_reason("the server returned 404"),
            ErrorId::HttpClientError4xx
        );
        assert_eq!(
            classify_error_reason("the server returned 503"),
            ErrorId::HttpServerError5xx
        );
        assert_eq!(
            classify_error_reason("could not read /tmp/missing.html: No such file"),
            ErrorId::LocalFileReadFailed
        );
    }

    #[test]
    fn easter_egg_constants_are_present_and_harmless() {
        assert!(SILENT_HILL_2_RESTLESS_DREAMS.contains("Silent Hill"));
        assert!(SILENT_HILL_2_RESTLESS_DREAMS.contains("There was a HOLE here"));
        assert!(ONE_PIECE_LAUGH_TALE_LOG_POSE.contains("Laugh Tale"));
        assert!(ONE_PIECE_LAUGH_TALE_LOG_POSE.contains("Grand Line"));
    }
}
