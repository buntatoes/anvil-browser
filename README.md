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

- HTML (including `<noscript>` inert raw-text isolation, regex-safe `<script>` comment tokenization, DOM sibling/child traversal, `getElementsByTagName`/`getElementsByClassName`/`getElementsByName`, `#fragment` anchor and `<a name>` target resolution via `find_fragment_target`, `<details>`/`<summary>` disclosure accordions and closed `<dialog>` suppression, `<input type="radio">` groups, `<label>` click forwarding, `Tab`/`Shift+Tab` form field focus cycling, mid-string caret/cursor movement and word deletion (`Left`/`Right`/`Home`/`End`/`Delete`/`Ctrl+Backspace`), floating `<select>` dropdown option popovers, interactive `<input type="range">` sliders and `<progress>`/`<meter>` bars, multi-line `<pre>`/`<textarea>` formatting, `insertAdjacentHTML`/`insertAdjacentElement`/`insertAdjacentText`, inline `<svg>` vector icon and shape rasterization, and `<canvas>` 2D RGBA backing surfaces), CSS (`anvil-css` 0.3.0 Selectors Level 4 `:is()`, `:where()`, `:not()`, `:has()`, `:host`, forgiving selector lists, vendor pseudo-element/class tolerance, CSS Color Level 4 `rgb(r g b / a)` / `hsl()` / `hwb()` / `oklab()` / `oklch()` / `color(srgb ...)`, `@supports`, `box-sizing: border-box`, `aspect-ratio`, `visibility`, `text-align`, `white-space`, `line-height`, `margin: 0 auto` block centering, `inline-block` / `inline-flex` and decorated inline (`<code>`, `<mark>`, badges, pills) row packing, flex row/column alignment, grid with `minmax()` and `auto-fill` / `auto-fit`, tables with `colspan`, media queries, transitions, keyframes), and layout, drawn through the CPU, persistent per-thread `anvil-gpu` compositor/tile devices (coalescing contiguous rects, rounded-rect box fills, opaque borders, box shadows, gradients, clips, and fixed-scroll offsets into batched `GpuSceneBatch` command buffers in `paint_span`, plus explicit `build_gpu_scene_batch` / `rasterize_gpu_batch`), and bounded LRU glyph/image caches (`GlyphLruCache` and `ImageLruCache`)
- Style sheets apply in document order: `<style>` blocks and `<link rel="stylesheet">` sheets keep their place in the page, each sheet is parsed on its own, and a sheet's `media` attribute is honoured the same way as `@media`, so a print sheet doesn't apply on screen and a broken sheet doesn't swallow the next one. The page's CSS is parsed once and only parsed again when a sheet changes, so animation frames and window resizes reuse it
- A JavaScript and WebAssembly 1.0 (`WASM`) runtime (`anvil-js` 0.5.0 self-hosted ES2022+ compiler and Web/DOM runtime executing on the C++20 `AnvilVM` register bytecode VM, with automatic ES2022+ DOM script routing in `ScriptRuntime::eval_script`, full hierarchical DOM tree seeding via `seedHostNode`/`seedHostAttr`, live DOM/storage/location mutation sync via `exportDomMutationRows` and `eval_webapp_with_live_dom_bridge`, interactive `click`/`input`/`submit`/`keydown` event replay via `dispatchHostEvent` with `preventDefault()`, microtask-queued `MutationObserver`/`IntersectionObserver`/`ResizeObserver` callbacks, parameterized and unary IIFEs, unified `window`/`self`/`globalThis`/`document`, nested object property assignment, `document.readyState`, `WasmModule`, `WasmInstance`, `WasmMemory`, `WebAssembly` JS namespace, `TypedArray` views, `ArrayBuffer`, `TextEncoder`, `TextDecoder`, `atob`/`btoa`, `URLSearchParams`, `DOMParser`, `CustomElementRegistry`, `ShadowRoot`, 2D `<canvas>` `getContext('2d')`, `MediaSource`, `VideoEncoder`, `VideoDecoder`, and `--renderer` interactive click hydration via `hydrate_interactive_bindings`) with a time/step budget
- HTTP/1.1 and HTTP/2 (over ALPN `h2` on both Windows Schannel and Linux OpenSSL), origin-keyed connection pooling (`connection_pool_stats`) for HTTP/1.1 keep-alive sockets (`Content-Length` and `Transfer-Encoding: chunked`) and persistent multi-stream `H2ClientSession` instances (reusing odd stream IDs `1, 3, 5, ...` and multiplexing same-origin `<link rel="stylesheet">` and `<img>` batches via `fetch_batch`), cookies, HSTS, CSP, and SRI (`sha256-` and `sha3-256-`). On Linux, OpenSSL offers the `X25519MLKEM768` post-quantum hybrid first, and the group each connection really used is shown on `anvil://security`. `anvil-net` also has an in-house TLS 1.3 key schedule and record layer built on `anvil-pqc` + `anvil-cipher` (`Tls13Stream`, `Tls13ClientHandshaker`, `Tls13ServerHandshaker`); page loads still connect through OpenSSL or Schannel, and the TLS 1.3 code checks itself once before the first HTTPS connection
- Tabs that each run in their own `--renderer` process (with optional Linux `bwrap`/`unshare`/`prlimit` confinement and a verified painted-frame draw mode on `anvil://security`), multi-core parallel subresource loading, cached page framebuffer compositing (`compose_frame_with_page_fb` + `Framebuffer::copy_rows_from`), four built-in themes (`BrowserTheme`: `Dark`, `Midnight`, `Light`, `Nordic`), real-time scrollbar thumb dragging, mouse-drag page text selection with visual selection highlights (`select_text_range`, `selection_rects`), middle-click tab close and link opening, pinned tabs (`toggle_pin_tab` with `[P]` badge and close protection), tab reordering (`move_active_tab_left`/`right`, `Ctrl+Shift+PageUp`/`PageDown`), `Close Other Tabs` (`close_other_tabs`), `Ctrl+Shift+T` closed-tab restoration (`reopen_closed_tab`), tab duplication (`duplicate_tab`), `Ctrl+Tab`/`Ctrl+Shift+Tab`/`Ctrl+1..9` tab switching, context-aware link right-click menu (`Open Link in New Tab`, `Copy Link Address`), live interactive `F12` DevTools Inspector dock (`inspect_at`, `inspected_element_summary`, `eval_console_js` Console REPL), `view-source:` (`Ctrl+U`) source inspector, `Ctrl+S` page saving and automatic `Content-Disposition: attachment` / binary MIME download capture (`is_downloadable_response`) with disk export on `anvil://downloads`, floating Omnibox autocomplete & smart suggestion popover (`omnibox_suggestions`, `Up`/`Down`/`Tab`/`Enter`/click across built-in routes, bookmarks, history, and search), Omnibox clear button and `Ctrl+A` select-all, plus bookmarks, history, downloads and find
- Privacy controls: HTTPS-only mode, cookie partitioning, and per-site permissions
- Images (including lossless `VP8L` and lossy `VP8 ` WebP), fonts (`glyf`, `CFF `, `kern`, `GPOS`), and small MP4/WebM clips (`anvil-video` 0.8.0 adds an in-house `ADCT` 8x8 integer DCT/IDCT + motion-compensated codec, `FLAC`/MDCT audio, and H.264/AV1/VP9/fMP4/WebM framing in the library; it is not a full H.264/AV1/VP9 bitstream decoder)
- A built-in map page, in-page `#fragment` anchor scrolling (`scroll_to_fragment`, with sticky/fixed top header offset), per-page back/forward/tab scroll restoration, and Google Search / DuckDuckGo Search / Bing Search results normalization (`sites::prepare_document`)

