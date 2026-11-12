<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.28-ff6b24?style=flat-square" alt="v.28" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <a href="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml"><img src="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="Apache-2.0" /></a>
  </p>
</div>

Anvil is a small desktop browser for Windows and Linux that I'm building mostly from scratch in Rust, Go, C++20, and TypeScript. It runs on its own engines: **Forge** (HTML and layout), **Prism** (CSS), and **Cinder** (JavaScript and WebAssembly). Ad blocking is built on the open-source `adblock-rust` crate.

It's an experimental proof of concept, so expect rough edges. Version `0.28.0` (`v.28`); see [CHANGELOG.md](./CHANGELOG.md).

This daily cut is `v.28` (`0.28.0`); `weekly` and `stable` are on `v.27` (`0.27.0`). Forms capture a submission snapshot when submitted, stream `POST` and `multipart/form-data` bodies in 16 KiB chunks with live byte/percentage upload progress on the status pill, close open `<dialog>` ancestors on `method="dialog"` / `formmethod="dialog"` (skipping constraint validation, setting `returnValue`, and firing `close` without navigating), keep file inputs empty unless a picker selection is pinned for that document and control, check `form-action` before cookies on every redirect hop, and refuse `javascript:`, `data:`, unknown schemes, and embedded C0 controls. Script loading separates classic, `async`, `defer`, `<script type="importmap">`, and `type="module"` slots with executable top-level `await`, dynamic `import()` (sharing canonical module namespace objects and enforcing `script-src` CSP; import attributes remain unsupported), a 32-re-entry ceiling on native builtin callbacks, one-time page suspension tokens, and a 5-second provisional per-task wall-clock deadline alongside the 200,000-step cap. Incremental layout caches pre-stretch subtrees and intrinsic widths across clean block, Flexbox, and CSS Grid siblings in both `anvil-layout` and Forge. Reader Mode starts from the article when there is one, `Ctrl+P` saves the page as a PDF of what the window draws, saved logins live in their own sealed file, local scripts run from a `userscripts` folder next to the profile, a bad certificate gets its own page, and `anvil://settings/exchange` moves bookmarks and a few settings between installs.

## Try it

Prebuilt Windows and Linux 64-bit builds are on the [releases page](https://github.com/buntatoes/anvil-browser/releases).

## Build it

You need Rust 1.99.1+, Go 1.22+, and a C++20 compiler (plus `libx11-dev` and `libssl-dev` on Linux). Clone this repo and [`adblock-rust`](https://github.com/brave/adblock-rust) side by side in the same folder, then run:

```sh
cargo test -- --test-threads=1
cargo run --release -p anvil-browser --bin anvil
```

## What's where

```
anvil-browser/
├── crates/              Rust crates
│   ├── anvil-browser/   the app: window, pages, downloads, profile
│   ├── anvil-dom/       HTML tree, selectors, form fields
│   ├── anvil-style/     CSS cascade and media queries
│   ├── anvil-layout/    block, inline, flex, grid, table boxes
│   ├── anvil-paint/     display list and zoom
│   ├── anvil-raster/    RGBA framebuffer
│   ├── anvil-gpu/       compositor passes (CPU, OpenGL, Direct3D 11)
│   ├── anvil-script/    script interpreter and x86-64 compiler
│   ├── anvil-net/       HTTP/1.1, HTTP/2, TLS, ad blocking
│   ├── anvil-cipher/    AES, SHA-256, HMAC, HKDF
│   ├── anvil-image/     image codecs and resampling
│   ├── anvil-ttf/       TrueType/OpenType parser and rasterizer
│   ├── anvil-video/     video, audio, and containers
│   └── anvil-sandbox/   process limits for the window and tab worker
├── engines/
│   ├── prism/           CSS engine
│   ├── cinder/          JavaScript and WebAssembly engine
│   └── go/              Forge and the Go libraries, built and embedded
├── ui/                  C++20 window code
├── frontend/            TypeScript pages and styles for the built-in pages
├── examples/            sample pages
├── assets/              the Anvil emblem
├── Cargo.toml           workspace
├── CHANGELOG.md
├── SECURITY.md
├── LICENSE
├── NOTICE
└── THIRD_PARTY_NOTICES.md
```

The details live in each crate's doc comments (`cargo doc --open`).

## A note on media

Video and audio use my own formats (`ADCT`, `AVID`, and simple PCM, ADPCM, FLAC-style, and MDCT audio) inside MP4 and WebM. The VP8, VP9, AV1, and Opus code only handles those formats' headers and framing around my own payloads (for AV1, plus a CDEF-style filter), so real-world VP8, VP9, AV1, Opus, H.264, and AAC files don't play yet.

## Security

Please report security issues privately through [SECURITY.md](./SECURITY.md), not in public issues.

## License

My code is under the **Apache License 2.0**, Copyright (c) 2026 Buntato. "Anvil", "Anvil Browser", "Anvil Forge" ("Forge"), "Cinder", and "Prism" are my trademarks, and the Anvil emblem (`assets/anvil-mark.png` and icons made from it) is all rights reserved. See [LICENSE](./LICENSE).

Search engine names in settings are trademarks of their owners, used only to say which engine you're picking. Outside code that ships with Anvil (`adblock-rust` and its crates, and the Go and Rust standard libraries) keeps its own license; see [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md) and [NOTICE](./NOTICE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
