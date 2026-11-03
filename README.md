<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.26-ff6b24?style=flat-square" alt="v.26" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <a href="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml"><img src="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml/badge.svg" alt="CI" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="Apache-2.0" /></a>
  </p>
</div>

Anvil is a small desktop browser for Windows and Linux that I'm building mostly from scratch in Rust, Go, C++20, and TypeScript. It runs on its own engines: **Forge** (HTML and layout), **Prism** (CSS), and **Cinder** (JavaScript and WebAssembly). Ad blocking is built on the open-source `adblock-rust` crate.

It's an experimental proof of concept, so expect rough edges. Version `0.26.0` (`v.26`); see [CHANGELOG.md](./CHANGELOG.md).

## Try it

Prebuilt Windows and Linux 64-bit builds are on the [releases page](https://github.com/buntatoes/anvil-browser/releases).

## Build it

You need Rust 1.99.1+, Go 1.22+, and a C++20 compiler (plus `libx11-dev` and `libssl-dev` on Linux). Clone this repo and [`adblock-rust`](https://github.com/brave/adblock-rust) side by side in the same folder, then run:

```sh
cargo test -- --test-threads=1
cargo run --release -p anvil-browser --bin anvil
```

## What's where

- `crates/`: the Rust crates. `anvil-browser` is the app.
- `engines/`: Prism (CSS), Cinder (JavaScript), and the Go side (Forge and the libraries), which the build compiles and embeds.
- `ui/` and `frontend/`: the C++20 window and the built-in pages.

The details live in each crate's doc comments (`cargo doc --open`).

## A note on media

Video and audio use my own formats (`ADCT`, `AVID`, and simple PCM, ADPCM, FLAC-style, and MDCT audio) inside MP4 and WebM. The VP8, VP9, AV1, and Opus code only handles those formats' headers and framing around my own payloads (for AV1, plus a CDEF-style filter), so real-world VP8, VP9, AV1, Opus, H.264, and AAC files don't play yet.

## Security

Please report security issues privately through [SECURITY.md](./SECURITY.md), not in public issues.

## License

My code is under the **Apache License 2.0**, Copyright (c) 2026 Buntato. "Anvil", "Anvil Browser", "Anvil Forge" ("Forge"), "Cinder", and "Prism" are my trademarks, and the Anvil emblem (`assets/anvil-mark.png` and icons made from it) is all rights reserved. See [LICENSE](./LICENSE).

Search engine names in settings are trademarks of their owners, used only to say which engine you're picking. Outside code that ships with Anvil (`adblock-rust` and its crates, and the Go and Rust standard libraries) keeps its own license; see [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md) and [NOTICE](./NOTICE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
