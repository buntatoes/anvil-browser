<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.23.3.2-ff6b24?style=flat-square" alt="v.23.3.2" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <img src="https://img.shields.io/badge/total_LOC-207%2C852-ff6b24?style=flat-square" alt="Total LOC: 207,852" />
    <img src="https://img.shields.io/badge/external_crates-0_%28offline_verified%29-2ea44f?style=flat-square" alt="0 external crates (offline verified)" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security policy" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-PolyForm_Noncommercial_1.0.0_%7C_Paid_Commercial_%28Non--Compete%29-blue?style=flat-square" alt="PolyForm Noncommercial 1.0.0 | Paid Commercial (Non-Compete)" /></a>
  </p>
</div>

Anvil is a small desktop browser built from scratch in Rust, with a bit of C++20 and JavaScript. It has no outside crates: every dependency lives in one of the Anvil repos. It runs on Linux and Windows.

It's a proof of concept, so expect rough edges. Current version: 23.3.2 (`v.23.3.2`).

See [CHANGELOG.md](./CHANGELOG.md) for what changed in each release.

## Try it

Windows 64-bit binaries (`AnvilBrowser-Setup-v.23.3.2-win64.exe` and portable `anvil.exe` in `dist/`) and the latest `main` release binaries are on the [releases page](https://github.com/buntatoes/anvil-browser/releases), or build it from source below.

## Build it

You need Rust 1.99.1 or newer. On Linux you also need the X11 and OpenSSL development files (`sudo apt install libx11-dev libssl-dev` on Debian or Ubuntu). A C++20 compiler is optional.

Clone this repo, [anvil-engine](https://github.com/buntatoes/anvil-engine), and the eleven libraries below side by side in one folder, then run this from `anvil-browser`:

```sh
cargo test -- --test-threads=1
cargo run --release -p anvil-browser --bin anvil
```

## What it does

- **HTML, CSS, and layout**: Parses HTML, inline `<svg>` icons and shapes, tables (`colspan`), `<details>`/`<summary>` accordions, and form controls (`<input>`, `<textarea>`, `<select>`, checkboxes, radios, range sliders, `<progress>`, and `<meter>`). Applies CSS in document order (`anvil-css` 0.3.0 Selectors Level 4, `@media`, `@supports`, CSS Color 4, custom properties, `calc()`, `box-sizing`, `aspect-ratio`, flex, grid, transitions, and keyframe animations) and draws through the CPU and `anvil-gpu` batched scene passes (`GpuSceneBatch`) with bounded glyph and image caches.
- **JavaScript and WebAssembly**: Runs page scripts and WebAssembly 1.0 modules under a step and memory budget. `anvil-script` pairs a Rust interpreter and WebAssembly stack VM with `anvil-js` 0.5.0 (an ES2022+ compiler and DOM runtime on `AnvilVM`) to handle DOM queries and mutations, `click`/`input`/`submit`/`keydown` events, `localStorage`, 2D `<canvas>`, observers (`MutationObserver`, `IntersectionObserver`, `ResizeObserver`), and `fetch()`.
- **Networking and TLS**: Speaks HTTP/1.1 and HTTP/2 (over ALPN `h2` on Windows Schannel and Linux OpenSSL) with origin-keyed connection pooling, cookies, HSTS, CSP, and Subresource Integrity (`sha256-` and `sha3-256-`). On Linux, OpenSSL offers the `X25519MLKEM768` post-quantum hybrid group first and shows the negotiated group on `anvil://security`. `anvil-net` also includes an in-house TLS 1.3 implementation built on `anvil-pqc` and `anvil-cipher`; page loads still connect through OpenSSL or Schannel after the TLS 1.3 self-test passes.
- **Process isolation and privacy**: Runs each tab in its own `--renderer` process (with optional Linux `bwrap`/`unshare`/`prlimit` confinement, Windows job limits and low-integrity labels, and a checked painted-frame draw mode on `anvil://security`). Supports HTTPS-only mode, top-level site cookie partitioning, and per-site script and cookie permissions.
- **Window, tabs, and tools**: Supports pinned tabs, tab reordering (`Ctrl+Shift+PageUp`/`PageDown`), closed-tab restore (`Ctrl+Shift+T`), duplicate/close-other-tabs, four themes (`Dark`, `Midnight`, `Light`, `Nordic`), a floating Omnibox suggestion popover (built-in pages, bookmarks, history, and search), mouse-drag page text selection, full caret and word-level text editing, a bottom `F12` DevTools dock with live DOM/style inspection and a JS console, `view-source:` (`Ctrl+U`), `Ctrl+S` page saving, automatic file downloads (`anvil://downloads`), find in page (`Ctrl+F`), and page zoom (`75%`–`150%`).
- **Media, maps, and search**: Decodes PNG, JPEG, GIF, WebP (`VP8L` and `VP8 `), BMP, ICO, and TrueType/OpenType fonts (`glyf`, `CFF `, `kern`, `GPOS`). Plays small MP4/WebM clips encoded with `anvil-video`'s in-house `AVID`/`ADCT` codecs (it is not a general H.264/AV1/VP9 stream decoder). Includes a built-in tile map (`anvil://maps`) and clean result formatting for Google, DuckDuckGo, and Bing search pages.

## How it's laid out

This repo is the browser shell: networking, the sandbox, the window and tabs, and the built-in pages. The page engine lives in [anvil-engine](https://github.com/buntatoes/anvil-engine), and the browser builds with all eleven companion libraries:

- [anvil-cipher](https://github.com/buntatoes/anvil-cipher), [anvil-compress](https://github.com/buntatoes/anvil-compress), [anvil-css](https://github.com/buntatoes/anvil-css), [anvil-hpack](https://github.com/buntatoes/anvil-hpack), [anvil-image](https://github.com/buntatoes/anvil-image), [anvil-js](https://github.com/buntatoes/anvil-js), [anvil-json](https://github.com/buntatoes/anvil-json), [anvil-pqc](https://github.com/buntatoes/anvil-pqc), [anvil-ttf](https://github.com/buntatoes/anvil-ttf), [anvil-gpu](https://github.com/buntatoes/anvil-gpu), [anvil-video](https://github.com/buntatoes/anvil-video)

## Security

Report problems privately, not in public issues. See [SECURITY.md](./SECURITY.md).

## License

Dual-licensed under the **PolyForm Noncommercial License 1.0.0** (free for personal, educational, academic, and noncommercial auditing/research use) and a **Paid Commercial License with a Non-Compete Clause** (required for any commercial, for-profit, or business use). Competing browser, rendering engine, JS VM, or codec use is prohibited without a signed commercial agreement from Buntato. See [LICENSE](./LICENSE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