## How it's laid out

This repo is the browser shell: networking, the sandbox, the window and tabs, and the built-in pages. The page engine lives in [anvil-engine](https://github.com/buntatoes/anvil-engine), and the browser builds with all eleven companion libraries:

- [anvil-cipher](https://github.com/buntatoes/anvil-cipher), [anvil-compress](https://github.com/buntatoes/anvil-compress), [anvil-css](https://github.com/buntatoes/anvil-css), [anvil-hpack](https://github.com/buntatoes/anvil-hpack), [anvil-image](https://github.com/buntatoes/anvil-image), [anvil-js](https://github.com/buntatoes/anvil-js), [anvil-json](https://github.com/buntatoes/anvil-json), [anvil-pqc](https://github.com/buntatoes/anvil-pqc), [anvil-ttf](https://github.com/buntatoes/anvil-ttf), [anvil-gpu](https://github.com/buntatoes/anvil-gpu), [anvil-video](https://github.com/buntatoes/anvil-video)

## Security

Report problems privately, not in public issues. See [SECURITY.md](./SECURITY.md).

## License

Dual-licensed under the **PolyForm Noncommercial License 1.0.0** (free for personal, educational, academic, and noncommercial auditing/research use) and a **Paid Commercial License with a Non-Compete Clause** (required for any commercial, for-profit, or business use). Competing browser, rendering engine, JS VM, or codec use is prohibited without a signed commercial agreement from Buntato. See [LICENSE](./LICENSE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
