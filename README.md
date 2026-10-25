<div align="center">
  <img src="./assets/anvil-mark.png" width="128" height="128" alt="Anvil" />
  <h1>Anvil Browser</h1>
  <p>A small desktop browser proof of concept. Rust, C++20, and JavaScript. No crates.io dependencies.</p>
  <p>
    <img src="https://img.shields.io/badge/release-v.22-ff6b24?style=flat-square" alt="v.22" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0-blue?style=flat-square" alt="Apache-2.0" /></a>
    <a href="./LICENSE-MPL"><img src="https://img.shields.io/badge/license-MPL--2.0-6f42c1?style=flat-square" alt="MPL-2.0" /></a>
    <a href="#license"><img src="https://img.shields.io/badge/engine-Proprietary-e8c37a?style=flat-square" alt="Engine proprietary" /></a>
  </p>
</div>

Version 0.22. Author: Buntos ([@buntatoes](https://github.com/buntatoes)).

This repository is the window, the network stack, and the profile file. HTML, CSS, layout, paint, raster, and script are in `anvil-engine`.

## License

| What | License |
| --- | --- |
| This repository (`anvil-net`, `anvil-sandbox`, `anvil-browser`, `frontend/`, `assets/anvil-mark.png`) | Apache-2.0. [`LICENSE`](./LICENSE) |
| `anvil-engine` (`anvil-dom`, `anvil-style`, `anvil-layout`, `anvil-paint`, `anvil-raster`, `anvil-script`) | Proprietary. All rights reserved |
| `anvil-cipher`, `anvil-compress`, `anvil-image`, `anvil-json`, `anvil-ttf`, `anvil-gpu`, `anvil-video` | MPL-2.0. [`LICENSE-MPL`](./LICENSE-MPL) |

## Download

No v.22 Windows binary is attached yet. The last packaged build is the [v.21 release](https://github.com/buntatoes/anvil-browser/releases/tag/v.21). SHA-256 checksums are in the release notes.

| File | What it is |
| --- | --- |
| `AnvilBrowser-Setup-v.21-win64.exe` | Setup program. Uninstall with `--uninstall` |
| `anvil.exe` | The browser, no installer |

## What it does

- HTML is one tree. Descendant, child, compound, comma, and attribute selectors. DOM mutation, forms, images, and video. An image can use `src`, `srcset`, or a lazy `data-src`. Text in `pre` and `textarea` keeps its line breaks.
- CSS cascade, media queries, custom properties, and `calc()`. `width`, `height`, `max-width`, and offsets keep `px`, `%`, `em`, `rem`, `vw`, `vh`, `vmin`, and `vmax`. Other units become `auto`. `font-size` keeps `px`, `%`, `em`, and `rem`. `%` and `em` use the parent font size. `rem` uses the `html` font size, 16px until `html` sets one. Margin, padding, border, and `gap` are `px` or `0`.
- `display: inline-block` is laid out as a block. An element with the `hidden` attribute stays hidden unless its own style shows it.
- A percentage width uses the containing block width. A percentage height uses the containing block height when that height is set, otherwise `auto`.
- Layout is block, inline, flex, grid (`px`, `%`, `fr`, `repeat()`), table, and positioned boxes. `flex-wrap: wrap` starts another line. `flex-grow` shares positive free space and does not shrink. Column flex does not wrap or grow.
- Paint is a CPU RGBA buffer. `rasterize_parallel` uses 64px strips. `rasterize_incremental` skips unchanged 64 by 64 tiles. Shadows, rounded boxes, and gradients use `anvil-gpu`'s software device. Optional C++20 runs when `cl.exe`, `g++`, or `clang++` is on `PATH`.
- Script is a budgeted interpreter: `try / catch / finally`, `for...of`, `for...in`, `switch`, `class` / `extends` / `super`, destructuring, spread, `??`, `?.`, and `Array` / `Object` methods. A numeric `while` or `for` can compile to x86-64, including `document.getElementById("...").textContent` set to a number. A script with no DOM, `fetch()`, or storage can take that path. Anything else stays interpreted. Not a general compiler.
- `fetch()` is same-origin. A cross-origin URL returns 403 and does not connect. The body cap is 64 KiB. The numeric VM does not call `fetch()`.
- A script runs when the code outside its quotes is at most 8 KiB. A longer bundle is left unrun. A `script` whose type is not JavaScript is not run. External script files on the hosts below are not downloaded.
- YouTube, Gmail, Facebook, Instagram, X, Outlook, TikTok, LinkedIn, and the other large sites below load as the sites themselves. The window sends the session cookie jar on that navigation and stores `Set-Cookie`. A named submit button is included in the form body. Google still answers that sign-in with its own "JavaScript is turned off" page. YouTube's player bundle is not run. A watch page keeps YouTube's document and adds a player from the JPEG frames YouTube publishes (`hqdefault`, `hq1`, `hq2`, `hq3`), or from a storyboard sheet when the page includes one. That is not the H.264 stream. Reddit opens the address you asked for, including `www.reddit.com`, and is not sent to `old.reddit.com`. Those requests use a longer `User-Agent` that names this browser. A network-policy block is Reddit's answer. Discord, Snapchat, Pinterest, Tumblr, Twitch, Threads, Bluesky, WhatsApp, Telegram, and the other social hosts in the engine load the same way. Bank and payment hosts, including Chase, Bank of America, Wells Fargo, Citi, Capital One, PayPal, and Venmo, load their own pages. Their script applications are not run. This program stores no password and no API key.
- Google Maps, OpenStreetMap, and `anvil://maps` are drawn in this process from nine OpenStreetMap tiles, fetched together. The Google Maps script is not downloaded. A place name is looked up once. Pan and zoom are links. No maps API key is stored.
- `localStorage` and `sessionStorage` are per origin, 64 KiB of key and value bytes. A write past the cap keeps the old value. `sessionStorage` dies with the tab.
- HTTP/1.1 GET and POST, TLS, cookies, HSTS, CSP, CORP, and SHA-256 subresource checks are in `anvil-net`. A short host list is blocked before connect.
- An http or https load starts one `--tab-worker` and waits 5 seconds for one `ANVILIPC` frame. The navigation request uses the session cookie jar. The worker does not get that jar or `localStorage`. A timeout or a bad frame is an error page. `anvil://` and local files stay in the window process. This is not a multi-process sandbox.

Built-in pages: `anvil://home`, `anvil://settings`, `anvil://security`, `anvil://history`, `anvil://downloads`, `anvil://inspect`. `frontend/` is text and the scripts in `frontend/scripts/`. No Node build.

Not implemented: hardware-GPU raster, a GPU video decoder, `ch`, grid row tracks, multi-column grid spans, and `calc()` that mixes viewport units. Margin and padding are still `px` only. Direct3D 11 only presents the finished CPU buffer. If that device or `compositor.hlsl` fails, Windows uses GDI. Video frames are decoded on the CPU. Demo clips may carry PCM or ADPCM. Encrypted samples are AES-128-CTR from `anvil-cipher`. YouTube's player bundle, Gmail after the HTML sign-in form, the Facebook feed, the current Reddit app, and bank web apps are not run. A watch page can still show YouTube's published preview frames.

Enforced limits: [SECURITY.md](./SECURITY.md).

## Window

Linux draws with X11 and uses OpenSSL (`libssl` / `libcrypto`). The window sets `PR_SET_NO_NEW_PRIVS`, turns core dumps off, and tries a 2 GiB address-space cap. `--tab-worker` asks for 256 MiB and exits if that cap does not stick.

Windows uses the system window APIs, Schannel, and DPAPI.

`profile.anvil` stores bookmarks, history, cookies, HSTS, settings, and origin-partitioned `localStorage`. Windows seals it with DPAPI. Linux seals it with AES-256-GCM. The Linux key is `profile.key` in the same directory, mode 0600, written before the profile.

| Keys | Action |
| --- | --- |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+L | Address bar |
| Ctrl+R / F5 | Reload |
| Alt+Left / Alt+Right | Back / forward |
| Ctrl+F | Find |
| Ctrl+J / Ctrl+H | Downloads / history |
| F12 | Inspector on Windows |

## Source

| Path | What it is |
| --- | --- |
| `../anvil-engine` | HTML, CSS, layout, paint, raster, script. Proprietary |
| `crates/anvil-net` | HTTP, TLS, host block list |
| `crates/anvil-sandbox` | Process limits and `ANVILIPC` |
| `crates/anvil-browser` | Window, pages, downloads, `profile.anvil` |
| `frontend/` | Built-in pages |

## Build

Clone these as siblings. Rust 1.85 or newer. A C++20 compiler is optional. If none is found, the C++ files are skipped.

```text
../anvil-browser
../anvil-engine
../anvil-cipher
../anvil-compress
../anvil-image
../anvil-json
../anvil-ttf
../anvil-gpu
../anvil-video
```

```text
cargo test
cargo run --release -p anvil-browser --bin anvil
```

```text
anvil [file or http/https/anvil URL]
anvil open [--width N] [--height N] [file or http/https/anvil URL]
anvil render [--width N] <file or http/https/anvil URL>
anvil raster [--width N] [--height N] <file or http/https/anvil URL> <output.ppm>
anvil install
anvil uninstall
```

`anvil install` copies the program into the per-user install directory. It does not download a second copy.

An engine or library change is tested and pushed in that repository before this one ships. `cargo test` passes for every touched crate. Source, tests, and release notes contain no keys, tokens, or personal data.

## Libraries

Linked by path. Not vendored. Not crates.io. Linux may use system libraries such as X11.

| Repository | License | What it is |
| --- | --- | --- |
| [anvil-engine](https://github.com/buntatoes/anvil-engine) | Proprietary | Page engine |
| [anvil-cipher](https://github.com/buntatoes/anvil-cipher) | MPL-2.0 | AES-128, AES-256, CTR, AES-256-GCM |
| [anvil-compress](https://github.com/buntatoes/anvil-compress) | MPL-2.0 | DEFLATE, zlib, gzip |
| [anvil-image](https://github.com/buntatoes/anvil-image) | MPL-2.0 | PNG, JPEG, GIF, WebP, ICO, BMP, PPM |
| [anvil-json](https://github.com/buntatoes/anvil-json) | MPL-2.0 | JSON |
| [anvil-ttf](https://github.com/buntatoes/anvil-ttf) | MPL-2.0 | TrueType and OpenType (`glyf`, `CFF `, `kern`, `GPOS`) |
| [anvil-gpu](https://github.com/buntatoes/anvil-gpu) | MPL-2.0 | Software boxes, shadows, gradients, NV12 |
| [anvil-video](https://github.com/buntatoes/anvil-video) | MPL-2.0 | CPU frames and PCM/ADPCM in a small MP4 |
