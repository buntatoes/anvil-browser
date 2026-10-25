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

Anvil is a small desktop browser I'm building from scratch in Rust, with a bit of C++20 and JavaScript. It uses no crates.io crates: every Rust dependency lives in my own repos, linked by path. On Linux it also links the system X11 and OpenSSL libraries, and on Windows it uses the system APIs. It runs on Linux and Windows.

It's a proof of concept, so expect rough edges. Current version: 0.22.

## Try it

No v.22 Windows build is on the releases page yet. The last packaged build is the [v.21 release](https://github.com/buntatoes/anvil-browser/releases/tag/v.21): `anvil.exe` is the browser on its own, and `AnvilBrowser-Setup-v.21-win64.exe` is the installer. The SHA-256 checksums are in the release notes.

## Build it

You need Rust 1.85 or newer. A C++20 compiler (`g++` on Linux, `cl.exe` on Windows) is optional. Without one, the C++ parts are skipped.

On Linux you also need the X11, `libssl` and `libcrypto` development files. On Debian or Ubuntu:

```sh
sudo apt install libx11-dev libssl-dev
```

The local TLS tests use the `openssl` command if it's installed and skip themselves if it isn't.

Clone the repos side by side in one folder:

```text
anvil-browser/
anvil-engine/     (private for now)
anvil-cipher/
anvil-compress/
anvil-hpack/
anvil-image/
anvil-json/
anvil-ttf/
anvil-gpu/
anvil-video/
```

Then, from `anvil-browser`:

```sh
cargo test
cargo run --release -p anvil-browser --bin anvil
```

Same steps on Windows. Note that the engine repo is private right now, so a full build needs access to it. The engine is there for auditing and use only. You can redistribute the browser without the engine, see [License](#license).

A few other ways to run it:

```text
anvil [file or URL]                     open a window
anvil render <file or URL>              print a page's display list
anvil raster <file or URL> <out.ppm>    draw a page to a PPM image
anvil install / anvil uninstall         per-user install
```

## What works today

- HTML parsing, a CSS cascade (media queries, custom properties, `calc()`), and layout for block, inline, flex, grid, tables and positioned boxes. Width, height, max-width, and offsets can use `vw`, `vh`, `vmin`, and `vmax`.
- Painting into a CPU framebuffer, with shadows, rounded boxes and gradients.
- A JavaScript interpreter with a time budget. Simple numeric loops can compile to x86-64 machine code. Everything else stays interpreted.
- HTTP/1.1, HTTP/2 over HTTPS on Linux, cookies, HSTS, CSP, and SHA-256 subresource checks. `fetch()` is same-origin only.
- `localStorage` and `sessionStorage`, kept per origin and capped at 64 KiB.
- Tabs, each with its own worker process for http, https and local pages. If a tab's worker hangs for 5 seconds or crashes, that tab shows an error page and the rest of the browser keeps going. Closing a tab ends its worker.
- Bookmarks, history, downloads, find, and built-in pages: `anvil://home`, `settings`, `security`, `history`, `downloads` and `inspect`.
- An encrypted profile file: DPAPI on Windows, AES-256-GCM on Linux.
- Images (PNG, JPEG, GIF, ICO, BMP, PPM), TrueType and OpenType fonts, and small MP4 clips decoded on the CPU.
- YouTube, Gmail, Facebook, Instagram, X, Outlook, TikTok, LinkedIn, Reddit, other social hosts, and bank and payment hosts load the documents those sites return. External scripts on those hosts are left unloaded. A YouTube watch page keeps the document and adds a player made from the JPEG frames YouTube publishes (`hqdefault`, `hq1`, `hq2`, and `hq3`), or from a storyboard sheet when the page has one. Reddit opens the address you asked for. Requests to Reddit, OpenStreetMap tiles, and OpenStreetMap place lookup use a longer User-Agent that names this browser.
- Google Maps, OpenStreetMap, ArcGIS, and `anvil://maps` are drawn in this process as nine tiles fetched together. Tiles already fetched are reused on the next pan or zoom. OpenStreetMap is the default layer. ArcGIS imagery, streets, and topo are optional. A `webmap=` item uses that item's tiled basemap. When the item has a feature layer, one layer is drawn on top, at most 80 features. An ArcGIS address uses the streets layer, the imagery layer when the address names `World_Imagery`, and the topo layer when it names `World_Topo`. A place name is looked up once. Pan, zoom, and the layer switch are links. A token on the address is sent to ArcGIS and is not kept. No maps API key is stored.

Handy keys: Ctrl+T / Ctrl+W for tabs, Ctrl+L or F6 for the address bar, Ctrl+R or F5 to reload, Alt+Left / Alt+Right for back and forward, Alt+Home for the start page, Ctrl+F to find (Enter for the next match), Ctrl+J / Ctrl+H for downloads and history, Ctrl+plus, Ctrl+minus and Ctrl+0 for zoom, and F12 for the inspector. These work the same on Windows and Linux. On Linux, Ctrl+G or F3 also go to the next match, and Shift with Enter, Ctrl+G or F3 goes to the previous one. You can type in the address bar, in find, and in page text fields on both. Escape leaves a field, then closes menus, find and the inspector, and with nothing left open it closes the window. Home, End, the arrow keys and Page Up / Page Down scroll. On Linux, copy and paste use the browser's own clipboard for now, not the X selection.

## What it can't do (yet)

- Each tab has its own worker process for http, https and local pages, but the window process still fetches pages, keeps cookies, and lays out and paints what the worker sends back. `anvil://` pages are built in the window process.
- No hardware GPU rendering. Pages are drawn on the CPU. Windows can hand the finished frame to Direct3D 11, and Linux copies pixels with X11.
- No GPU video decoding. Video is decoded on the CPU.
- Real WebP files don't load. `anvil-image` only reads the simple WebP-style files it writes itself.
- The machine-code path only handles numeric loops. It isn't a general compiler.
- CSS gaps: no `ch` units, no grid row tracks, no grid items that span several columns, and no `calc()` that mixes viewport units. Margin and padding are `px` only.
- The YouTube player shows the published JPEG frames. External scripts on the large sites above stay unloaded. A map address draws the tile page in this process.

## How it's laid out

This repo is the browser shell:

| Path | What it is |
| --- | --- |
| `crates/anvil-net` | HTTP/1.1, HTTP/2, TLS, and the host block list |
| `crates/anvil-sandbox` | Process limits and the worker messages |
| `crates/anvil-browser` | The window, tabs, downloads, and the profile file |
| `frontend/` | The built-in pages (plain files, no Node build) |
| `examples/` | A couple of test pages |

The page engine (HTML, CSS, layout, paint, raster and script) lives in `anvil-engine`, which is private for now.

The eight libraries are their own repos, and Cargo finds them by path (`../anvil-image` and so on):

- [anvil-cipher](https://github.com/buntatoes/anvil-cipher): AES and AES-256-GCM
- [anvil-compress](https://github.com/buntatoes/anvil-compress): DEFLATE, zlib and gzip
- [anvil-hpack](https://github.com/buntatoes/anvil-hpack): HPACK header compression for HTTP/2
- [anvil-image](https://github.com/buntatoes/anvil-image): image decoders
- [anvil-json](https://github.com/buntatoes/anvil-json): JSON
- [anvil-ttf](https://github.com/buntatoes/anvil-ttf): fonts
- [anvil-gpu](https://github.com/buntatoes/anvil-gpu): software drawing for shadows, gradients and video frames
- [anvil-video](https://github.com/buntatoes/anvil-video): small MP4 clips with PCM or ADPCM audio

## Security

The limits that are really in the code, and how to report a problem privately, are in [SECURITY.md](./SECURITY.md). Please don't post security bugs in public issues.

## License

This repo is Apache-2.0 ([LICENSE](./LICENSE)). The eight libraries are MPL-2.0 ([LICENSE-MPL](./LICENSE-MPL)), and `anvil-engine` is proprietary.

The engine is shared for auditing and use only. You can't redistribute it. Everything else (this repo and the eight libraries) can be redistributed under the licenses above, as a browser without the engine included.

Made by Buntato ([@buntatoes](https://github.com/buntatoes)).
