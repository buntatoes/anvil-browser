<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil</h1>
  <p>
    <img src="https://img.shields.io/badge/release-v.24.3-ff6b24?style=flat-square" alt="v.24.3" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <a href="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml?query=branch%3Atesting"><img src="https://github.com/buntatoes/anvil-browser/actions/workflows/ci.yml/badge.svg?branch=testing" alt="CI on testing" /></a>
    <img src="https://img.shields.io/badge/total_LOC-308%2C116-ff6b24?style=flat-square" alt="Total LOC: 308,116" />
    <img src="https://img.shields.io/badge/shields-Brave_adblock--rust-ff6b24?style=flat-square" alt="Brave adblock-rust" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security policy" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-PolyForm_Noncommercial_1.0.0_%7C_Paid_Commercial_%28Non--Compete%29-blue?style=flat-square" alt="PolyForm Noncommercial 1.0.0 | Paid Commercial (Non-Compete)" /></a>
  </p>
</div>

Anvil is a small desktop browser built in Rust, C++20 (native Direct2D + DirectWrite + Win32/DWM desktop UI shell in `ui/` and SIMD/VM helpers in `engines/`), JavaScript/TypeScript (`frontend/` and self-hosted `engines/anvil-js`), and CSS (`engines/anvil-css/stylesheets/` and `frontend/styles/`), and integrates Brave Browser's open-source `adblock-rust` crate (`../adblock-rust`, MPL-2.0) for network and cosmetic ad/tracker blocking. It runs on Linux and Windows.

It's a proof of concept, so expect rough edges. Current version: 0.24.3 (`v.24.3`).

See [CHANGELOG.md](./CHANGELOG.md) for what changed in each release.

## Try it

