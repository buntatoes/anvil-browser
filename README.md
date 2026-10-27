<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.23.2-ff6b24?style=flat-square" alt="v.23.2" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <img src="https://img.shields.io/badge/total_LOC-188%2C568-ff6b24?style=flat-square" alt="Total LOC: 188,568" />
    <img src="https://img.shields.io/badge/external_crates-0_%28offline_verified%29-2ea44f?style=flat-square" alt="0 external crates (offline verified)" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security policy" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/browser-Apache--2.0-blue?style=flat-square" alt="Browser: Apache-2.0" /></a>
    <a href="./LICENSE-MPL"><img src="https://img.shields.io/badge/libraries-MPL--2.0-6f42c1?style=flat-square" alt="Libraries: MPL-2.0" /></a>
    <a href="./LICENSE-PROPRIETARY"><img src="https://img.shields.io/badge/engine-Proprietary-e8c37a?style=flat-square" alt="Engine: proprietary" /></a>
  </p>
</div>

Anvil is a small desktop browser built from scratch in Rust, with a bit of C++20 and JavaScript. It has no outside crates: every dependency lives in one of the Anvil repos. It runs on Linux and Windows.

It's a proof of concept, so expect rough edges. Current version: 0.23.2.

This is the `main` release line (`v.23.2`).

See [CHANGELOG.md](./CHANGELOG.md) for what changed in each release.

## Try it

Windows 64-bit binaries (`AnvilBrowser-Setup-v.23.2-win64.exe` and portable `anvil.exe`) are attached to the [v.23.2 release](https://github.com/buntatoes/anvil-browser/releases/tag/v.23.2), or build it from source below.

## Build it

You need Rust 1.99.1 or newer. On Linux you also need the X11 and OpenSSL development files (`sudo apt install libx11-dev libssl-dev` on Debian or Ubuntu). A C++20 compiler is optional.

Clone this repo, [anvil-engine](https://github.com/buntatoes/anvil-engine), and the eleven libraries below side by side in one folder, then run this from `anvil-browser`:

```sh
cargo test
cargo run --release -p anvil-browser --bin anvil
```

## What it does

- HTML (including `<script>` comment-preserving tokenization), CSS (flex, grid, tables, media queries, transitions, keyframes), and layout, drawn through the CPU, persistent per-thread `anvil-gpu` compositor/tile devices (including rounded-rect box fills and borders), and bounded LRU glyph/image caches (`GlyphLruCache` and `ImageLruCache`)
- Style sheets apply in document order: `<style>` blocks and `<link rel="stylesheet">` sheets keep their place in the page, each sheet is parsed on its own, and a sheet's `media` attribute is honoured the same way as `@media`, so a print sheet doesn't apply on screen and a broken sheet doesn't swallow the next one. The page's CSS is parsed once and only parsed again when a sheet changes, so animation frames and window resizes reuse it
- A JavaScript and WebAssembly 1.0 (`WASM`) runtime (`anvil-js` self-hosted ES2022+ compiler and Web/DOM runtime executing on the C++20 `AnvilVM` register bytecode VM, `WasmModule`, `WasmInstance`, `WasmMemory`, `WebAssembly` JS namespace, `TypedArray` views, `ArrayBuffer`, `TextEncoder`, `TextDecoder`, `atob`/`btoa`, `URLSearchParams`, `DOMParser`, `CustomElementRegistry`, `ShadowRoot`, `MediaSource`, `MutationObserver`, `IntersectionObserver`, `ResizeObserver`, and `--renderer` interactive click hydration via `hydrate_interactive_bindings`) with a time/step budget
- HTTP/1.1 and HTTP/2 (over ALPN `h2` on both Windows Schannel and Linux OpenSSL), cookies, HSTS, CSP, and SRI (`sha256-` and `sha3-256-`). On Linux, OpenSSL offers the `X25519MLKEM768` post-quantum hybrid first, and the group each connection really used is shown on `anvil://security`. Page loads still open one connection per request and send one HTTP/2 stream on it. `anvil-net` also has an in-house TLS 1.3 key schedule and record layer built on `anvil-pqc` + `anvil-cipher` (`Tls13Stream`, `Tls13ClientHandshaker`, `Tls13ServerHandshaker`), plus a multi-stream `H2ClientSession` and an origin-keyed connection pool, but page loads don't use those yet. The TLS 1.3 code checks itself once, before the first HTTPS connection
- Tabs that each run in their own worker process, multi-core parallel subresource loading, plus bookmarks, history, downloads and find
- Privacy controls: HTTPS-only mode, cookie partitioning, and per-site permissions
- Images (including spec-compliant lossless `VP8L` WebP encoding and decoding), fonts (`glyf`, `CFF `, `kern`, `GPOS`), and small MP4 clips
- A built-in map page

## How it's laid out

This repo is the browser shell: networking, the sandbox, the window and tabs, and the built-in pages. The page engine lives in [anvil-engine](https://github.com/buntatoes/anvil-engine), and the browser builds with all eleven companion libraries, all MPL-2.0:

- [anvil-cipher](https://github.com/buntatoes/anvil-cipher), [anvil-compress](https://github.com/buntatoes/anvil-compress), [anvil-css](https://github.com/buntatoes/anvil-css), [anvil-hpack](https://github.com/buntatoes/anvil-hpack), [anvil-image](https://github.com/buntatoes/anvil-image), [anvil-js](https://github.com/buntatoes/anvil-js), [anvil-json](https://github.com/buntatoes/anvil-json), [anvil-pqc](https://github.com/buntatoes/anvil-pqc), [anvil-ttf](https://github.com/buntatoes/anvil-ttf), [anvil-gpu](https://github.com/buntatoes/anvil-gpu), [anvil-video](https://github.com/buntatoes/anvil-video)

## Security

Report problems privately, not in public issues. See [SECURITY.md](./SECURITY.md).

## License

This repo is Apache-2.0 ([LICENSE](./LICENSE) / [LICENSE-APACHE](./LICENSE-APACHE)) and the eleven libraries are MPL-2.0 ([LICENSE-MPL](./LICENSE-MPL)). The engine's source is public to read, audit and use, but it's proprietary ([LICENSE-PROPRIETARY](./LICENSE-PROPRIETARY)) and not open source, so it can't be redistributed on its own. Builds of Anvil Browser with the engine compiled in can be shared as long as the engine's LICENSE goes with them. Everything else can be redistributed without the engine. See [NOTICE](./NOTICE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
