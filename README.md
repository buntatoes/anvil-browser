<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.22-ff6b24?style=flat-square" alt="v.22" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security policy" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/browser-Apache--2.0-blue?style=flat-square" alt="Browser: Apache-2.0" /></a>
    <a href="./LICENSE-MPL"><img src="https://img.shields.io/badge/libraries-MPL--2.0-6f42c1?style=flat-square" alt="Libraries: MPL-2.0" /></a>
    <a href="#license"><img src="https://img.shields.io/badge/engine-Proprietary-e8c37a?style=flat-square" alt="Engine: proprietary" /></a>
  </p>
</div>

Anvil is a small desktop browser built from scratch in Rust, with a bit of C++20 and JavaScript. It has no outside crates: every dependency lives in one of the Anvil repos. It runs on Linux and Windows.

It's a proof of concept, so expect rough edges. Current version: 0.22.


## Build it

You need Rust 1.85.1 or newer. On Linux you also need the X11 and OpenSSL development files (`sudo apt install libx11-dev libssl-dev` on Debian or Ubuntu). A C++20 compiler is optional.

Clone this repo, [anvil-engine](https://github.com/buntatoes/anvil-engine), and the libraries below side by side in one folder, then run this from `anvil-browser`:

```sh
cargo test
cargo run --release -p anvil-browser --bin anvil
```

## What it does

- HTML, CSS (flex, grid, tables, media queries), and layout, drawn on the CPU
- A JavaScript interpreter with a time budget
- HTTP/1.1, HTTP/2, cookies, HSTS and CSP, with a post-quantum key exchange on Linux when the server supports it
- Tabs that each run in their own worker process, plus bookmarks, history, downloads and find
- Privacy controls: HTTPS-only mode and per-site permissions
- Images, fonts, and small MP4 clips
- A built-in map page

It can't do hardware GPU rendering or GPU video decoding yet, and some CSS (named grid areas, 3D transforms) is still missing.

## How it's laid out

This repo is the browser shell: networking, the sandbox, the window and tabs, and the built-in pages. The page engine lives in [anvil-engine](https://github.com/buntatoes/anvil-engine), and the browser builds with eight of the companion libraries, all MPL-2.0:

- [anvil-cipher](https://github.com/buntatoes/anvil-cipher), [anvil-compress](https://github.com/buntatoes/anvil-compress), [anvil-hpack](https://github.com/buntatoes/anvil-hpack), [anvil-image](https://github.com/buntatoes/anvil-image), [anvil-json](https://github.com/buntatoes/anvil-json), [anvil-ttf](https://github.com/buntatoes/anvil-ttf), [anvil-gpu](https://github.com/buntatoes/anvil-gpu), [anvil-video](https://github.com/buntatoes/anvil-video)

The ninth, [anvil-pqc](https://github.com/buntatoes/anvil-pqc), is a post-quantum key exchange library the browser doesn't use yet.

## Security

Report problems privately, not in public issues. See [SECURITY.md](./SECURITY.md).

## License

This repo is Apache-2.0 ([LICENSE](./LICENSE)) and the nine libraries are MPL-2.0 ([LICENSE-MPL](./LICENSE-MPL)). The engine's source is public to read, audit and use, but it's proprietary and not open source, so it can't be redistributed on its own. Builds of Anvil Browser with the engine compiled in can be shared as long as the engine's LICENSE goes with them. Everything else can be redistributed without the engine.

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
