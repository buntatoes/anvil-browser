# flux-browser

[![Release](https://img.shields.io/badge/release-v.05-e8c37a)](https://github.com/buntatoes/flux-browser)
[![License: Proprietary](https://img.shields.io/badge/license-Proprietary-8b3a3a.svg)](./LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://github.com/buntatoes/flux-browser)

`flux-browser` is a ground-up web browser and custom rendering engine written in Rust with a privileged React/TSX front-end (`frontend/App.tsx`, `frontend/components/StartDashboard.tsx`, `frontend/components/SecurityCenter.tsx`), native window shell, hardened security sandbox, and cross-platform Windows/Linux installer (`flux-setup`).

It parses HTML into a depth- and node-bounded arena document tree, loads linked and embedded CSS stylesheets, decodes bounded `gzip`/`deflate` HTTP responses and `<img src="...">` raster images (PNG, JPEG, GIF89a, BMP, PPM/PGM), computes cascaded styles (tag, `.class`, `#id`, compound, and descendant selectors, inline `style` attributes, positioned layout, `z-index`, `overflow: hidden`, `box-shadow`, and `linear-gradient(...)`), lays out block, inline, Flexbox, and interactive form controls with TrueType font metrics, executes `<script>` blocks under deterministic instruction and recursion budgets with Same-Origin Policy (`SOP`) enforcement, emits a display list, rasterizes SDF rounded corners, box shadows, linear gradients, images, and anti-aliased TrueType text into an RGBA framebuffer, and presents interactive multi-tab pages with a security badge, status bar, bookmarks bar, vertical scrollbar, and asynchronous `GET`/`POST` networking.

---

## Release Highlights (`v.05`)

- **React/TSX Browser Front-End (`frontend/App.tsx`), Security Padlock Badge, Status Bar, & Built-In `flux://` Dashboards**:
  - Full React/TSX front-end architecture (`frontend/App.tsx`, `frontend/components/StartDashboard.tsx`, `frontend/components/SecurityCenter.tsx`, and `frontend/styles/browser.css`) compiled by `frontend.rs` directly into Flux DOM trees.
  - Launching `flux` with no arguments opens the window directly on the built-in **`flux://home`** start dashboard (`StartDashboard.tsx`), rendered by Flux's own engine with linear gradients, soft SDF box shadows, quick-access cards, and an interactive `<form action="flux://navigate">` launcher.
  - Built-in **`flux://security`** Security Center page (`SecurityCenter.tsx`) detailing all active transport, decompression, image, DOM, and script sandbox protections.
  - Upgraded browser shell (`App.tsx` + `ui.rs`) with SDF rounded tabs and buttons, a **Home (`H`)** button, a color-coded **Security Badge (`TLS`, `SYS`, `HTTP`, `FILE`)** next to the address bar that opens `flux://security`, a bottom **Status Bar** showing live hovered link URLs, security shield state, and scroll percentage, plus Windows 11 Dark Mode titlebar integration (`DwmSetWindowAttribute`).
  - Full keyboard shortcuts: `Ctrl+T` (new tab on `flux://home`), `Ctrl+W` (close tab), `Ctrl+L` / `F6` (focus address bar), `Ctrl+R` / `F5` (reload), `Alt+Left` / `Alt+Right` (back/forward), `Alt+Home` (`flux://home`), and `Home` / `End` (scroll top/bottom).
- **Massive Security Hardening Across All Layers**:
  - **Transport & Origin Sandbox (`flux-net`)**: Enforces 8 MiB raw response / 16 MiB decompressed body / 64 KiB header / 128 header-count limits, blocks `https://` -> `http://` protocol downgrade redirects, rejects userinfo (`@`) credential-phishing URLs and CRLF header/URL injection, blocks remote origins from referencing local files or `flux://` pages, exports `is_same_origin`, and sends privacy-hardened headers (`Referrer-Policy: no-referrer`, `DNT: 1`, `X-Content-Type-Options: nosniff`).
  - **Decompression-Bomb Guard (`flux-compress v.02`)**: Bounded DEFLATE, zlib, and gzip stream expansion (`decompress_deflate_bounded`, `decompress_zlib_bounded`, `decompress_gzip_bounded` with a 32 MiB ceiling) halting zip-bomb attacks immediately.
  - **Image Allocation-Bomb Guard (`flux-image v.03`)**: Enforces `MAX_IMAGE_DIMENSION = 8192` and `MAX_IMAGE_PIXELS = 16,777,216` (64 MiB RGBA cap) across PNG, JPEG, GIF89a, BMP, and PPM/PGM before buffer allocation.
  - **DOM & Script Execution Sandbox (`flux-dom` & `flux-script`)**: Enforces `MAX_DOM_DEPTH = 256`, `MAX_DOM_NODES = 50,000`, `MAX_ATTRS_PER_ELEMENT = 128`, `MAX_EXECUTION_STEPS = 100,000`, `MAX_CALL_DEPTH = 64`, `MAX_ARRAY_LEN = 10,000`, and Same-Origin Policy (`SOP`) on script `fetch()` calls.
- **Cross-Platform Native Installer (`flux-setup` / `flux install`) & Shader Parity (`flux-gpu v.03`)**:
  - Dedicated `flux-setup` installer binary (and `flux install` / `flux uninstall` CLI commands) supporting both **Windows** (installs to `%LOCALAPPDATA%\Programs\Flux Browser`, creates Start Menu and Desktop `.lnk` shortcuts, and registers the per-user Windows `Uninstall\FluxBrowser` registry entry) and **Linux** (installs to `~/.local/share/flux-browser` and `~/.local/bin/flux`, and writes an XDG `~/.local/share/applications/flux-browser.desktop` launcher entry).
  - **`flux-gpu v.03`** embeds both **HLSL Shader Model 5.0** (`shaders/compositor.hlsl` for Windows Direct3D) and **GLSL 330 Core** (`shaders/compositor.glsl` for Linux OpenGL/Vulkan).

---

## Rendering & Execution Pipeline

```text
URL / File / flux:// Input
  -> flux-net + flux-compress (HTTP/1.1 GET & POST, downgrade guard, bounded gzip/deflate, Schannel TLS)
  -> flux-dom + flux-image    (Bounded Document tree [depth<=256], forms, 16 MP capped PNG/JPEG/GIF/BMP/PPM)
  -> flux-script + flux-json  (100K-step budgeted interpreter, SOP-enforced fetch, timers, JSON, click events)
  -> flux-style               (CSS cascade, position/z-index/overflow, border-radius, box-shadow, gradients)
  -> flux-layout + flux-ttf   (Block, Inline, Flexbox, & Out-of-Flow layout, TrueType metrics, hit-testing)
  -> flux-paint               (DisplayList: clips, fixed layers, shadows, gradients, rounded rects, text)
  -> flux-raster + flux-gpu   (RGBA framebuffer, scissor stack, SDF shadows/gradients/corners, TTF glyphs)
  -> flux-browser             (Multi-tab GUI, flux://home & security pages, status bar, installer, Win32 shell)
```

---

## Workspace & Standalone Dependency Architecture

| Crate / Dependency | Responsibility |
| :--- | :--- |
| [`flux-dom`](./crates/flux-dom) | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations, `MAX_DOM_DEPTH = 256` and `MAX_DOM_NODES = 50,000` security caps, DOM query/mutation helpers, `classList`, form input/checkbox state, and `application/x-www-form-urlencoded` serialization), HTML parser, `<link rel="stylesheet">`, `<script>`, and `<img src>` discovery. |
| [`flux-style`](./crates/flux-style) | CSS stylesheet parser (tag, `.class`, `#id`, compound, and descendant selectors), inline `style="..."` attributes, user-agent defaults, property inheritance, `(ids, classes, tags)` specificity cascade, `position`/`top`/`right`/`bottom`/`left`/`z-index`/`overflow`, `border-radius`, `box-shadow`, `linear-gradient(...)`, and Flexbox properties. |
| [`flux-layout`](./crates/flux-layout) | Block, inline, Flexbox (`row` / `column`, `gap`, `justify-content`, `align-items`), and out-of-flow (`relative`, `absolute`, `fixed`) layout, interactive `<input>`/checkbox/button layout, intrinsic `<img>` sizing, TrueType proportional text measurement (`flux-ttf`), and `z-index`/clip-aware hit testing. |
| [`flux-paint`](./crates/flux-paint) | Converts the layout box tree into a `DisplayList` of `PushClip`/`PopClip`, `PushFixed`/`PopFixed`, `BoxShadow`, `LinearGradientRect`, `RoundedRect`, `Rect`, `Image`, underline, and `Text` commands ordered by `z-index`. |
| [`flux-raster`](./crates/flux-raster) | Software RGBA framebuffer rasterizer with scissor clip stack, fixed-layer scroll pinning, `flux-gpu` SDF rounded-corner, box-shadow, and linear-gradient rendering, `flux-image` bilinear image blitting, `flux-ttf` outline & Win32 GDI TrueType glyph rasterization, GDI BGRA output, and binary PPM export. |
| [`flux-script`](./crates/flux-script) | Proprietary scripting interpreter with deterministic step/recursion budgets (`MAX_EXECUTION_STEPS = 100,000`, `MAX_CALL_DEPTH = 64`), Same-Origin Policy (`SOP`) on `fetch()`, and DOM bindings (`let`/`const`, `if`/`else`, `while`, `for`, arrays, `getElementById`, `querySelector`, `createElement`, `textContent`, `value`, `checked`, `classList`, `style.*`, `setTimeout`, `JSON.parse`/`stringify`, and `click` listeners). |
| [`flux-net`](./crates/flux-net) | Hardened URL parser (rejecting `@` userinfo phishing and CRLF injection), relative HTTP/HTTPS/file/`flux://` URL resolver, Same-Origin Policy (`is_same_origin`), HTTP/1.1 `GET` and `POST` client with HTTPS downgrade protection, bounded `gzip`/`deflate` decompression (`flux-compress`), and native Windows Schannel (`secur32.dll`) TLS. |
| [`flux-browser`](./crates/flux-browser) | Multi-tab session, history, and bookmarks controller, built-in `flux://home` and `flux://security` front-end pages, SDF rounded toolbar/security badge/status bar UI, cross-platform Windows/Linux installer (`flux-setup` / `installer.rs`), asynchronous networking worker, Win32 dark-mode window shell, and `flux` CLI. |
| [`flux-compress`](https://github.com/buntatoes/flux-compress) | Standalone DEFLATE (RFC 1951), `zlib` (Adler-32), and `gzip` (CRC-32) compression and bounded decompression library (`v.02`). |
| [`flux-image`](https://github.com/buntatoes/flux-image) | Standalone PNG, Baseline JPEG, GIF87a/GIF89a, BMP, and Netpbm (PPM/PGM) image decoder, encoder, bilinear resampler, and 16 MP allocation-bomb guard (`v.03`). |
| [`flux-json`](https://github.com/buntatoes/flux-json) | Standalone zero-dependency RFC 8259 JSON parser, value tree, and compact/pretty serializer (`v.01`). |
| [`flux-ttf`](https://github.com/buntatoes/flux-ttf) | Standalone TrueType/OpenType SFNT font table parser (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, `kern`, `loca`, `glyf`), outline decoder, and anti-aliased scanline glyph rasterizer (`v.02`). |
| [`flux-gpu`](https://github.com/buntatoes/flux-gpu) | Standalone 2D signed-distance-field (SDF) quad compositor, linear gradients, box shadows, shelf-packing RGBA texture atlas, and cross-platform HLSL + GLSL shader pipeline (`v.03`). |

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
