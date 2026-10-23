# Security Policy

## Proof of Concept (PoC) Notice

**Flux Browser** (`flux-browser`) is an experimental **Proof of Concept (PoC)** desktop web browser and custom rendering engine architecture authored from scratch by **Buntos**. While it implements defense-in-depth security controls—including mandatory OS process sandboxing, bounded parsers, pre-socket tracker/malware shielding, DPAPI profile encryption, and SHA-256 download quarantine—it is distributed as a research and engineering Proof of Concept on an `"AS IS"` basis.

---

## Supported Versions

Only the latest standalone Proof of Concept release is actively maintained and receives security updates:

| Version | Supported |
| :--- | :---: |
| `v.15` (Latest) | Yes |
| `< v.15` | No |

---

## Reporting a Vulnerability

Please **do not** report security vulnerabilities through public GitHub issues, discussions, or pull requests.

### 1. GitHub Private Vulnerability Reporting (Recommended)
To report a vulnerability privately without exposing sensitive details publicly, submit a report through GitHub's **Private Vulnerability Reporting** portal:

- **Report Privately**: [`https://github.com/buntatoes/flux-browser/security/advisories/new`](https://github.com/buntatoes/flux-browser/security/advisories/new)

*(Note: The author commit attribution address `324143321+buntatoes@users.noreply.github.com` is a GitHub privacy `noreply` address for Git commit sign-off and does not accept inbound external email; GitHub Private Vulnerability Reporting routes reports directly to the maintainer's private inbox.)*

### 2. What to Include in a Report
- **Affected Crate or Subsystem**: (`flux-sandbox`, `flux-net`, `flux-script`, `flux-raster`, `flux-dom`, `flux-style`, `flux-layout`, `flux-paint`, `flux-browser`, or a `flux-*` companion library).
- **Reproduction Steps / Proof of Concept**: Minimal HTML, CSS, JavaScript, network payload, or binary input demonstrating the issue.
- **Impact Assessment**: Whether the issue involves memory safety, resource exhaustion / allocation bounds bypass, Same-Origin Policy (`SOP`) / Content-Security-Policy (`CSP`) bypass, or OS sandbox escape.

---

## Architecture Security Boundaries (`v.15`)

| Boundary | Crate / Component | Enforcement Mechanism |
| :--- | :--- | :--- |
| **OS Process Sandbox** | `crates/flux-sandbox` | Fail-closed Windows Job Object (`JOB_OBJECT_LIMIT_ACTIVE_PROCESS = 1`, `512 MiB` memory cap, UI clipboard/display isolation) and Linux `RLIMIT_AS` / `RLIMIT_NPROC`. |
| **Per-Tab Worker Isolation** | `crates/flux-sandbox` | Out-of-process `--tab-worker` execution over length-prefixed `FLUXIPC1` pipes (`MAX_IPC_FRAME_BYTES = 8 MiB`). |
| **Network & TLS Policy** | `crates/flux-net` | Pre-Socket Tracker/Malware Shield (`NetworkShield`), `Strict-Transport-Security` (`HSTS`), `Content-Security-Policy` (`CSP`), `Cross-Origin-Resource-Policy` (`CORP`), Subresource Integrity (`SRI` FIPS 180-4 SHA-256), and RFC 6265bis `CookieJar`. |
| **DOM & Parser Ceilings** | `crates/flux-dom` | Bounded DOM arena (`MAX_DOM_DEPTH = 256`, `MAX_DOM_NODES = 50,000`, `MAX_ATTRS_PER_ELEMENT = 128`). |
| **JavaScript Execution Budget** | `crates/flux-script` | Deterministic `100,000`-step instruction budget, `64`-frame call-stack cap, Same-Origin Policy (`SOP`) on `fetch()`, and runtime toggles in `flux://settings` to disable JavaScript or switch between the C++20 JIT tier and Rust Interpreter-Only Safe Mode. |
| **Profile & Download Protection** | `crates/flux-browser` | Windows DPAPI (`CryptProtectData` / `CryptUnprotectData`) + 64-bit MAC tag on `profile.flx`; SHA-256 verification, `.quarantine` holding for executables, and Windows Mark-of-the-Web (`Zone.Identifier` `ZoneId=3`). |

---

## Maintainer & Attribution

- **Author**: **Buntos** ([@buntatoes](https://github.com/buntatoes) · `Buntos <324143321+buntatoes@users.noreply.github.com>`)
