# Flux Browser

Flux is a small desktop browser. The HTML, CSS, layout, and a limited JavaScript interpreter are written here. It is not a drop-in replacement for Chrome or Firefox.

Version 0.16.

Linux uses X11 for the window and OpenSSL (`libssl` / `libcrypto`) for HTTPS. Windows uses the system windowing APIs, Schannel, and DPAPI. The Linux profile is sealed by [flux-cipher](https://github.com/buntatoes/flux-cipher), not by libcrypto.

There are no crates.io dependencies. Seven other repositories have to sit next to this one or the build cannot find them:

- [flux-cipher](https://github.com/buntatoes/flux-cipher)
- [flux-compress](https://github.com/buntatoes/flux-compress)
- [flux-image](https://github.com/buntatoes/flux-image)
- [flux-json](https://github.com/buntatoes/flux-json)
- [flux-ttf](https://github.com/buntatoes/flux-ttf)
- [flux-gpu](https://github.com/buntatoes/flux-gpu)
- [flux-video](https://github.com/buntatoes/flux-video)

```text
../flux-cipher
../flux-browser
../flux-compress
../flux-image
../flux-json
../flux-ttf
../flux-gpu
../flux-video
```

## Build

Rust 1.85 or newer, and `g++` or `clang++` if you want the small C++ helpers compiled in. The browser still builds if the C++ compile is skipped.

```powershell
cargo test
cargo run --release -p flux-browser --bin flux
```

On Linux the window process sets `PR_SET_NO_NEW_PRIVS`, turns core dumps off, and tries a 2 GiB address-space cap. Pages are drawn in that same process. `--tab-worker` is a separate mode with a 256 MiB cap. It is not started for normal browsing.

Video frames are converted on the CPU. Encrypted samples use AES-128-CTR from `flux-cipher`. The profile on Linux is zlib plus AES-256-GCM. The key file is `profile.key` in the same directory, written before the profile, mode 0600.

## Layout

| Path | What it is |
| --- | --- |
| `crates/flux-dom` | HTML tree |
| `crates/flux-style` | CSS |
| `crates/flux-layout` | Box layout |
| `crates/flux-paint` | Display list |
| `crates/flux-raster` | Framebuffer. Linux text uses `flux-ttf` when a system font is present |
| `crates/flux-script` | A small script interpreter. `cpp/flux_jit_accel.cpp` only does arithmetic |
| `crates/flux-net` | HTTP and TLS |
| `crates/flux-sandbox` | The process limits above |
| `crates/flux-browser` | Window, pages, downloads, profile |

`frontend/` is included as text and turned into HTML with a few string replacements. It is not compiled by a React toolchain.

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

## License

`flux-dom`, `flux-style`, `flux-layout`, `flux-paint`, `flux-raster`, and `flux-script` are Apache-2.0. `flux-net`, `flux-sandbox`, `flux-browser`, `frontend/`, and `assets/flux-logo.png` are all rights reserved. The sibling libraries, including `flux-cipher`, are MPL-2.0. The license text is in `LICENSE`, `LICENSE-APACHE`, `LICENSE-MPL`, and `LICENSE-PROPRIETARY`.

## Author

Buntos ([@buntatoes](https://github.com/buntatoes))
