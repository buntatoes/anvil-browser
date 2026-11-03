<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.26-ff6b24?style=flat-square" alt="v.26" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <a href="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml"><img src="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
    <img src="https://img.shields.io/badge/blocking-adblock--rust-ff6b24?style=flat-square" alt="adblock-rust" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <img src="https://img.shields.io/badge/go-1.22%2B-00ADD8?style=flat-square" alt="Go 1.22+" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security policy" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="Apache-2.0" /></a>
  </p>
</div>

Anvil is a small desktop browser for Windows and Linux built in Rust, Go, C++20, and TypeScript/JavaScript. It runs on the in-tree **Forge** (`anvil-forge`), **Prism**, and **Cinder** engines and links the open-source `adblock-rust` crate (`MPL-2.0`, see [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md)) for network and cosmetic ad/tracker blocking.

It is an experimental proof of concept, so expect rough edges. Current version: `0.26.0` (`v.26`). See [CHANGELOG.md](./CHANGELOG.md) for release history.

## Try it

Prebuilt Windows 64-bit (`AnvilBrowser-Setup-v.26-win64.exe` and `anvil.exe`) and Linux 64-bit (`AnvilBrowser-Setup-v.26-linux64` and `anvil`) binaries are on the [releases page](https://github.com/buntatoes/anvil-browser/releases).

## Build it

Requires Rust 1.99.1+, Go 1.22+, and a C++20 compiler (plus `libx11-dev` and `libssl-dev` on Linux). Clone this repository and [`adblock-rust`](https://github.com/brave/adblock-rust) side by side in the same parent directory, then run:

```sh
cargo test -- --test-threads=1
cargo run --release -p anvil-browser --bin anvil
```

## Overview

- **HTML, CSS, and Layout (`Forge` & `Prism`)**: Parses HTML5 with error recovery, Shadow DOM (`<slot>`), Custom Elements, inline/image `<svg>`, `<picture>`, `<canvas>`, `<dialog>`, tables, and form controls. Evaluates Selectors Level 4, `@layer` (`!important` inversion and `revert-layer`), `@scope`, `@container`, `@supports`, custom properties (`var(--*)`), CSS Color 4/5, Values 4 math (`calc()`, `min()`, `max()`, `clamp()`), Block/Inline/Flexbox/Grid layout, mixed inline formatting contexts, `position: sticky` and `position: absolute`, scrollable overflow containers (`overflow: auto | scroll`), and incremental style/layout caching (`LayoutCache`).
- **JavaScript and WebAssembly (`Cinder`)**: Executes ES2022-ES2024 scripts and WebAssembly 1.0 modules under a step and memory budget with per-`<script>` fault isolation, classes, async/generators, `Proxy`/`Reflect`, `BigInt`, 3-phase DOM events, `MutationObserver`, live scroll/resize `IntersectionObserver` & `ResizeObserver`, `fetch`/`FormData`, History API (`pushState`/`replaceState`/`popstate`), and 2D `<canvas>`.
- **Networking and TLS**: Speaks HTTP/1.1 and HTTP/2 (`h2` ALPN with HPACK; priority frames are read but not acted on yet), DEFLATE/zlib/gzip, cookies, HSTS, CSP, and SRI (`sha256-`, plus my own non-standard `sha3-256-`; `sha384-` and `sha512-` aren't checked yet). Live HTTPS connections use Linux OpenSSL (offering hybrid post-quantum `X25519MLKEM768` first) or Windows Schannel after the in-tree TLS 1.3 / `anvil-pqc` self-test passes.
- **Isolation and Shields**: Runs each tab in its own `--renderer` process (with optional Linux `bwrap`/`unshare`/`prlimit` confinement and Windows job/low-integrity limits), blocks network and cosmetic trackers via `adblock-rust`, and supports HTTPS-only mode, partitioned cookies, and per-site script/cookie permissions.
- **Desktop UI and DevTools**: Native C++20 shell (`Direct2D`/`DirectWrite`/`D3D11` on Windows; SDF + SFNT TrueType coverage rasterizer with native Wayland and X11 presentation on Linux). Includes adaptive tabs with hover previews, quick-filter tab search (`Ctrl+Shift+A`), domain-highlighted Omnibox with site Shields/permissions popover, Reader Mode (`Alt+R`), bookmarks bar (`Ctrl+Shift+B`), downloads shelf popover, find-in-page (`Ctrl+F` with case toggle and scrollbar match ticks), four themes, and a 5-tab `F12` DevTools dock (`Elements` with interactive click-to-select and viewport box-model overlay, `Styles`, `Network`, `Console`, `Storage`).
- **Media and Fonts**: Decodes PNG, JPEG, GIF, WebP, BMP, ICO, SVG, and TrueType/OpenType fonts (`glyf`, `CFF `, `kern`, `GPOS`). Video and audio use my own formats: `ADCT` (8x8 DCT with motion-compensated P-frames) and `AVID` for video, plus PCM, IMA-ADPCM, FLAC-style lossless frames, and a simple MDCT audio format, muxed into MP4 and WebM containers. The `VP8`, `VP9`, `AV1`, and `Opus` code reads and writes those formats' headers and framing (VP8 frame tags, VP9 uncompressed headers and superframes, AV1 OBUs, Opus TOC and packet framing) and carries my own simplified payloads with a few of their filters and transforms (for AV1, a CDEF-style filter). It doesn't decode real-world VP8, VP9, AV1, Opus, H.264, or AAC files yet, and the Opus path isn't real SILK or CELT. The `avc1` (H.264) stream wraps `ADCT` payloads, and AAC (`mp4a`) isn't implemented.

## Security

Report security issues privately via [SECURITY.md](./SECURITY.md), not in public issues.

## License

All original Anvil Browser code in this repository is licensed under the **Apache License, Version 2.0 (`Apache-2.0`)**, Copyright (c) 2026 Buntato. Under Section 6 ("Trademarks") of Apache-2.0, the names **"Anvil"**, **"Anvil Browser"**, **"Anvil Forge"** (**"Forge"**), **"Cinder"**, and **"Prism"** are proprietary trademarks of **Buntato**, and the Anvil emblem artwork (`assets/anvil-mark.png` and the icons made from it) is copyrighted by **Buntato** (*All Rights Reserved*). Other names that show up in the browser, like the search engines in the settings picker, are trademarks of their owners and are only used to say which search engine you're picking. The linked unmodified `adblock-rust` crate is licensed under **MPL-2.0**, and the Go runtime and standard library built into the Go helpers are under a BSD-style license; see [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md) and [NOTICE](./NOTICE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
