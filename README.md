<div align="center">
  <img src="./assets/flux-logo.png" width="96" height="96" alt="Flux Browser Logo" />
  <h1>Flux Browser</h1>
  <p><strong>A Proof of Concept (PoC) ground-up web browser and hybrid Rust, C++20, and JavaScript rendering engine with hardcoded OS sandboxing and GPU video acceleration.</strong></p>

  <p>
    <a href="https://github.com/buntatoes/flux-browser/releases/tag/v.15"><img src="https://img.shields.io/badge/release-v.15-ff6b24?style=flat-square" alt="Release v.15" /></a>
    <a href="https://github.com/buntatoes/flux-browser"><img src="https://img.shields.io/badge/status-Proof_of_Concept_(PoC)-d4a359?style=flat-square" alt="Status: Proof of Concept" /></a>
    <a href="https://github.com/buntatoes/flux-browser"><img src="https://img.shields.io/badge/platform-Windows_x64_%7C_Linux_x64-2ea44f?style=flat-square" alt="Platform" /></a>
    <a href="https://github.com/buntatoes/flux-browser"><img src="https://img.shields.io/badge/engine-Rust_%7C_C%2B%2B20_%7C_JavaScript-58a6ff?style=flat-square" alt="Engine" /></a>
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security Policy" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0_%2B_MPL--2.0_%2B_Proprietary-e8c37a?style=flat-square" alt="License" /></a>
  </p>

  <p>
    <a href="#download--quick-start">Download</a> ·
    <a href="#whats-new-in-v15">What's New in v.15</a> ·
    <a href="#core-features">Core Features</a> ·
    <a href="#architecture--workspace-crates">Architecture</a> ·
    <a href="#building-from-source">Build from Source</a> ·
    <a href="#keyboard--mouse-shortcuts">Shortcuts</a> ·
    <a href="./SECURITY.md">Security Policy</a> ·
    <a href="#licensing">Licensing</a>
  </p>
</div>

---

## Overview

**Flux Browser** (`flux-browser`) is a ground-up **Proof of Concept (PoC)** desktop web browser and custom rendering engine built from scratch without external browser engines or third-party crate dependencies:

- **Rust Core & Hardcoded Security Boundary**: Arena-allocated DOM tree, CSS cascade, layout engine, native `Schannel` / `OpenSSL` TLS transport, pre-socket tracker/malware shield, and mandatory OS process isolation.
- **Native C++20 Acceleration Kernels**: Freestanding C++20 rasterization (`flux_raster_accel.cpp`) and JavaScript JIT numeric/loop evaluation (`flux_jit_accel.cpp`) compiled directly into the workspace via zero-dependency `build.rs` pipelines.
- **Privileged React/TSX & JavaScript Front-End**: Built-in `flux://home`, `flux://settings`, `flux://downloads`, `flux://history`, `flux://security`, and `flux://inspect` pages rendered natively by the Flux engine.

---

## Download & Quick Start

