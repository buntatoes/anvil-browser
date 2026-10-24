<div align="center">
  <img src="./assets/flux-logo.png" width="96" height="96" alt="Flux" />
  <h1>Flux Browser</h1>
  <p>A small desktop browser. Not a stand-in for Chrome or Firefox.</p>
  <p>
    <img src="https://img.shields.io/badge/release-v.16-ff6b24?style=flat-square" alt="v.16" />
    <img src="https://img.shields.io/badge/status-Proof_of_Concept-d4a359?style=flat-square" alt="Proof of Concept" />
    <img src="https://img.shields.io/badge/platform-Windows_%7C_Linux-2ea44f?style=flat-square" alt="Windows and Linux" />
    <img src="https://img.shields.io/badge/rust-2024-orange?style=flat-square" alt="Rust 2024" />
    <a href="./SECURITY.md"><img src="https://img.shields.io/badge/security-policy-2ea44f?style=flat-square" alt="Security" /></a>
    <a href="./LICENSE"><img src="https://img.shields.io/badge/license-Apache--2.0_%2B_MPL--2.0_%2B_Proprietary-e8c37a?style=flat-square" alt="License" /></a>
  </p>
</div>

Version 0.16.

Linux draws the window with X11 and does HTTPS with OpenSSL (`libssl` / `libcrypto`). Windows uses the system window APIs, Schannel, and DPAPI. The Linux profile is AES-256-GCM from [flux-cipher](https://github.com/buntatoes/flux-cipher). The key file is `profile.key` in the same directory, mode 0600, written before the profile.

Video frames are converted on the CPU. Encrypted samples are AES-128-CTR from `flux-cipher`. There is no GPU decoder here.

## Layout

| Path | What it is |
| --- | --- |
| `crates/flux-dom` | HTML tree and selectors |
| `crates/flux-style` | CSS cascade and media queries |
| `crates/flux-layout` | Block, inline, flex, table, and positioned boxes |
| `crates/flux-paint` | Display list and zoom |
| `crates/flux-raster` | RGBA framebuffer. Linux text uses `flux-ttf` when a system font is present. A small C++ file helps with scanlines when a compiler is on `PATH` |
| `crates/flux-script` | A small script interpreter. `cpp/flux_jit_accel.cpp` only does arithmetic |
| `crates/flux-net` | HTTP, TLS, and a short list of blocked host names |
| `crates/flux-sandbox` | Process limits |
| `crates/flux-browser` | Window, pages, downloads, and the profile file |

`frontend/` is stored as text and turned into HTML with a few string replacements. It is not built by a React toolchain.

## Build

Clone this repository and the seven libraries below as sibling directories. There are no crates.io dependencies.

```text
../flux-browser
../flux-cipher
../flux-compress
../flux-image
../flux-json
../flux-ttf
../flux-gpu
../flux-video
```

Rust 1.85 or newer. `g++` or `clang++` is optional. The C++ files are skipped if no compiler is on `PATH`.

```
cargo test
cargo run --release -p flux-browser --bin flux
```

On Linux the window sets `PR_SET_NO_NEW_PRIVS`, turns core dumps off, and tries a 2 GiB address-space cap. Pages are drawn in that process. `--tab-worker` is a separate mode with a 256 MiB cap. Normal browsing does not start it. If that cap does not stick, the worker exits.

## Keys

| Keys | Action |
| --- | --- |
| Ctrl+T / Ctrl+W | New tab / close tab |
| Ctrl+L | Address bar |
| Ctrl+R | Reload |
| Alt+Left / Alt+Right | Back / forward |
| Ctrl+F | Find |
| Ctrl+J / Ctrl+H | Downloads / history |
| F12 | Inspector |

## The other repositories

| Repository | What it is |
| --- | --- |
| [flux-cipher](https://github.com/buntatoes/flux-cipher) | AES-128, AES-256, CTR, AES-256-GCM |
| [flux-compress](https://github.com/buntatoes/flux-compress) | DEFLATE, zlib, gzip |
| [flux-image](https://github.com/buntatoes/flux-image) | PNG, JPEG, GIF, WebP, ICO, BMP, PPM |
| [flux-json](https://github.com/buntatoes/flux-json) | JSON |
| [flux-ttf](https://github.com/buntatoes/flux-ttf) | TrueType and OpenType |
| [flux-gpu](https://github.com/buntatoes/flux-gpu) | CPU drawing of boxes, shadows, and NV12 |
| [flux-video](https://github.com/buntatoes/flux-video) | CPU frames in a small MP4 container |

## License

`flux-dom`, `flux-style`, `flux-layout`, `flux-paint`, `flux-raster`, and `flux-script` are Apache-2.0. `flux-net`, `flux-sandbox`, `flux-browser`, `frontend/`, and `assets/flux-logo.png` are all rights reserved. The seven libraries above are MPL-2.0. The text is in `LICENSE`, `LICENSE-APACHE`, `LICENSE-MPL`, and `LICENSE-PROPRIETARY`.

## Author

Buntos ([@buntatoes](https://github.com/buntatoes))
