<p align="center">
  <img src="./assets/flux-logo.png" width="108" height="108" alt="Flux Browser Logo" />
</p>

# flux-browser

[![Release](https://img.shields.io/badge/release-v.08-e8c37a.svg)](https://github.com/buntatoes/flux-browser/releases/tag/v.08)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-2ea44f.svg)](https://github.com/buntatoes/flux-browser)
[![GPU](https://img.shields.io/badge/GPU-HLSL%20%7C%20GLSL%20%7C%20NV12-58a6ff.svg)](https://github.com/buntatoes/flux-browser)
[![Frontend](https://img.shields.io/badge/frontend-React_TSX-61dafb.svg)](https://github.com/buntatoes/flux-browser)
[![License: Proprietary](https://img.shields.io/badge/license-Proprietary-8b3a3a.svg)](./LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://github.com/buntatoes/flux-browser)

`flux-browser` is a ground-up web browser and custom rendering engine written in Rust with a privileged React/TSX front-end (`frontend/App.tsx`, `frontend/components/StartDashboard.tsx`, `frontend/components/SecurityCenter.tsx`, `frontend/components/SettingsPage.tsx`, `frontend/components/HistoryPage.tsx`), GPU-accelerated `NV12`/BT.709 video encode/decode (`flux-video` + `flux-gpu`), native cross-platform window shells (`Win32` on Windows and `X11` on Linux), native cross-platform TLS (`Schannel` on Windows and `OpenSSL` on Linux), hardened security sandbox, and cross-platform Windows/Linux installer (`flux-setup`).

It parses HTML into a depth- and node-bounded arena document tree, loads linked and embedded CSS stylesheets, decodes bounded `gzip`/`deflate` HTTP responses, `<img src="...">` raster images (PNG, JPEG, GIF89a, BMP, PPM/PGM), and `<video src="...">` MP4/NV12 streams, computes cascaded styles (tag, `.class`, `#id`, compound, and descendant selectors, `@media` viewport & color-scheme queries, inline `style` attributes, positioned layout, `z-index`, `overflow: hidden`, `box-shadow`, and `linear-gradient(...)`), lays out block, inline, Flexbox, HTML `<table>` grids, and interactive form controls (`<input>`, `<textarea>`, `<select>`, checkboxes, buttons) with TrueType font metrics, executes `<script>` blocks under deterministic instruction and recursion budgets with Same-Origin Policy (`SOP`) enforcement, emits a display list, rasterizes SDF rounded corners, box shadows, linear gradients, images, and anti-aliased TrueType text into an RGBA framebuffer, and presents interactive multi-tab pages with a security badge, status bar, bookmarks bar, vertical scrollbar, and asynchronous `GET`/`POST` networking.

---

## Release Highlights (`v.08`)

- **Hardcoded Mandatory OS Process Sandbox (`crates/flux-sandbox` — Fail-Closed at Startup) & `FLUXIPC1` Tab Isolation**:
  - **Fail-Closed Startup Enforcement (`enforce_mandatory_sandbox_or_abort`)**: All sandbox limits are compile-time constants (`256 MiB` per-tab RAM cap, `ActiveProcessLimit = 1`, `JobObjectBasicUIRestrictions = 0xFF` clipboard/handle/desktop isolation, `KillOnJobClose`, `8 MiB` IPC ceiling) with zero configuration flags or environment overrides. Flux refuses to launch if the operating system fails to initialize the sandbox primitives.
  - **Cross-Platform Kernel Enforcement**: Uses native **Win32 Job Objects** (`CreateJobObjectW`, `SetInformationJobObject`, `AssignProcessToJobObject`) on Windows and **`prctl(PR_SET_NO_NEW_PRIVS)` + `setrlimit(RLIMIT_AS / RLIMIT_CORE)`** on Linux, paired with `FLUXIPC1` length-prefixed IPC frames (`--tab-worker`) and per-tab crash containment (`open_isolated_workload`).
- **Content Security Policy (`CSP`), Cross-Origin Resource Policy (`CORP`), & Hardened RFC 6265bis Cookie Jar (`flux-net` & `flux-dom`)**:
  - **`Content-Security-Policy` (`CSP`) Enforcement**: Parses both HTTP `Content-Security-Policy` headers and `<meta http-equiv="Content-Security-Policy">` tags (`default-src`, `script-src`, `style-src`, `img-src`, `media-src`, `connect-src`), blocking unauthorized inline `<script>` / `<style>` blocks and untrusted subresources.
  - **`Cross-Origin-Resource-Policy` (`CORP`)**: Evaluates `same-origin`, `same-site`, and `cross-origin` subresource policies before loading external stylesheets, images, or videos.
  - **RFC 6265bis `CookieJar`**: Enforces `HttpOnly` (strictly omitted from `script_cookie_string`), `Secure`, `SameSite=Strict|Lax|None`, and `__Host-` / `__Secure-` prefix invariants, with bounded JSON import/export (`flux-json v.02`).
- **Native Linux X11 Window Shell (`libX11`) & Linux OpenSSL (`libssl` / `libcrypto`) TLS Backend (`flux-browser` & `flux-net`)**:
  - **Cross-Platform Window Shell (`crates/flux-browser/src/shell.rs`)**: Pairs the Windows `Win32` (`user32` / `gdi32` / `dwmapi`) window shell with a native Linux `X11` (`libX11`) window shell supporting `XOpenDisplay`, `XCreateSimpleWindow`, `WM_DELETE_WINDOW` protocol handling, 32-bit top-down BGRA `ZPixmap` presentation (`XCreateImage` / `XPutImage`), mouse motion/click/wheel events, keyboard navigation & text entry (`XLookupString`), and asynchronous background page loading.
  - **Cross-Platform Native TLS (`crates/flux-net/src/tls.rs`)**: Pairs Windows `Schannel` (`secur32.dll`) with a native Linux `OpenSSL` (`libssl` / `libcrypto`) TLS 1.2/1.3 client supporting `TLS_client_method`, system CA bundle verification (`SSL_CTX_set_default_verify_paths`), `SSL_VERIFY_PEER`, SNI hostname indication (`SSL_CTRL_SET_TLSEXT_HOSTNAME`), and peer X.509 hostname validation (`X509_VERIFY_PARAM_set1_host`).
  - **Linux System TrueType Font Discovery (`crates/flux-raster`)**: Automatically loads system TrueType fonts on Linux (`DejaVuSans.ttf` / `LiberationSans-Regular.ttf`) via `flux-ttf` alongside Windows `GDI` TrueType rendering.
- **CSS Media Queries (`@media`), HTML Tables (`<table>`), & `<textarea>` / `<select>` Controls (`flux-dom`, `flux-style`, `flux-layout`, `flux-browser`)**:
  - **Responsive `@media` Queries (`flux-style`)**: Parses and evaluates `@media (max-width: ...px)`, `@media (min-width: ...px)`, and `@media (prefers-color-scheme: dark|light)` blocks (`compute_styles_with_viewport`), automatically restaging responsive layouts on window resize.
  - **HTML `<table>` Multi-Column Row Layout (`flux-dom` & `flux-layout`)**: Implicit HTML tag auto-closing for `<tr>`, `<td>`, `<th>`, `<thead>`, `<tbody>`, `<tfoot>`, and `<option>`, paired with automatic equal-width column distribution and uniform row height stretching (`layout_table_row`).
  - **Interactive `<textarea>` & `<select>` Controls (`flux-dom`, `flux-layout`, `flux-browser`)**: Supports multi-line `<textarea rows="..." cols="...">` editing, `<select name="..."><option value="..." selected>...</option></select>` click option cycling (`ClickOutcome::SelectChanged`), dropdown badge rendering (`[Label  v]`), and `GET`/`POST` form serialization.
- **GPU-Accelerated `NV12` / BT.709 Video Encode & Decode (`flux-video v.01` + `flux-gpu v.04`) & HTML `<video>` Support**:
  - Dedicated standalone **`flux-video v.01`** library and **`flux-gpu v.04`** `NV12` BT.709 GPU surface shaders (`ConvertNv12Bt709ToRgba` and `ConvertRgbaToYuvBt709` in both HLSL Shader Model 5.0 and GLSL 330 Core).
  - Supports **Windows Direct3D 11 Video / Media Foundation DXVA2** (`D3d11Dxva2` — NVIDIA NVDEC/NVENC, AMD AMF/VCN, Intel QuickSync) and **Linux VA-API / DRM** (`LinuxVaApi`) on `NV12` surfaces, alongside temporal `I-frame` / `P-frame` encoding, ISOBMFF (`.mp4`) container muxing/demuxing, and 4K (`4096x2160`) security caps.
  - Native HTML `<video src="...">` and `<video><source src="..."></video>` discovery, layout, and frame decoding into the browser display list.
- **Interactive React/TSX Settings (`flux://settings`) & Browsing History (`flux://history`) Dashboards**:
  - **`flux://settings` (`frontend/components/SettingsPage.tsx`)**: Live control center for toggling **GPU Hardware Video Decode & Encode**, **Strict HTTPS-Only Mode**, and **Script Execution**, complete with live GPU backend telemetry, a cross-platform Windows/Linux hardware acceleration matrix `<table>`, and an embedded `<video src="flux://demo.mp4">` preview stream.
  - **`flux://history` (`frontend/components/HistoryPage.tsx`)**: Local zero-telemetry session history log with one-click history clearing (`flux://history?action=clear`).
- **React/TSX Browser Front-End (`frontend/App.tsx`), Security Padlock Badge, Status Bar, & Built-In `flux://` Dashboards**:
  - Full React/TSX front-end architecture (`frontend/App.tsx`, `frontend/components/StartDashboard.tsx`, `frontend/components/SecurityCenter.tsx`, `frontend/components/SettingsPage.tsx`, `frontend/components/HistoryPage.tsx`, and `frontend/styles/browser.css`) compiled by `frontend.rs` directly into Flux DOM trees.
  - Launching `flux` with no arguments opens the window directly on the built-in **`flux://home`** start dashboard (`StartDashboard.tsx`), rendered by Flux's own engine with linear gradients, soft SDF box shadows, quick-access cards, and an interactive `<form action="flux://navigate">` launcher.
  - Built-in **`flux://security`** Security Center page (`SecurityCenter.tsx`) detailing all active transport, decompression, image, DOM, and script sandbox protections.
  - Upgraded browser shell (`App.tsx` + `ui.rs`) with SDF rounded tabs and buttons, a **Home (`H`)** button, a color-coded **Security Badge (`TLS`, `SYS`, `HTTP`, `FILE`)** next to the address bar that opens `flux://security`, a bottom **Status Bar** showing live hovered link URLs, security shield state, and scroll percentage, plus Windows 11 Dark Mode titlebar integration (`DwmSetWindowAttribute`).
  - Full keyboard shortcuts: `Ctrl+T` (new tab on `flux://home`), `Ctrl+W` (close tab), `Ctrl+L` / `F6` (focus address bar), `Ctrl+R` / `F5` (reload), `Alt+Left` / `Alt+Right` (back/forward), `Alt+Home` (`flux://home`), and `Home` / `End` (scroll top/bottom).
- **Massive Security Hardening Across All Layers**:
  - **Transport & Origin Sandbox (`flux-net`)**: Enforces 8 MiB raw response / 16 MiB decompressed body / 64 KiB header / 128 header-count limits, blocks `https://` -> `http://` protocol downgrade redirects, rejects userinfo (`@`) credential-phishing URLs and CRLF header/URL injection, blocks remote origins from referencing local files or `flux://` pages, exports `is_same_origin`, and sends privacy-hardened headers (`Referrer-Policy: no-referrer`, `DNT: 1`, `X-Content-Type-Options: nosniff`).
  - **Decompression-Bomb Guard (`flux-compress v.02`)**: Bounded DEFLATE, zlib, and gzip stream expansion (`decompress_deflate_bounded`, `decompress_zlib_bounded`, `decompress_gzip_bounded` with a 32 MiB ceiling) halting zip-bomb attacks immediately.
  - **Image Allocation-Bomb Guard (`flux-image v.03`)**: Enforces `MAX_IMAGE_DIMENSION = 8192` and `MAX_IMAGE_PIXELS = 16,777,216` (64 MiB RGBA cap) across PNG, JPEG, GIF89a, BMP, and PPM/PGM before buffer allocation.
  - **DOM & Script Execution Sandbox (`flux-dom` & `flux-script`)**: Enforces `MAX_DOM_DEPTH = 256`, `MAX_DOM_NODES = 50,000`, `MAX_ATTRS_PER_ELEMENT = 128`, `MAX_EXECUTION_STEPS = 100,000`, `MAX_CALL_DEPTH = 64`, `MAX_ARRAY_LEN = 10,000`, and Same-Origin Policy (`SOP`) on script `fetch()` calls.
- **Cross-Platform Native Installer & Uninstaller (`flux-setup` / `FluxBrowser-Uninstall.exe`), Brand Logo (`flux://logo.png` / `flux-logo.ico`), & Shader Parity (`flux-gpu v.04`)**:
  - Dedicated `flux-setup` installer and `FluxBrowser-Uninstall.exe` interactive uninstaller supporting both **Windows** (installs `flux.exe`, `FluxBrowser-Uninstall.exe`, `flux-logo.ico`, and `flux-logo.png` to `%LOCALAPPDATA%\Programs\Flux Browser`, creates icon-branded Start Menu and Desktop `.lnk` shortcuts including `Uninstall Flux Browser.lnk`, and registers the per-user Windows `Uninstall\FluxBrowser` registry entry with `DisplayIcon` and `UninstallString`) and **Linux** (installs to `~/.local/share/flux-browser` and `~/.local/bin/flux`, and writes an XDG `~/.local/share/applications/flux-browser.desktop` launcher entry).
  - Official **Flux Browser Logo (`crates/flux-browser/src/logo.rs` + `assets/flux-logo.png`)** rendered in the top-left of the browser window toolbar, top-left of all internal `flux://` pages (`flux://settings`, `flux://home`, `flux://security`, `flux://history`) via `flux://logo.png`, the native Win32 window titlebar/taskbar icon (`CreateIcon` + `WM_SETICON`), and installed shortcut `.ico` files.
  - **`flux-gpu v.04`** embeds both **HLSL Shader Model 5.0** (`shaders/compositor.hlsl` for Windows Direct3D) and **GLSL 330 Core** (`shaders/compositor.glsl` for Linux OpenGL/Vulkan).

---

## Rendering & Execution Pipeline

```text
URL / File / flux:// Input
  -> flux-net + flux-compress (HTTP/1.1 GET & POST, downgrade guard, bounded gzip/deflate, Schannel/OpenSSL TLS)
  -> flux-dom + flux-image + flux-video (Bounded DOM [depth<=256], tables/forms, 16 MP images, NV12/MP4 video)
  -> flux-script + flux-json  (100K-step budgeted interpreter, SOP-enforced fetch, timers, JSON, click events)
  -> flux-style               (CSS cascade, @media queries, position/z-index/overflow, shadows, gradients)
  -> flux-layout + flux-ttf   (Block, Inline, Flexbox, Table, & Out-of-Flow layout, TrueType metrics, hit-test)
  -> flux-paint               (DisplayList: clips, fixed layers, shadows, gradients, rounded rects, text)
  -> flux-raster + flux-gpu   (RGBA framebuffer, scissor stack, SDF shadows/gradients/corners, TTF glyphs)
  -> flux-browser             (Multi-tab React/TSX GUI, flux:// dashboards, installer, Win32 & X11 shells)
```

---

## Workspace & Standalone Dependency Architecture

| Crate / Dependency | Version | Responsibility |
| :--- | :---: | :--- |
| [`flux-dom`](./crates/flux-dom) | `v.08` | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations, `MAX_DOM_DEPTH = 256` and `MAX_DOM_NODES = 50,000` security caps, DOM query/mutation helpers, `classList`, `<table>` auto-closing, `<input>`/`<textarea>`/`<select>`/checkbox state, and `application/x-www-form-urlencoded` serialization), HTML parser, `<meta http-equiv="Content-Security-Policy">`, `<link rel="stylesheet">`, `<script>`, `<img src>`, and `<video src>` discovery. |
| [`flux-style`](./crates/flux-style) | `v.08` | CSS stylesheet parser (tag, `.class`, `#id`, compound, and descendant selectors), `@media` viewport & color-scheme queries (`compute_styles_with_viewport`), inline `style="..."` attributes, user-agent defaults, property inheritance, `(ids, classes, tags)` specificity cascade, `position`/`top`/`right`/`bottom`/`left`/`z-index`/`overflow`, `border-radius`, `box-shadow`, `linear-gradient(...)`, and Flexbox properties. |
| [`flux-layout`](./crates/flux-layout) | `v.08` | Block, inline, Flexbox (`row` / `column`, `gap`, `justify-content`, `align-items`), HTML `<table>` (`layout_table_row`), and out-of-flow (`relative`, `absolute`, `fixed`) layout, interactive `<input>`/`<textarea>`/`<select>`/checkbox/button layout, intrinsic `<img>` and `<video>` sizing, TrueType proportional text measurement (`flux-ttf`), and `z-index`/clip-aware hit testing. |
| [`flux-paint`](./crates/flux-paint) | `v.08` | Converts the layout box tree into a `DisplayList` of `PushClip`/`PopClip`, `PushFixed`/`PopFixed`, `BoxShadow`, `LinearGradientRect`, `RoundedRect`, `Rect`, `Image`, underline, and `Text` commands ordered by `z-index`. |
| [`flux-raster`](./crates/flux-raster) | `v.08` | Software RGBA framebuffer rasterizer with scissor clip stack, fixed-layer scroll pinning, `flux-gpu` SDF rounded-corner, box-shadow, and linear-gradient rendering, `flux-image` bilinear image blitting, `flux-ttf` outline & Win32 GDI / Linux TrueType glyph rasterization, Win32 bottom-up & X11 top-down BGRA output, and binary PPM export. |
| [`flux-script`](./crates/flux-script) | `v.08` | Proprietary scripting interpreter with deterministic step/recursion budgets (`MAX_EXECUTION_STEPS = 100,000`, `MAX_CALL_DEPTH = 64`), Same-Origin Policy (`SOP`) on `fetch()`, and DOM bindings (`let`/`const`, `if`/`else`, `while`, `for`, arrays, `getElementById`, `querySelector`, `createElement`, `textContent`, `value`, `checked`, `classList`, `style.*`, `setTimeout`, `JSON.parse`/`stringify`, and `click` listeners). |
| [`flux-net`](./crates/flux-net) | `v.08` | Hardened URL parser (rejecting `@` userinfo phishing and CRLF injection), relative HTTP/HTTPS/file/`flux://` URL resolver, Same-Origin Policy (`is_same_origin`), `Content-Security-Policy` (`CSP`), `Cross-Origin-Resource-Policy` (`CORP`), RFC 6265bis `CookieJar` (`HttpOnly`, `Secure`, `SameSite`, `__Host-`), HTTP/1.1 `GET` and `POST` client with HTTPS downgrade protection, bounded `gzip`/`deflate` decompression (`flux-compress`), and native Windows Schannel + Linux OpenSSL TLS. |
| [`flux-sandbox`](./crates/flux-sandbox) | `v.08` | Hardcoded, fail-closed OS process sandbox (`Win32 JobObject` with 256 MiB RAM cap, `ActiveProcessLimit = 1`, `UIRestrictions = 0xFF`, `KillOnJobClose`, and Linux `prctl(PR_SET_NO_NEW_PRIVS)` + `rlimit`), `FLUXIPC1` framed tab worker protocol, and per-tab fault containment. |
| [`flux-browser`](./crates/flux-browser) | `v.08` | Multi-tab session, history, and bookmarks controller gated by `MandatorySandboxGuard`, built-in React/TSX `flux://home`, `flux://security`, `flux://settings`, and `flux://history` dashboards with top-left `flux://logo.png` branding, SDF rounded toolbar/security badge/status bar UI, cross-platform Windows/Linux installer & uninstaller (`flux-setup` / `FluxBrowser-Uninstall.exe`), asynchronous networking worker, native Win32 & Linux X11 window shells, and `flux` CLI. |
| [`flux-compress`](https://github.com/buntatoes/flux-compress) | `v.02` | Standalone DEFLATE (RFC 1951), `zlib` (Adler-32), and `gzip` (CRC-32) compression and bounded decompression library (`MPL-2.0`). |
| [`flux-image`](https://github.com/buntatoes/flux-image) | `v.03` | Standalone PNG, Baseline JPEG, GIF87a/GIF89a, BMP, and Netpbm (PPM/PGM) image decoder, encoder, bilinear resampler, and 16 MP allocation-bomb guard (`MPL-2.0`). |
| [`flux-json`](https://github.com/buntatoes/flux-json) | `v.02` | Standalone zero-dependency RFC 8259 bounded JSON parser (`MAX_JSON_DEPTH = 64`, `MAX_JSON_BYTES = 16 MiB`), value tree, and compact/pretty serializer (`MPL-2.0`). |
| [`flux-ttf`](https://github.com/buntatoes/flux-ttf) | `v.02` | Standalone TrueType/OpenType SFNT font table parser (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, `kern`, `loca`, `glyf`), outline decoder, and anti-aliased scanline glyph rasterizer (`MPL-2.0`). |
| [`flux-gpu`](https://github.com/buntatoes/flux-gpu) | `v.04` | Standalone 2D signed-distance-field (SDF) quad compositor, `NV12` BT.709 GPU video surfaces (`Nv12Surface`), shelf-packing RGBA texture atlas, and cross-platform HLSL + GLSL shader pipeline (`MPL-2.0`). |
| [`flux-video`](https://github.com/buntatoes/flux-video) | `v.01` | Standalone GPU-accelerated `NV12`/BT.709 video encoder and decoder (`GpuVideoSession`, `D3d11Dxva2`, `LinuxVaApi`), temporal `I-frame`/`P-frame` codec, and ISOBMFF (`.mp4`) container muxer/demuxer (`MPL-2.0`). |

---

## Building, Testing, and Installing

```powershell
# Run full test suite across all workspace crates
cargo test

# Launch Flux Browser GUI directly on flux://home
cargo run -p flux-browser --bin flux

# Run the standalone installer (installs flux.exe, Start Menu & Desktop shortcuts, and uninstaller entry)
cargo run -p flux-browser --bin flux-setup

# Uninstall Flux Browser
cargo run -p flux-browser --bin flux-setup -- --uninstall
```

---

## CLI Usage

```powershell
# Open the interactive browser window (defaults to flux://home)
flux
flux open https://example.com
flux open flux://security

# Install or uninstall per-user desktop package
flux install
flux uninstall

# Print the laid-out display list to stdout
flux render examples/hello.html

# Rasterize a page directly to a binary PPM image
flux raster --width 960 --height 720 flux://home home.ppm

# Print release version
flux version
```

---

## Interactive Window & Keyboard Controls

| Input | Action |
| :--- | :--- |
| Click tab / `x` / `+` (`Ctrl+T` / `Ctrl+W`) | Switch active tab, close tab, or open a new tab on `flux://home` |
| Click `<` / `>` (`Alt+Left` / `Alt+Right`) | Navigate back or forward in tab history |
| Click `R` (`Ctrl+R` / `F5`) | Reload the active page |
| Click `H` (`Alt+Home`) | Navigate to the built-in `flux://home` dashboard |
| Click `TLS` / `SYS` / `HTTP` / `FILE` badge | Open the built-in `flux://security` Security Center |
| Click `*` / bookmark pill | Bookmark the active page or open a saved bookmark |
| Click address field (`Ctrl+L` / `F6`) | Focus the address field (`Enter` to navigate, `Esc` to cancel) |
| Click page `<input>` / checkbox / button | Focus text input (`Enter` to submit `<form>`, `Esc` to blur), toggle checkbox, or submit form (`GET` / `POST`) |
| Hover / click `<a href="...">` link | Preview destination URL in the bottom status bar / navigate to link |
| Mouse wheel / Scrollbar / `Up` / `Down` / `PageUp` / `PageDown` / `Home` / `End` | Scroll the active page vertically |
| `Esc` | Cancel address/input editing, or close the window |

---

## License

Proprietary — All Rights Reserved. See [`LICENSE`](./LICENSE) for details.

## Author

**Buntos**