Pre-built, Authenticode-signed Windows x64 binaries are available on the [**Releases (`v.15`)**](https://github.com/buntatoes/flux-browser/releases/tag/v.15) page:

| Package | Description | Format |
| :--- | :--- | :--- |
| [**`FluxBrowser-Setup-v.15-win64.exe`**](https://github.com/buntatoes/flux-browser/releases/download/v.15/FluxBrowser-Setup-v.15-win64.exe) | Unified Windows x64 desktop installer & uninstaller (`--uninstall`). Installs to `%LOCALAPPDATA%\Programs\Flux Browser` with Start Menu and Desktop shortcuts. | Windows x64 Installer |
| [**`flux.exe`**](https://github.com/buntatoes/flux-browser/releases/download/v.15/flux.exe) | Standalone portable Windows x64 browser binary (zero installation required). | Portable Executable |

---

## What's New in `v.15`

- **Hybrid C++20 / Rust / JavaScript Engine**:
  - **C++20 Framebuffer & SDF Rasterization Kernel (`crates/flux-raster/cpp/flux_raster_accel.cpp`)**: Accelerates Win32 bottom-up RGBA-to-BGRA framebuffer conversion, scanline alpha-blended rectangle fills, and anti-aliased SDF rounded rectangles.
  - **C++20 JavaScript JIT Kernel & Expanded Web JS (`crates/flux-script/cpp/flux_jit_accel.cpp`)**: Adds bytecode block compilation (`CompiledOp`), C++20 numeric/loop evaluation, inline DOM selector caching (`lookup_cached_dom`), function hoisting, `return` values, arrow functions (`() => {}`), object literals, template strings, `try`/`catch`, `console.*`, `Math.*`, `String`/`Array` methods, `document.querySelectorAll`, `el.innerHTML`, `el.remove()`, and `window.location.href`.
  - **Front-End `.js` Controllers (`frontend/scripts/home.js`, `frontend/scripts/settings.js`)**: Drives interactive search input focus on `flux://home` and instant category filtering on `flux://settings`.
- **Two-Column Browser Settings (`flux://settings`), Search Engine Logos & JIT Toggle**:
  - **Two-Column Settings UI**: Left vertical category sidebar (**All Settings**, **Search Engine**, **JavaScript & JIT**, **Privacy & Security**, **System & Hardware**) paired with horizontal setting cards.
  - **Integrated Search Engines with Brand Logos**: Select **Google Search**, **DuckDuckGo**, or **Microsoft Bing** in `flux://settings` with procedural SDF brand badges (`flux://engine-google.png`, `flux://engine-ddg.png`, `flux://engine-bing.png`), while keeping `flux://home` a clean, minimal start screen displaying the active search engine badge inside the search bar.
  - **JavaScript & JIT Compilation Toggles**: Independently enable/disable website JavaScript execution (`flux://settings?toggle=scripts`) and switch between the **Native C++20 JIT + Bytecode Tier** and **Rust Interpreter-Only Safe Mode** (`flux://settings?toggle=jit`).
- **Three-Tier Hybrid Licensing (`Apache-2.0` + `MPL-2.0` + `Proprietary`)**:
  - Releases the core rendering, layout, C++20 raster, and JS engine crates under **Apache License 2.0** while keeping the hardcoded security sandbox, network shield, and browser shell under **Proprietary** protection.

---

## Core Features

| Area | Capabilities |
| :--- | :--- |
| **HTML5 & CSS3 Layout** | Depth-bounded DOM arena (`MAX_DOM_DEPTH = 256`, `MAX_DOM_NODES = 50,000`), CSS specificity cascade, `@media` viewport & color-scheme queries, block/inline/Flexbox/HTML `<table>` layout, positioned layers (`relative`, `absolute`, `fixed`, `z-index`), SDF `border-radius`, `box-shadow`, and `linear-gradient(...)`. |
| **JavaScript & JIT (`flux-script`)** | 100,000-step instruction budget, 64-frame recursion cap, Same-Origin Policy (`SOP`) on `fetch()`, optional C++20 JIT + inline DOM cache tier, timers, event listeners, JSON parsing/serialization, and W3C `navigator.requestMediaKeySystemAccess(...)`. |
| **GPU Video & W3C EME / CENC** | Hardware-accelerated `NV12` / BT.709 video decode & encode (`D3d11Dxva2` on Windows, `LinuxVaApi` on Linux), ISO/IEC 23001-7 `CENC` encrypted `.mp4` demuxing (`sinf`/`schm`/`tenc`/`pssh`), W3C ClearKey CDM session, and interactive on-video playback & timeline scrubber HUD. |
| **Security & Privacy Shield** | Mandatory OS process sandbox (Windows Job Object / Linux `rlimit`), per-tab fault isolation, Pre-Socket Tracker/Ad/Malware Shield (`NetworkShield`), `Strict-Transport-Security` (`HSTS`), `Content-Security-Policy` (`CSP`), `Cross-Origin-Resource-Policy` (`CORP`), Subresource Integrity (`SRI` SHA-256), and RFC 6265bis `CookieJar`. |
| **Encrypted Profile & Downloads** | Windows DPAPI (`CryptProtectData` / `CryptUnprotectData`) + `zlib` compressed `FLXP\x01` profile container (`profile.flx`) with 64-bit MAC tag; FIPS 180-4 SHA-256 Download Manager (`flux://downloads`) with executable quarantine and Windows Mark-of-the-Web (`ZoneId=3`). |
| **Developer Inspector (`F12`)** | Docked bottom Developer Tools (`Elements`, `Styles & Layout`, `Network & Shield`, `Console & Sandbox`) and full-page `flux://inspect` diagnostic dashboard. |

---

## Architecture & Workspace Crates

```text
URL / File / flux:// Input
  ├── flux-net + flux-compress   HTTP/1.1 GET & POST, HSTS, SRI SHA-256, Schannel/OpenSSL TLS, Pre-Socket Shield
  ├── flux-dom + flux-image      Bounded DOM Arena, HTML/Table/Form Parser, 16 MP Image Decoder, NV12/CENC Video
  ├── flux-script (Rust + C++20) Budgeted JS Interpreter + C++20 JIT Kernel, Inline DOM Cache, SOP Fetch, Events
  ├── flux-style                 CSS Cascade, @media Queries, Positioning, Z-Index, Box Shadows, Linear Gradients
  ├── flux-layout + flux-ttf     Block, Inline, Flexbox, Table & Out-of-Flow Layout, TrueType Font Metrics
  ├── flux-paint                 Z-Ordered DisplayList (Clips, Fixed Layers, Shadows, Gradients, Rounded Rects, Text)
  ├── flux-raster (Rust + C++20) C++20 Scanline/SDF/BGRA Kernel, Scissor Stack, Win32 GDI & Linux TTF Glyphs
  └── flux-browser + flux-sandbox Multi-Tab UI, React/TSX + JS Pages, DPAPI Profile, OS Process Sandbox (Win32 / X11)
```

### Workspace Crates (`Flux browser`)

| Crate | Version | License | Description |
| :--- | :---: | :---: | :--- |
| `crates/flux-dom` | `0.15.0` | `Apache-2.0` | Arena-backed HTML document tree, selector engine (`getElementById`, `querySelector`, `querySelectorAll`), `set_inner_html`, and form serialization. |
| `crates/flux-style` | `0.15.0` | `Apache-2.0` | CSS parser, specificity cascade, `@media` queries, Flexbox properties, `box-shadow`, and `linear-gradient(...)`. |
| `crates/flux-layout` | `0.15.0` | `Apache-2.0` | Block, inline, Flexbox, HTML `<table>`, and positioned layout engine with TrueType text measurement and hit-testing. |
| `crates/flux-paint` | `0.15.0` | `Apache-2.0` | Display list builder with `z-index` stacking and `50%`–`250%` viewport zoom scaling. |
| `crates/flux-raster` | `0.15.0` | `Apache-2.0` | Hybrid Rust + C++20 (`cpp/flux_raster_accel.cpp`) RGBA/BGRA framebuffer and SDF rasterizer. |
| `crates/flux-script` | `0.15.0` | `Apache-2.0` | Hybrid Rust + C++20 (`cpp/flux_jit_accel.cpp`) JavaScript engine, bytecode compiler, and optional JIT tier. |
| `crates/flux-net` | `0.15.0` | `Proprietary` | Hardened URL/HTTP/TLS stack, Pre-Socket Tracker/Malware Shield, HSTS, CSP, CORP, SRI SHA-256, and `CookieJar`. |
| `crates/flux-sandbox` | `0.15.0` | `Proprietary` | Mandatory OS process sandbox (Windows Job Object / Linux `rlimit`) and per-tab fault isolation. |
| `crates/flux-browser` | `0.15.0` | `Proprietary` | Multi-tab browser shell, React/TSX & JS `flux://` pages, DPAPI profile encryption, SHA-256 download quarantine, and installer (`flux-setup`). |

### Standalone Companion Libraries (`MPL-2.0`)

| Repository | Version | License | Description |
| :--- | :---: | :---: | :--- |
| [`flux-compress`](https://github.com/buntatoes/flux-compress) | `v.02` | `MPL-2.0` | DEFLATE (RFC 1951), `zlib` (Adler-32), and `gzip` (CRC-32) compression & bounded decompression. |
| [`flux-image`](https://github.com/buntatoes/flux-image) | `v.04` | `MPL-2.0` | PNG, JPEG, GIF89a, WebP, Windows `.ico`, BMP, and Netpbm (PPM/PGM) decoder, encoder, and resampler. |
| [`flux-json`](https://github.com/buntatoes/flux-json) | `v.02` | `MPL-2.0` | Zero-dependency RFC 8259 bounded JSON parser, value tree, and compact/pretty serializer. |
| [`flux-ttf`](https://github.com/buntatoes/flux-ttf) | `v.03` | `MPL-2.0` | TrueType/OpenType (`.ttf`/`.otf`) SFNT table parser, outline decoder, and scanline glyph rasterizer. |
| [`flux-gpu`](https://github.com/buntatoes/flux-gpu) | `v.04` | `MPL-2.0` | 2D SDF quad compositor, `NV12` BT.709 GPU surfaces, RGBA texture atlas, and HLSL/GLSL shaders. |
| [`flux-video`](https://github.com/buntatoes/flux-video) | `v.03` | `MPL-2.0` | GPU-accelerated `NV12`/BT.709 video codec, ISO/IEC 23001-7 `CENC` encryption, W3C `EME`, and `.mp4` muxer/demuxer. |

---

## Building from Source

### Prerequisites
- **Rust**: `1.85+` (`2024` edition)
- **C++20 Compiler**: MSVC `cl.exe` (Visual Studio 2022/2026 Build Tools on Windows) or `g++` / `clang++` (on Linux)

```powershell
# Run the full test suite across all 9 workspace crates
cargo test

# Build optimized release binaries (flux.exe and flux-setup.exe)
cargo build --release --bins

# Launch Flux Browser directly on flux://home
cargo run --release -p flux-browser --bin flux

# Rasterize any URL or built-in flux:// page to a binary PPM image
cargo run --release -p flux-browser --bin flux -- raster --width 1200 --height 820 flux://home home.ppm
```

---

## Keyboard & Mouse Shortcuts

| Shortcut / Action | Function |
| :--- | :--- |
| `Ctrl+T` / `Ctrl+W` | Open a new tab on `flux://home` / close active tab |
| `Ctrl+L` / `F6` | Focus the address & search bar (`Enter` to navigate, `Tab` to autocomplete) |
| `Ctrl+R` / `F5` | Reload the active page |
| `Alt+Left` / `Alt+Right` | Navigate back / forward in tab history |
| `Alt+Home` | Open the `flux://home` start screen |
| `Ctrl+F` | Open the floating Find-in-Page search bar |
| `Ctrl+J` / `Ctrl+H` | Open `flux://downloads` (Download Manager) / `flux://history` (Browsing History) |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` | Zoom in, zoom out (`50%`–`250%`), or reset page zoom (`100%`) |
| `F12` | Toggle the bottom Developer Inspector dock (`Elements`, `Styles`, `Network`, `Console`) |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy, cut, or paste via the system clipboard |

---

## Licensing

`flux-browser` is distributed under a **Three-Tier Hybrid License** (see [`LICENSE`](./LICENSE), [`LICENSE-APACHE`](./LICENSE-APACHE), [`LICENSE-MPL`](./LICENSE-MPL), [`LICENSE-PROPRIETARY`](./LICENSE-PROPRIETARY), and [`NOTICE`](./NOTICE) for full legal text):

1. **Tier 1 — Core Rendering, Layout, C++20 Raster & JavaScript Engine ([`Apache-2.0`](./LICENSE-APACHE) + [`NOTICE`](./NOTICE))**: `crates/flux-dom`, `crates/flux-style`, `crates/flux-layout`, `crates/flux-paint`, `crates/flux-raster`, and `crates/flux-script` are open-source under the **Apache License, Version 2.0**.
2. **Tier 2 — Hardcoded Security, OS Sandbox, Shield & Browser Application ([`Proprietary — All Rights Reserved`](./LICENSE-PROPRIETARY))**: `crates/flux-sandbox`, `crates/flux-net`, `crates/flux-browser`, `frontend/`, and `assets/flux-logo.png` (including DPAPI profile sealing, SHA-256 download quarantine, and official `Flux Browser` branding) are source-available for security auditing under `LicenseRef-Proprietary` (**All Rights Reserved**).
3. **Tier 3 — Standalone Companion Libraries ([`MPL-2.0`](./LICENSE-MPL))**: `flux-compress`, `flux-image`, `flux-json`, `flux-ttf`, `flux-gpu`, and `flux-video` are licensed under the **Mozilla Public License 2.0 (`MPL-2.0`)**.

---

## Author

**Buntos** ([@buntatoes](https://github.com/buntatoes))