Windows 64-bit binaries (`AnvilBrowser-Setup-v.24.3-win64.exe` and portable `anvil.exe` in `dist/`) and the latest `main` release binaries are on the [releases page](https://github.com/buntatoes/anvil-browser/releases), or build it from source below.

## Build it

You need Rust 1.99.1 or newer. On Linux you also need the X11 and OpenSSL development files (`sudo apt install libx11-dev libssl-dev` on Debian or Ubuntu). A C++20 compiler builds the native Direct2D/DirectWrite desktop UI engine (`ui/`) on Windows and the C++20 CSS/JS engine helpers (`engines/anvil-css/cpp` and `engines/anvil-js/cpp`).

Clone this repo, [anvil-engine](https://github.com/buntatoes/anvil-engine), [adblock-rust](https://github.com/brave/adblock-rust), and the nine companion libraries below side by side in one folder, then run this from `anvil-browser`:

```sh
cargo test -- --test-threads=1
cargo run --release -p anvil-browser --bin anvil
```

## What it does

- **HTML, CSS, and layout**: Parses HTML with WHATWG HTML5 error recovery (active formatting element reconstruction across misnested inline tags, table foster parenting, implicit heading/paragraph/list/table auto-closure, and foreign `<svg>`/`<math>` self-closing tags), inline `<svg>` icons and shapes, tables (`colspan`, `rowspan`, and intrinsic column sizing), `<details>`/`<summary>` accordions, and form controls (`<input>`, `<textarea>`, `<select>`, checkboxes, radios, range sliders, `<progress>`, and `<meter>`). Applies the CSS cascade (the in-tree `engines/anvil-css` engine and native C++20 CSS kernels linked directly into `anvil-style`: Selectors Level 4 with `:scope`, `::before` and `::after` generated content pseudo-elements with `content: "..." | attr(...)`, interactive/validation states `:hover`/`:active`/`:focus`/`:focus-visible`/`:focus-within`/`:target`/`:valid`/`:invalid`/`:in-range`/`:out-of-range`, CSS Nesting (`&`), `@media`, `@supports` with selector/declaration fault recovery, `@layer` cascade layers with `!important` layer inversion and `revert-layer`, `@scope (<root>) to (<limit>)` scoped cascade boundaries with ancestor hop proximity, `@property` custom property registration, CSS-wide keywords `initial`/`inherit`/`unset`/`revert`/`revert-layer` and `all`, `@container` size queries, `@starting-style`, CSS Color 4/5 `color(display-p3 | xyz-d50 | xyz-d65)`, `color-mix()`, `light-dark()`, `linear-gradient()` and `radial-gradient()`, CSS Values 4 math `calc()`/`min()`/`max()`/`clamp()`/`round()`/`mod()`/`rem()`/trig/exponential and universal viewport/font units across box edges, `text-overflow: ellipsis`, `overflow-wrap: break-word` / `word-break: break-all`, CSS Logical Properties, standalone `translate`/`scale`/`rotate`, custom properties with recursive `var(--missing, fallback)` recovery, `box-sizing`, `aspect-ratio`, complete Flexbox and CSS Grid, transitions, and keyframe animations) and draws through the CPU and `anvil-gpu` batched scene passes (`GpuSceneBatch`) with gamma-correct linear-space glyph blending, 4-bin (`0.25 px`) subpixel phase positioning, and pairwise kerning (`anvil-ttf` 0.9.0) and bounded glyph and image caches.
- **JavaScript and WebAssembly**: Runs page scripts and WebAssembly 1.0 modules under a step and memory budget with per-`<script>` and statement-level fault isolation (`window.onerror`, `ErrorEvent`, `PromiseRejectionEvent`, `unhandledrejection`). `anvil-script` links the in-tree `engines/anvil-js` engine directly (a register bytecode VM with mark-sweep GC, hidden classes, 4-way polymorphic inline caches, ES2022–ES2024 classes/`#private` fields/destructuring/regex/`async`-`await`, Generators `function*`/`yield`/`yield*`, `BigInt`, ES module `import`/`export`/`import.meta`/`import()` lowering, `Proxy` and `Reflect`, `WeakMap`/`WeakSet`/`WeakRef`/`FinalizationRegistry`, `DataView`/`ArrayBuffer`/`TypedArray`s, `Symbol.for`/`Symbol.keyFor`, `Error(..., { cause })`, `AggregateError`, `Intl`, and live `AnvilJsDomHost` W3C 3-phase DOM event propagation `Capture -> Target -> Bubble` with `once`/`capture`/`passive`/`signal`/`stopImmediatePropagation`/`composedPath`, `Element`/`ChildNode`/`ParentNode` mutation methods, `TreeWalker`/`NodeIterator`/`Range`/`Selection`, `fetch`/`Headers`/`Request`/`Response`/`AbortController`, `history.pushState`/`replaceState`/`popstate`, `crypto.getRandomValues`/`randomUUID`/`subtle.digest`, `performance.mark`/`measure`, and 2D `<canvas>` transforms/bezier curves/gradients/`getImageData`/`putImageData` via `eval_with_anvil_js_engine`, alongside the self-hosted ES2022–ES2024 compiler and DOM runtime on the C++20 `AnvilVM`).
- **Networking and TLS**: Speaks HTTP/1.1 and HTTP/2 (over ALPN `h2` on Windows Schannel and Linux OpenSSL) with origin-keyed connection pooling, cookies, HSTS, CSP, and Subresource Integrity (`sha256-` and `sha3-256-`). On Linux, OpenSSL offers the `X25519MLKEM768` post-quantum hybrid group first and shows the negotiated group on `anvil://security`. `anvil-net` also includes an in-house TLS 1.3 implementation built on `anvil-pqc` and `anvil-cipher`; page loads still connect through OpenSSL or Schannel after the TLS 1.3 self-test passes.
- **Process isolation, Brave Shields, and privacy**: Runs each tab in its own `--renderer` process (with optional Linux `bwrap`/`unshare`/`prlimit` confinement, Windows job limits and low-integrity labels, and a checked painted-frame draw mode on `anvil://security`). Links Brave Browser's unmodified `adblock-rust` crate (`adblock::Engine`, MPL-2.0) for ABP/uBlock Origin network filter matching, `@@` exceptions, `$important`, `$redirect` script/frame surrogates, `$removeparam` tracking query parameter stripping, cosmetic element hiding (`##` and `#@#`), and per-site Shields Up/Down controls on `anvil://shields` (`anvil://security`). Supports HTTPS-only mode, top-level site cookie partitioning, and per-site script and cookie permissions.
- **Native C++20 desktop UI, tabs, and tools**: Renders the browser shell chrome through a native C++20 Direct2D + DirectWrite + Win32/DWM UI engine (`ui/anvil_core_abi.h`, `ui/anvil_ui_d2d.cpp`, `ui/anvil_ui_window.cpp`) with ClearType proportional typography (`Segoe UI`, `Segoe UI Semibold`, `Cascadia Mono`), an integrated 38px top tab strip with curved tabs, draggable titlebar region, and Windows 11 Minimize/Maximize/Close caption buttons, a pill-shaped Omnibox with domain-highlighted URL (`split_url_for_domain_highlight`), inline Security pill, inline Shields pill, and floating suggestion popover, a floating bottom-left link-hover/loading status pill, and 2-column left sidebar navigation + card layouts across `anvil://home`, `anvil://settings`, `anvil://security`, `anvil://downloads`, `anvil://history`, and `anvil://inspector`. Supports pinned tabs, tab reordering (`Ctrl+Shift+PageUp`/`PageDown`), closed-tab restore (`Ctrl+Shift+T`), duplicate/close-other-tabs, four themes (`Dark`, `Midnight`, `Light`, `Nordic`), mouse-drag page text selection, full caret and word-level text editing, a bottom `F12` DevTools dock with live DOM/style inspection and a JS console, `view-source:` (`Ctrl+U`), `Ctrl+S` page saving, automatic file downloads (`anvil://downloads`), find in page (`Ctrl+F`), and page zoom (`75%`–`150%`).
- **Media, maps, and search**: Decodes PNG, JPEG, GIF, WebP (`VP8L` and `VP8 `), BMP, ICO, and TrueType/OpenType fonts (`glyf`, `CFF `, `kern`, `GPOS`). Plays small MP4/WebM clips encoded with `anvil-video`'s in-house `AVID`/`ADCT` codecs (it is not a general H.264/AV1/VP9 stream decoder). Includes a built-in tile map (`anvil://maps`) and clean result formatting for Google, DuckDuckGo, and Bing search pages.

## How it's laid out

This repo houses the browser shell (`crates/`), the native C++20 Direct2D/DirectWrite desktop UI engine (`ui/`), the TypeScript/TSX built-in pages (`frontend/`), and the in-tree `Anvil-CSS` (`engines/anvil-css`) and `Anvil-JS` (`engines/anvil-js`) engines that link directly into the Rust core (`anvil-style` and `anvil-script`). The page engine lives in [anvil-engine](https://github.com/buntatoes/anvil-engine), Shields adblocking links Brave Browser's unmodified [adblock-rust](https://github.com/brave/adblock-rust) crate (`MPL-2.0`), and the browser builds with all nine standalone companion libraries:

- [anvil-cipher](https://github.com/buntatoes/anvil-cipher), [anvil-compress](https://github.com/buntatoes/anvil-compress), [anvil-hpack](https://github.com/buntatoes/anvil-hpack), [anvil-image](https://github.com/buntatoes/anvil-image), [anvil-json](https://github.com/buntatoes/anvil-json), [anvil-pqc](https://github.com/buntatoes/anvil-pqc), [anvil-ttf](https://github.com/buntatoes/anvil-ttf), [anvil-gpu](https://github.com/buntatoes/anvil-gpu), [anvil-video](https://github.com/buntatoes/anvil-video)

## Security

Report problems privately, not in public issues. See [SECURITY.md](./SECURITY.md).

## License

All original Anvil Browser code (including the in-tree `ui/`, `engines/anvil-css`, and `engines/anvil-js` engines), `anvil-engine`, and the nine `anvil-*` companion libraries are dual-licensed under the **PolyForm Noncommercial License 1.0.0** (free for personal, educational, academic, and noncommercial auditing/research use) and a **Paid Commercial License with a Non-Compete Clause** (required for any commercial, for-profit, or business use). Competing browser, rendering engine, JS VM, or codec use is prohibited without a signed commercial agreement from Buntato. The linked unmodified [`adblock-rust`](https://github.com/brave/adblock-rust) component (`adblock` crate) is licensed under the **Mozilla Public License 2.0 (`MPL-2.0`)**. See [LICENSE](./LICENSE).

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
