<p align="center">
  <img src="./assets/flux-logo.png" width="108" height="108" alt="Flux Browser Logo" />
</p>

# flux-browser

[![Release](https://img.shields.io/badge/release-v.15-e8c37a.svg)](https://github.com/buntatoes/flux-browser/releases/tag/v.15)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20Linux-2ea44f.svg)](https://github.com/buntatoes/flux-browser)
[![Engine](https://img.shields.io/badge/engine-Rust%20%7C%20C%2B%2B20%20%7C%20JS-ff6b24.svg)](https://github.com/buntatoes/flux-browser)
[![GPU](https://img.shields.io/badge/GPU-HLSL%20%7C%20GLSL%20%7C%20NV12-58a6ff.svg)](https://github.com/buntatoes/flux-browser)
[![Frontend](https://img.shields.io/badge/frontend-React_TSX_%2B_JS-61dafb.svg)](https://github.com/buntatoes/flux-browser)
[![License: Apache-2.0 + Proprietary Security](https://img.shields.io/badge/license-Apache--2.0_%2B_Proprietary_Security-2ea44f.svg)](./LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://github.com/buntatoes/flux-browser)

`flux-browser` is a ground-up web browser and hybrid **Rust + C++20 + JavaScript** rendering engine with a privileged React/TSX & JavaScript front-end (`frontend/App.tsx`, `frontend/components/StartDashboard.tsx`, `frontend/components/SecurityCenter.tsx`, `frontend/components/SettingsPage.tsx`, `frontend/components/DownloadsPage.tsx`, `frontend/components/InspectorPage.tsx`, `frontend/components/HistoryPage.tsx`, `frontend/scripts/home.js`, `frontend/scripts/settings.js`), native C++20 rasterization and JIT compilation kernels (`flux_raster_accel.cpp`, `flux_jit_accel.cpp`), GPU-accelerated `NV12`/BT.709 video encode/decode + W3C Encrypted Media Extensions (`EME`) & ISO/IEC 23001-7 Common Encryption (`CENC`) (`flux-video v.03` + `flux-gpu v.04`), native cross-platform window shells (`Win32` on Windows and `X11` on Linux), native cross-platform TLS (`Schannel` on Windows and `OpenSSL` on Linux), hardcoded proprietary security sandbox, SHA-256 download verifier & quarantine (`flux://downloads`), built-in Developer Inspector dock (`F12` & `flux://inspect`), pre-socket tracker/ad/malware shield, and unified cross-platform Windows/Linux installer (`flux-setup`).

It parses HTML into a depth- and node-bounded arena document tree, loads linked and embedded CSS stylesheets, decodes bounded `gzip`/`deflate` HTTP responses, `<img src="...">` raster images (PNG, JPEG, GIF89a, WebP, ICO, BMP, PPM/PGM), and `<video src="...">` MP4/CENC/NV12 streams, computes cascaded styles (tag, `.class`, `#id`, compound, and descendant selectors, `@media` viewport & color-scheme queries, inline `style` attributes, positioned layout, `z-index`, `overflow: hidden`, `box-shadow`, and `linear-gradient(...)`), lays out block, inline, Flexbox, HTML `<table>` grids, and interactive form controls (`<input>`, `<textarea>`, `<select>`, checkboxes, buttons) with TrueType font metrics, executes modern `<script>` JavaScript (`flux-script` + C++20 JIT tier or Interpreter-Only Safe Mode) under deterministic instruction and recursion budgets with Same-Origin Policy (`SOP`) enforcement, emits a display list with dynamic `50%`–`250%` page zoom scaling, rasterizes SDF rounded corners, box shadows, linear gradients, images, and anti-aliased TrueType text into an RGBA framebuffer accelerated by native C++20 kernels, and presents interactive multi-tab pages with a Matte Charcoal & Flux Orange UI, vector toolbar icons, floating Top-Right App Menu (`≡`), docked Developer Inspector (`F12`), floating Find-in-Page (`Ctrl+F`) search bar, custom right-click context menu, system clipboard (`Ctrl+C`/`V`/`X`), security badge, status bar, bookmarks bar, vertical scrollbar, and asynchronous `GET`/`POST` networking.

---

## Release Highlights (`v.15`)

- **Hybrid C++20 / Rust / JavaScript Engine Architecture (`crates/flux-raster/cpp/flux_raster_accel.cpp`, `crates/flux-script/cpp/flux_jit_accel.cpp`, `crates/flux-script/src/lib.rs`)**:
  - **Native C++20 Framebuffer & SDF Rasterization Kernel (`flux_raster_accel.cpp`)**: Compiles freestanding C++20 (`/O2 /fp:fast /std:c++20`) directly into `flux-raster` via a zero-dependency `build.rs` script, accelerating Win32 bottom-up RGBA-to-BGRA framebuffer conversion (`flux_cpp_rgba_to_bgra_bottom_up`), scanline alpha-blended rectangle fills (`flux_cpp_fill_rect_rgba`), and anti-aliased SDF rounded rectangles (`flux_cpp_fill_rounded_rect_sdf`).
  - **Native C++20 JavaScript JIT Kernel & Expanded Web JS Engine (`flux_jit_accel.cpp` & `flux-script`)**: Adds a C++20 JIT numeric/loop evaluation kernel (`flux_cpp_jit_eval_binary`, `flux_cpp_jit_sum_range_i64`), inline DOM selector caching (`lookup_cached_dom`), and expanded modern JavaScript support (`return` values, arrow functions `() => {}`, object literals `{ key: val }`, template strings `` `${x}` ``, logical `&&`/`||`/`!`, ternary `? :`, `typeof`, `+=`/`-=`/`++`/`--`, `try`/`catch`, `console.log`/`warn`/`error`, `Math.*`, `String` & `Array` methods, `document.querySelectorAll`, `el.innerHTML`, `el.remove()`, and `window.location.href`).
  - **Privileged Front-End `.js` Controllers (`frontend/scripts/home.js` & `frontend/scripts/settings.js`)**: Executes embedded JavaScript controllers on `flux://home` and `flux://settings` for instant interactive section filtering and search bar focus.
- **Two-Column Browser Settings UI (`flux://settings`), Search Engine Logos, & JavaScript JIT Toggle (`frontend/components/SettingsPage.tsx`, `crates/flux-browser/src/logo.rs`)**:
  - **Two-Column Settings Layout**: Organizes `flux://settings` into a left vertical category sidebar (**All Settings**, **Search Engine**, **JavaScript & JIT**, **Privacy & Security**, **System & Hardware**) and a right column of structured setting cards with green/red status pills.
  - **Default Search Engine Selector with Brand Logos (`flux://engine-google.png`, `flux://engine-ddg.png`, `flux://engine-bing.png`)**: Moves the default search engine selector off `flux://home` into `flux://settings`, rendering anti-aliased SDF brand badges for **Google Search**, **DuckDuckGo**, and **Microsoft Bing** both inside the `flux://settings` engine cards and inside the left end of the centered search bar on `flux://home`.
  - **JavaScript Execution & JIT Compilation Toggles (`scripts_enabled` & `jit_enabled`)**: Lets users independently enable/disable website JavaScript execution (`flux://settings?toggle=scripts`) and toggle JavaScript JIT Compilation (`flux://settings?toggle=jit` — switching between the **Native C++20 JIT + Bytecode Tier** and **Rust Interpreter-Only Safe Mode**), persisted in the encrypted `profile.flx` container.
- **Three-Tier Hybrid Licensing (`Apache-2.0` Engine + `MPL-2.0` Companion Libraries + `Proprietary` Hardcoded Security)**:
  - **Open-Source `Apache-2.0` Core Engine**: Licenses the core HTML/CSS/layout/paint/C++20 rasterizer/JS engine crates (`flux-dom`, `flux-style`, `flux-layout`, `flux-paint`, `flux-raster`, `flux-script`) under the permissive **Apache License 2.0** with full source code hosted on GitHub.
  - **Hardcoded Proprietary Security & Browser Shell**: Retains **Proprietary — All Rights Reserved** protection over the hardcoded OS process sandbox (`flux-sandbox`), pre-socket tracker/malware shield & HSTS/CSP/TLS stack (`flux-net`), and DPAPI profile / SHA-256 quarantine shell (`flux-browser`).

- **Solid Blank Home Screen (`flux://home`), Integrated Search Engines (`Google`, `DuckDuckGo`, `Microsoft Bing`), & Brand Identity (`crates/flux-browser/src/frontend.rs`, `frontend/components/StartDashboard.tsx`, `assets/flux-logo.png`, `crates/flux-browser/src/logo.rs`)**:
  - **Minimal Solid Blank `flux://home` Screen**: Replaces card clutter on `flux://home` with a clean, solid matte-charcoal (`#12151c`) home screen featuring only the centered circular `F` ribbon emblem (`64x64`), the `Flux` wordmark, a wide centered `580px` search & URL input bar, and one-click search engine selector pills.
  - **Built-In `Google Search`, `DuckDuckGo`, & `Microsoft Bing` Integration (`SearchEngine`)**: Searches entered in either the centered `flux://home` search bar or the top Omnibox automatically route to the user's selected search engine (**Google Search** `https://www.google.com/search?q=...`, **DuckDuckGo** `https://duckduckgo.com/?q=...`, or **Microsoft Bing** `https://www.bing.com/search?q=...`) with RFC 3986 percent-encoding, persisted in `profile.flx` and switchable with one click on `flux://home` or in `flux://settings`.
  - **Official Circular `F` Ribbon Logo (`assets/flux-logo.png` & `logo.rs`)**: Integrates the new circular orange-red-white `F` ribbon emblem across the top-left window toolbar, `flux://home`, internal `flux://` headers, native Win32 window/taskbar icons, and Desktop/Start Menu `.ico` shortcuts.
  - **Calibrated Win32 GDI Font Metrics & Flexbox `<input>` Layout (`flux-raster`, `flux-layout`, `flux-paint`)**: Calibrates `CreateFontW` cell height (`0.90 * font_size`) and baseline centering to match layout glyph width advances, prevents single-line `<input>` controls from wrapping text vertically inside flex rows, and positions link underlines cleanly below glyph descenders.
- **Matte Charcoal & Flux Orange UI + Mouse-First Vector Toolbar (`crates/flux-browser/src/ui.rs`, `crates/flux-browser/src/shell.rs`, `frontend/styles/browser.css`)**:
  - **Matte Charcoal Dark Palette & Unified Omnibox Capsule**: Features a Matte Charcoal & Flux Orange palette (`#0e1015` tab strip, `#161920` toolbar, `#20242e` active tab with top orange accent `#ff6b24`, `#58a6ff` cyan, and `#6ee7b7` emerald), tab protocol status dots, and a unified recessed Omnibox capsule combining the security badge (`TLS`/`SYS`/`HTTP`/`FILE`), editable URL/search bar, and inline bookmark star (`★`).
  - **Custom Geometric Vector Icons & Instant Mouse-Hover Feedback**: Renders crisp geometric vector icons (`draw_nav_icon_back`, `draw_nav_icon_forward`, `draw_nav_icon_reload`, `draw_nav_icon_home`, `draw_nav_icon_star`, `draw_nav_icon_settings`, `draw_nav_icon_downloads`, `draw_nav_icon_inspector`, `draw_nav_icon_hamburger`, and `draw_close_x_icon`) with real-time `WM_MOUSEMOVE` hover highlights across every tab, close button, toolbar icon, menu row, and inspector tab.
- **Top-Right Unified App Menu Popover (`≡`) & Quick-Action Toolbar (`crates/flux-browser/src/ui.rs`, `crates/flux-browser/src/shell.rs`, `frontend/App.tsx`)**:
  - **Quick-Action Toolbar Cluster**: Adds one-click mouse buttons on the right of the Omnibox for **Zoom Pill (`100%`)**, **Settings (`⚙`)**, **Downloads (`↓`)**, **Developer Inspector (`<>`)**, and **App Menu (`≡`)**.
  - **Floating 10-Row App Menu Popover (`AppMenuAction`)**: Clicking `≡` opens an SDF-shadowed popover menu with mouse-clickable rows and shortcut badges for *New Tab (`Ctrl+T`)*, *Home Dashboard (`Alt+Home`)*, *Downloads & Quarantine (`Ctrl+J`)*, *Browsing History (`Ctrl+H`)*, *Security & Shield Center (`TLS/SYS`)*, *Find in Page (`Ctrl+F`)*, *Developer Inspector Dock (`F12`)*, *Full Inspector Page (`flux://inspect`)*, *Cycle Page Zoom (`Ctrl++`)*, and *Browser Settings (`flux://settings`)*.
- **Built-In Developer Tools / Inspector Dock (`F12`) & Full-Page Inspector (`flux://inspect`) (`crates/flux-browser/src/ui.rs`, `crates/flux-browser/src/frontend.rs`, `frontend/components/InspectorPage.tsx`)**:
  - **Interactive Bottom Inspector Dock (`compose_inspector_dock`)**: Toggleable via `F12`, the `<>` toolbar button, right-click -> *Inspect Page (`F12`)*, or the `≡` App Menu, featuring 4 mouse-clickable tabs (**Elements (DOM)**, **Styles & Layout**, **Network & Shield**, and **Console & Sandbox**), a `[Full Page]` launcher button, and a `[×]` close button.
  - **Full-Page Developer Inspector (`flux://inspect`)**: Dedicated React/TSX diagnostic page (`InspectorPage.tsx`) inspecting live DOM arena node/depth utilization, layout & display-list geometry, HSTS/SRI/Shield network telemetry, and script sandbox step/call budgets.
- **W3C Encrypted Media Extensions (`EME`) & ISO/IEC 23001-7 Common Encryption (`CENC`) (`flux-video v.03`, `flux-script`, `flux-browser`)**:
  - **ISO/IEC 23001-7 `CENC` Encrypted MP4 Container & Sample Decryption**: Parses and muxes `sinf` (`schm` + `tenc`) and W3C Common `pssh` protection headers inside `.mp4` containers (`wrap_encrypted_mp4_container`, `inspect_mp4_metadata`, `cenc_encrypt_sample`, `cenc_decrypt_sample`, `generate_encrypted_demo_mp4`).
  - **W3C `org.w3.clearkey` & Vendor-Neutral CDM Host Session (`CdmSession`, `EmeKeySystem`)**: Implements built-in W3C ClearKey (`org.w3.clearkey`), OS Media Foundation / DRM Bridge (`org.flux.mediafoundation.cdm`), and External CDM Host Plugin Slot (`org.flux.external.cdm`) session management, `navigator.requestMediaKeySystemAccess(...)` in `flux-script`, and `flux://encrypted.mp4` playback.
- **Hardened Download Manager, SHA-256 Verifier, & Safe File Quarantine (`crates/flux-browser/src/downloads.rs`, `frontend/components/DownloadsPage.tsx`, `flux://downloads`)**:
  - **FIPS 180-4 SHA-256 Checksum & Quarantine Policy (`DownloadManager`)**: Computes the 64-character SHA-256 hex digest of every downloaded payload, sanitizes filenames against directory traversal (`..`, `/`, `\`), quarantines executable/script extensions (`.exe`, `.bat`, `.cmd`, `.ps1`, `.scr`, `.vbs`, `.msi`, `.dll`) as `QuarantinedExecutable`, tags saved files on Windows with the Mark-of-the-Web (`Zone.Identifier` `ZoneId=3`), and persists the download log in `profile.flx`.
- **Windows DPAPI (`crypt32.dll`) At-Rest Profile Protection (`crates/flux-browser/src/profile.rs`)**:
  - **OS-Managed User-Credential Sealing (`CryptProtectData` / `CryptUnprotectData`)**: Wraps the compressed `FLXP\x01` profile container (`profile.flx` — bookmarks, browsing history, cookies, user settings, and HSTS policy table) via Windows Data Protection API (`crypt32.dll`, `CRYPTPROTECT_UI_FORBIDDEN`) tied to the active Windows user account, paired with a 64-bit integrity verification tag.
- **`Strict-Transport-Security` (`HSTS`) Store & Subresource Integrity (`SRI` SHA-256) (`crates/flux-net/src/policy.rs`, `flux-dom`, `flux-browser`)**:
  - **HSTS Preload & Dynamic Header Enforcement (`HstsStore`)**: Parses `Strict-Transport-Security: max-age=...; includeSubDomains` headers, maintains a preloaded + persistent HSTS policy table, and automatically upgrades `http://` navigations and subdomain requests to `https://` before any socket opens.
  - **FIPS 180-4 SHA-256 Subresource Integrity (`verify_subresource_integrity`)**: Extracts `integrity="sha256-..."` attributes on external `<script src="..." integrity="...">` and `<link rel="stylesheet" href="..." integrity="...">` elements and verifies the cryptographic SHA-256 digest of fetched payloads before script execution or CSS cascade evaluation.
- **Universal Tab Fault Containment (`MandatorySandboxGuard::execute_isolated`) (`crates/flux-browser/src/lib.rs` & `frontend/components/SecurityCenter.tsx`)**:
  - **Per-Tab Fault Isolation Across All Navigations**: Executes `open_html`, `open_url`, `open_target`, and `reload_active` inside the mandatory OS sandbox fault boundary so any unexpected parser or script panic is isolated to that single tab's recovery card (`render_sandbox_recovery_page`) while other open tabs and the browser process remain healthy.
- **Persistent Local Profile & Session Storage (`crates/flux-browser/src/profile.rs` + `Session`)**:
  - **Compressed & Integrity-Verified Profile Container (`FLXP\x01` / `profile.flx`)**: Serializes bookmarks, browsing history, user settings (`gpu_video_acceleration`, `https_only_mode`, `scripts_enabled`), open tab URLs, and the RFC 6265bis `CookieJar` via `flux-json v.02`, compresses the snapshot with `flux-compress v.02` (`zlib`), and seals the container with a 64-bit integrity verification tag.
  - **Automatic & On-Demand Persistence**: Automatically restores the local profile on startup (`Session::load_default_profile`), persists changes on bookmark toggle or window exit (`Session::save_default_profile`), and exposes manual **Save Profile Now** (`flux://settings?action=save_profile`) and **Clear All Local Data** (`flux://settings?action=clear_profile`) controls inside `flux://settings`.
- **Interactive `<video>` Timeline Scrubbing & Playback Bar (`lib.rs`, `shell.rs`, `flux-video v.02`)**:
  - **On-Video SDF Playback HUD (`overlay_video_controls_hud`)**: Overlays a translucent bottom control bar (`rgba(9, 13, 20, 214)`) with an interactive **Play / Pause (`>` / `||`)** button pill, a timeline scrubber track + progress bar + playhead knob, and a live `current / duration` timestamp label (`0:00 / 0:03`).
  - **Click-to-Seek & Frame Stepping (`VideoPlayerState` & `Session::seek_video`)**: Clicking the play/pause area toggles playback (`ClickOutcome::VideoToggled`), while clicking anywhere along the timeline scrubber bar immediately seeks to that timestamp (`ClickOutcome::VideoSeeked`) and updates the `<video>` surface via `VideoClip::frame_at_ms(timestamp_ms)`.
- **Heavily Refined React/TSX Front-End UI, Omnibox Autocomplete, & Shell Polish (`frontend/*.tsx`, `frontend/styles/browser.css`, `frontend.rs`, `ui.rs`, `shell.rs`)**:
  - **Upgraded React/TSX Shell & Dashboards (`App.tsx`, `StartDashboard.tsx`, `SettingsPage.tsx`, `browser.css`)**: Adds linear-gradient surfaces across the tab strip, active tab, navigation toolbar, status bar, cards, and sections; adds a clickable **Zoom Pill (`100%`)** and **Settings (`S`)** button to the toolbar; adds Section 4 (**Encrypted Local Profile & Session Persistence**) to `SettingsPage.tsx`; and adds a Quick Keyboard & Interactive Controls reference table to `StartDashboard.tsx`.
  - **Omnibox `Tab` Autocomplete & Hover Feedback (`ui.rs` & `shell.rs`)**: Suggests matching `flux://` internal routes, saved bookmarks, and history entries inline while typing in the address bar (`Tab` to accept), tracks live cursor hover states across tabs, toolbar buttons, bookmarks, and context menu items, and renders multi-layer drop shadows behind the floating Find bar and right-click context menu.
- **Upgraded Standalone Media, Image, & Font Dependencies (`flux-image v.04`, `flux-video v.02`, `flux-ttf v.03`)**:
  - **WebP (`VP8L` / `VP8X`) & Windows `.ico` Support (`flux-image v.04`)**: Adds RIFF `WEBP` (`decode_webp`, `encode_webp`) and multi-entry Windows `.ico` (`decode_ico`, `encode_ico`) decoding and encoding to `decode_auto` under the strict 16 MP (`MAX_IMAGE_PIXELS = 16,777,216`) allocation-bomb guard.
  - **ISOBMFF (`.mp4`) Container Hardening & Timestamp Frame Seeking (`flux-video v.02`)**: Enforces `MAX_CONTAINER_BYTES = 64 MiB`, `MAX_MP4_BOXES = 256`, and `MAX_MP4_BOX_DEPTH = 16` against nested atom-bomb streams, adds zero-decode `inspect_mp4_metadata`, and adds `VideoClip::frame_at_ms(timestamp_ms)`.
  - **Untrusted Web-Font Security Caps & SFNT `name` Table (`flux-ttf v.03`)**: Enforces `MAX_FONT_BYTES = 32 MiB`, `MAX_SFNT_TABLES = 128`, `MAX_GLYPH_CONTOURS = 256`, `MAX_CONTOUR_POINTS = 4096`, and `MAX_COMPOSITE_GLYPH_DEPTH = 16` against cyclic composite-glyph font exploits, and parses the SFNT `name` table (`TtfFont::family_name`).

- **Pre-Socket Tracker, Ad-Tech, & Malicious Domain Shield (`crates/flux-net/src/shield.rs` + `flux-browser`)**:
  - **Zero-Connection Blocking (`NetworkShield` & `is_url_blocked_by_default_shield`)**: Intercepts known ad networks, behavioral telemetry/analytics beacons, browser fingerprinting scripts, and phishing/cryptomining/malware hosts (`ShieldBlockReason::{AdNetwork, TrackerOrAnalytics, Fingerprinter, MaliciousOrPhishing, CustomRule}`) before any DNS lookup or TCP/TLS socket connection is opened.
  - **Live Shield Telemetry**: Tracks blocked subresources per page (`Page::blocked_tracker_count`) and across the session, surfacing live shield status (`Shields: ON (...)`) in the bottom status bar and `flux://security`.
- **System Clipboard (`Ctrl+C` / `Ctrl+V` / `Ctrl+X`), Text Selection, & Right-Click Context Menu (`ui.rs`, `shell.rs`, `lib.rs`)**:
  - **Native Clipboard & On-Page Text Selection**: Integrates Win32 `CF_UNICODETEXT` (`OpenClipboard` / `GetClipboardData` / `SetClipboardData`) and session clipboard state for copying selected page text (`select_text_at`) or focused input text (`Ctrl+C`), cutting (`Ctrl+X`), and pasting into the address bar, Find bar, or `<input>`/`<textarea>` controls (`Ctrl+V`).
  - **Custom Right-Click Context Menu (`WM_RBUTTONDOWN` / `Button3`)**: Floating SDF-styled context menu with quick actions for *Back*, *Forward*, *Reload Page*, *Copy (`Ctrl+C`)*, *Paste (`Ctrl+V`)*, *Find in Page... (`Ctrl+F`)*, *Reset Zoom (100%)*, and *Open Security Shield*.
- **Find-in-Page (`Ctrl+F`) & Dynamic Page Zoom (`50%`–`250%`, `Ctrl++` / `Ctrl+-` / `Ctrl+0`) (`ui.rs`, `shell.rs`, `lib.rs`, `flux-paint`)**:
  - **Floating Find-in-Page Search Bar (`Ctrl+F`)**: Case-insensitive text search across the active page's display list (`Page::find_text_matches`), highlighting all matching text runs in the viewport with a live match counter (`1/3`), previous/next (`<` / `>` / `Enter`) navigation, and automatic smooth scroll-to-match.
  - **Dynamic Viewport Page Zoom (`50%`–`250%`)**: Scales display list geometry, rounded radii, shadows, images, and TrueType glyph sizes (`DisplayList::scaled`) via `Ctrl++`, `Ctrl+-`, `Ctrl+0`, or `Ctrl+MouseWheel`, displaying the live zoom level in the status bar.
- **Process Isolation, Content Security Policy (`CSP`), Cross-Origin Resource Policy (`CORP`), & Cookie Jar (`flux-net` & `flux-dom`)**:
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
- **GPU-Accelerated `NV12` / BT.709 Video Encode & Decode (`flux-video v.02` + `flux-gpu v.04`) & HTML `<video>` Support**:
  - Dedicated standalone **`flux-video v.02`** library and **`flux-gpu v.04`** `NV12` BT.709 GPU surface shaders (`ConvertNv12Bt709ToRgba` and `ConvertRgbaToYuvBt709` in both HLSL Shader Model 5.0 and GLSL 330 Core).
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
  - **Image Allocation-Bomb Guard (`flux-image v.04`)**: Enforces `MAX_IMAGE_DIMENSION = 8192` and `MAX_IMAGE_PIXELS = 16,777,216` (64 MiB RGBA cap) across PNG, JPEG, GIF89a, WebP, ICO, BMP, and PPM/PGM before buffer allocation.
  - **DOM & Script Execution Sandbox (`flux-dom` & `flux-script`)**: Enforces `MAX_DOM_DEPTH = 256`, `MAX_DOM_NODES = 50,000`, `MAX_ATTRS_PER_ELEMENT = 128`, `MAX_EXECUTION_STEPS = 100,000`, `MAX_CALL_DEPTH = 64`, `MAX_ARRAY_LEN = 10,000`, and Same-Origin Policy (`SOP`) on script `fetch()` calls.
- **Cross-Platform Unified Setup & Uninstaller (`flux-setup` / `FluxBrowser-Setup-v.15-win64.exe`), Brand Logo (`flux://logo.png` / `flux-logo.ico`), & Shader Parity (`flux-gpu v.04`)**:
  - Unified single-executable `flux-setup` installer and Control Panel uninstaller supporting both **Windows** (installs `flux.exe`, `Installer\setup.exe`, `flux-logo.ico`, and `flux-logo.png` to `%LOCALAPPDATA%\Programs\Flux Browser`, embeds an official Windows 10/11 `asInvoker` XML Application Manifest, creates icon-branded Start Menu and Desktop `.lnk` shortcuts, and registers the per-user Windows `Uninstall\FluxBrowser` registry entry with `DisplayIcon` and `UninstallString` pointing to `Installer\setup.exe --uninstall`) and **Linux** (installs to `~/.local/share/flux-browser` and `~/.local/bin/flux`, and writes an XDG `~/.local/share/applications/flux-browser.desktop` launcher entry).
  - Official **Flux Browser Logo (`crates/flux-browser/src/logo.rs` + `assets/flux-logo.png`)** rendered in the top-left of the browser window toolbar, top-left of all internal `flux://` pages (`flux://settings`, `flux://home`, `flux://downloads`, `flux://inspect`, `flux://security`, `flux://history`) via `flux://logo.png`, the native Win32 window titlebar/taskbar icon (`CreateIcon` + `WM_SETICON`), and installed shortcut `.ico` files.
  - **`flux-gpu v.04`** embeds both **HLSL Shader Model 5.0** (`shaders/compositor.hlsl` for Windows Direct3D) and **GLSL 330 Core** (`shaders/compositor.glsl` for Linux OpenGL/Vulkan).

---

## Rendering & Execution Pipeline

```text
URL / File / flux:// Input
  -> flux-net + flux-compress (HTTP/1.1 GET & POST, HSTS, SRI SHA-256, bounded gzip/deflate, Schannel/OpenSSL TLS)
  -> flux-dom + flux-image + flux-video (Bounded DOM [depth<=256], tables/forms, 16 MP images, NV12/CENC MP4 video)
  -> flux-script + flux-json  (100K-step budgeted interpreter, EME access, SOP fetch, timers, JSON, click events)
  -> flux-style               (CSS cascade, @media queries, position/z-index/overflow, shadows, gradients)
  -> flux-layout + flux-ttf   (Block, Inline, Flexbox, Table, & Out-of-Flow layout, TrueType metrics, hit-test)
  -> flux-paint               (DisplayList: clips, fixed layers, shadows, gradients, rounded rects, text)
  -> flux-raster + flux-gpu   (RGBA framebuffer, scissor stack, SDF shadows/gradients/corners, TTF glyphs)
  -> flux-browser             (Multi-tab React/TSX GUI, App Menu, Inspector Dock, DPAPI profile, Win32 & X11 shells)
```

---

## Workspace & Standalone Dependency Architecture

| Crate / Dependency | Version | Responsibility |
| :--- | :---: | :--- |
| `flux-dom` | `v.15` | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations, `MAX_DOM_DEPTH = 256` and `MAX_DOM_NODES = 50,000` security caps, `querySelectorAll`, `set_inner_html`, `remove_node`, `classList`, `<table>` auto-closing, `<input>`/`<textarea>`/`<select>`/checkbox state, and `application/x-www-form-urlencoded` serialization), HTML parser, `<meta http-equiv="Content-Security-Policy">`, `<link rel="stylesheet" integrity="...">`, `<script integrity="...">`, `<img src>`, and `<video src>` discovery (`Apache-2.0`). |
| `flux-style` | `v.15` | CSS stylesheet parser (tag, `.class`, `#id`, compound, and descendant selectors), `@media` viewport & color-scheme queries (`compute_styles_with_viewport`), inline `style="..."` attributes, user-agent defaults, property inheritance, `(ids, classes, tags)` specificity cascade, `position`/`top`/`right`/`bottom`/`left`/`z-index`/`overflow`, `border-radius`, `box-shadow`, `linear-gradient(...)`, and Flexbox properties (`Apache-2.0`). |
| `flux-layout` | `v.15` | Block, inline, Flexbox (`row` / `column`, `gap`, `justify-content`, `align-items`), HTML `<table>` (`layout_table_row`), and out-of-flow (`relative`, `absolute`, `fixed`) layout, interactive `<input>`/`<textarea>`/`<select>`/checkbox/button layout, intrinsic `<img>` and `<video>` sizing, TrueType proportional text measurement (`flux-ttf`), and `z-index`/clip-aware hit testing (`Apache-2.0`). |
| `flux-paint` | `v.15` | Converts the layout box tree into a `DisplayList` of `PushClip`/`PopClip`, `PushFixed`/`PopFixed`, `BoxShadow`, `LinearGradientRect`, `RoundedRect`, `Rect`, `Image`, underline, and `Text` commands ordered by `z-index`, with dynamic `50%`–`250%` page zoom scaling (`DisplayList::scaled`) (`Apache-2.0`). |
| `flux-raster` | `v.15` | Hybrid Rust + C++20 (`cpp/flux_raster_accel.cpp`) RGBA framebuffer rasterizer with scissor clip stack, fixed-layer scroll pinning, `flux-gpu` SDF rounded-corner, box-shadow, and linear-gradient rendering, `flux-image` bilinear image blitting, `flux-ttf` outline & Win32 GDI / Linux TrueType glyph rasterization, Win32 bottom-up & X11 top-down BGRA output, and binary PPM export (`Apache-2.0`). |
| `flux-script` | `v.15` | Hybrid Rust + C++20 (`cpp/flux_jit_accel.cpp`) JavaScript engine & optional JIT compiler (`with_jit_enabled`) with deterministic step/recursion budgets (`MAX_EXECUTION_STEPS = 100,000`, `MAX_CALL_DEPTH = 64`), Same-Origin Policy (`SOP`) on `fetch()`, W3C `navigator.requestMediaKeySystemAccess(...)`, and modern DOM/JS bindings (`let`/`const`/`var`, `return`, arrow functions, objects, template strings, `try`/`catch`, `console.*`, `Math.*`, `String`/`Array` methods, `getElementById`, `querySelector`, `querySelectorAll`, `innerHTML`, `createElement`, `remove`, `textContent`, `value`, `checked`, `classList`, `style.*`, `setTimeout`, `JSON.parse`/`stringify`, and `click` listeners) (`Apache-2.0`). |
| `flux-net` | `v.15` | Hardened URL parser (rejecting `@` userinfo phishing and CRLF injection), Pre-Socket Tracker/Ad/Malware Shield (`NetworkShield` & `is_url_blocked_by_default_shield`), `Strict-Transport-Security` (`HstsStore`), FIPS 180-4 SHA-256 Subresource Integrity (`verify_subresource_integrity`), relative HTTP/HTTPS/file/`flux://` URL resolver, Same-Origin Policy (`is_same_origin`), `Content-Security-Policy` (`CSP`), `Cross-Origin-Resource-Policy` (`CORP`), RFC 6265bis `CookieJar` (`HttpOnly`, `Secure`, `SameSite`, `__Host-`), HTTP/1.1 `GET` and `POST` client with HTTPS downgrade protection, bounded `gzip`/`deflate` decompression (`flux-compress`), and native Windows Schannel + Linux OpenSSL TLS (`Proprietary`). |
| `flux-sandbox` | `v.15` | Mandatory OS process isolation (Windows Job Object / Linux `rlimit`), tab worker communication, and per-tab fault containment (`Proprietary`). |
| `flux-browser` | `v.15` | Multi-tab session, Matte Charcoal & Flux Orange UI with vector toolbar icons, Top-Right App Menu (`≡`), Developer Inspector Dock (`F12` & `flux://inspect`), Two-Column Browser Settings (`flux://settings`) with search engine logos (`Google`, `DuckDuckGo`, `Bing`) and JavaScript/JIT toggles, Windows DPAPI (`crypt32.dll`) profile container (`profile.rs`), SHA-256 Download Manager & Quarantine (`downloads.rs`), history, bookmarks, Omnibox autocomplete, Find-in-Page (`Ctrl+F`), `50%`–`250%` viewport zoom, interactive `<video>` scrubber HUD, system clipboard (`Ctrl+C`/`V`/`X`), right-click context menu, unified Windows/Linux installer & uninstaller (`flux-setup`), native Win32 & Linux X11 window shells, and `flux` CLI (`Proprietary`). |
| [`flux-compress`](https://github.com/buntatoes/flux-compress) | `v.02` | Standalone DEFLATE (RFC 1951), `zlib` (Adler-32), and `gzip` (CRC-32) compression and bounded decompression library (`MPL-2.0`). |
| [`flux-image`](https://github.com/buntatoes/flux-image) | `v.04` | Standalone PNG, Baseline JPEG, GIF87a/GIF89a, WebP (`VP8L`/`VP8X`), Windows `.ico`, BMP, and Netpbm (PPM/PGM) image decoder, encoder, bilinear resampler, and 16 MP allocation-bomb guard (`MPL-2.0`). |
| [`flux-json`](https://github.com/buntatoes/flux-json) | `v.02` | Standalone zero-dependency RFC 8259 bounded JSON parser (`MAX_JSON_DEPTH = 64`, `MAX_JSON_BYTES = 16 MiB`), value tree, and compact/pretty serializer (`MPL-2.0`). |
| [`flux-ttf`](https://github.com/buntatoes/flux-ttf) | `v.03` | Standalone TrueType/OpenType SFNT font table parser (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, `kern`, `loca`, `glyf`, `name`), bounded outline decoder, and anti-aliased scanline glyph rasterizer (`MPL-2.0`). |
| [`flux-gpu`](https://github.com/buntatoes/flux-gpu) | `v.04` | Standalone 2D signed-distance-field (SDF) quad compositor, `NV12` BT.709 GPU video surfaces (`Nv12Surface`), shelf-packing RGBA texture atlas, and cross-platform HLSL + GLSL shader pipeline (`MPL-2.0`). |
| [`flux-video`](https://github.com/buntatoes/flux-video) | `v.03` | Standalone GPU-accelerated `NV12`/BT.709 video encoder and decoder (`GpuVideoSession`, `D3d11Dxva2`, `LinuxVaApi`), temporal `I-frame`/`P-frame` codec, ISO/IEC 23001-7 `CENC` encryption, W3C `EME`/`CDM` session (`CdmSession`), and hardened ISOBMFF (`.mp4`) container muxer/demuxer (`MPL-2.0`). |

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
flux open flux://inspect

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
| Click tab / `×` / `+` (`Ctrl+T` / `Ctrl+W`) | Switch active tab, close tab, or open a new tab on `flux://home` |
| Click `←` / `→` (`Alt+Left` / `Alt+Right`) | Navigate back or forward in tab history |
| Click `↻` (`Ctrl+R` / `F5`) | Reload the active page |
| Click `⌂` (`Alt+Home`) | Navigate to the built-in `flux://home` dashboard |
| Click `TLS` / `SYS` / `HTTP` / `FILE` badge | Open the built-in `flux://security` Security Center |
| Click `★` / bookmark pill | Bookmark the active page or open a saved bookmark |
| Click `100%` / `⚙` / `↓` / `<>` / `≡` | Cycle zoom, open `flux://settings`, open `flux://downloads` (`Ctrl+J`), toggle Inspector Dock (`F12`), or open the Top-Right App Menu (`≡`) |
| Click address field (`Ctrl+L` / `F6`) | Focus the address field (`Enter` to navigate, `Tab` to autocomplete, `Esc` to cancel) |
| `F12` | Toggle the bottom Developer Inspector Dock (`Elements`, `Styles`, `Network`, `Console`, or `[Full Page]` -> `flux://inspect`) |
| `Ctrl+J` / `Ctrl+H` | Open `flux://downloads` (Download Manager) or `flux://history` (Browsing History) |
| `Ctrl+F` | Open the floating Find-in-Page search bar (`Enter` / `>` next, `<` previous, `Esc` close) |
| `Ctrl+C` / `Ctrl+X` / `Ctrl+V` | Copy selected text or focused input, cut, or paste from system clipboard |
| `Ctrl++` / `Ctrl+-` / `Ctrl+0` / `Ctrl+Wheel` | Zoom in, zoom out (`50%`–`250%`), or reset page zoom (`100%`) |
| Right-click anywhere in window | Open the custom context menu (*Back*, *Forward*, *Reload*, *Copy*, *Paste*, *Find in Page*, *Reset Zoom*, *Inspect Page (`F12`)*, *Security Shield*) |
| Click page `<input>` / checkbox / button | Focus text input (`Enter` to submit `<form>`, `Esc` to blur), toggle checkbox, or submit form (`GET` / `POST`) |
| Hover / click `<a href="...">` link | Preview destination URL in the bottom status bar / navigate to link |
| Mouse wheel / Scrollbar / `Up` / `Down` / `PageUp` / `PageDown` / `Home` / `End` | Scroll the active page vertically |
| `Esc` | Close App Menu, Inspector Dock, context menu, or Find bar; cancel address/input editing; or close the window |

---

## Three-Tier Hybrid Licensing (`Apache-2.0` + `MPL-2.0` + `Proprietary`)

See [`LICENSE`](./LICENSE) for the complete Three-Tier Hybrid License terms:

- **Tier 1 — Core Rendering, Layout, C++20 Raster & JavaScript Engine (`Apache-2.0`)**: `crates/flux-dom`, `crates/flux-style`, `crates/flux-layout`, `crates/flux-paint`, `crates/flux-raster` (including `cpp/flux_raster_accel.cpp`), and `crates/flux-script` (including `cpp/flux_jit_accel.cpp`) are licensed under the permissive **Apache License, Version 2.0**.
- **Tier 2 — Hardcoded Security, OS Process Sandbox, Shield & Browser Application (`Proprietary — All Rights Reserved`)**: `crates/flux-sandbox`, `crates/flux-net`, and `crates/flux-browser` (including DPAPI profile sealing, SHA-256 download quarantine, and official `Flux Browser` branding) are proprietary software authored by **Buntos** (`LicenseRef-Proprietary`).
- **Tier 3 — Standalone Companion Libraries (`MPL-2.0`)**: The standalone companion libraries (`flux-compress`, `flux-image`, `flux-json`, `flux-ttf`, `flux-gpu`, and `flux-video`) are authored by **Buntos** under the file-level `MPL-2.0` license, which is 100% compatible with both the `Apache-2.0` engine crates and the proprietary security/browser crates.

## Author

**Buntos**
