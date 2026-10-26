# Changelog

What changed in each Anvil release, newest first. Every entry comes from the commits in this repo.

Anvil has two release lines. `main` holds the latest release (v.23.1 right now). `testing` is a separate line where the next release gets built and tried first, so `testing` can be ahead of `main` and its version isn't on `main` until it ships there. Version numbers read like `v.23.2`, and the crates use `0.23.2`.

The engine and the nine libraries are their own repos with their own version numbers. When a release needs a newer one, the lock file here moves with it, and the entry says so.

## v.23.2 (testing, not released yet)

Style sheets:

- `<style>` blocks and `<link rel="stylesheet">` sheets now apply in the order they appear in the page. Before, every linked sheet was put ahead of every `<style>` block, so a link written later lost ties it should have won.
- Each sheet is parsed on its own, so a broken sheet no longer swallows the one after it.
- A sheet's `media` attribute works the same way as `@media`, so a print sheet doesn't apply on screen.
- Restyling a page now respects a policy that blocks inline style, the same as the first render.
- The page's CSS is parsed once and only parsed again when a sheet changes. Animation frames and window resizes reuse it.
- Uses anvil-engine 0.23.2, which honours `!important`, drops a whole rule when one selector in its list is invalid, reads media types, `not`, `only` and comma lists in `@media`, skips strings, escapes and unquoted `url()` bodies when scanning a sheet, and lists sheets in document order with their media.

Building and testing:

- Builds on Rust 1.85.1 again. The HTTP/2 `RST_STREAM` handling and anvil-video both used a newer `if let` form that 1.85.1 can't compile. Same behaviour, plain nested `if`s. Uses anvil-video 0.7.1, which has that fix.
- Tests run one at a time. Opening a session caps the test process at 2 GiB of address space, the same cap the window gets, and with tests running side by side a test could sometimes fail with "failed to spawn thread".
- CI now clones anvil-pqc too, which it was missing, and also runs on pushes to `testing`.
- `Cargo.toml` says `rust-version = "1.85.1"`, every crate links the repo, and the C++20 parts are marked optional.

Docs:

- The README and SECURITY.md now say what page loads really use: OpenSSL on Linux or Schannel on Windows, one connection per request. The in-house TLS 1.3 code, the multi-stream HTTP/2 session and the connection pool are in `anvil-net` but page loads don't use them yet.
- The `MutationObserver`, `IntersectionObserver` and `ResizeObserver` objects are described as placeholders that never call back.
- The line count badge is up to date, the two badges that CI didn't back up are gone, and a live CI badge shows how `testing` is doing.
- Added this changelog.

## v.23.1 (main)

- A WebAssembly 1.0 runtime for pages, with the `WebAssembly` namespace, typed array views, `ArrayBuffer`, `TextEncoder`, `TextDecoder`, `atob` and `btoa`, `URLSearchParams` and `DOMParser`.
- Click handlers on pages loaded in a tab worker are hooked back up in the window.
- `anvil-net` gained an in-house TLS 1.3 handshake and record layer built on anvil-pqc and anvil-cipher, a multi-stream HTTP/2 session and a connection pool. These aren't on the page load path yet.
- Windows offers HTTP/2 through ALPN too, and shows the key exchange group of recent connections on `anvil://security`.
- Subresource integrity accepts `sha3-256-` hashes as well as `sha256-`.
- Uses anvil-pqc 0.1.1.
- Added the full Apache-2.0 and engine license texts, and a total line count badge.

## v.22

- Basic pages for some large sites that open as themselves. Their full web apps don't run.
- A built-in map page drawn from public map tiles, with optional imagery, streets and topo layers and saved web maps.
- Every tab gets its own worker process for http(s) pages and local files.
- HTTP/2 for https on Linux when the server offers it. Response fields with CR, LF or NUL are refused.
- Privacy controls: HTTPS-only mode, per-site permissions, and cookies partitioned by the top-level site.
- On Linux the X25519MLKEM768 post-quantum hybrid key exchange is offered first, and `anvil://security` shows the group each connection used.
- Changes made through `anvil://` pages need a key made fresh each time the browser starts.
- CSS transitions and animations run, with frames drawn from the window loop, and sticky boxes are hit-tested where they're drawn.
- Typing into the address bar, find, and page fields works on Linux.
- A TLS connection fails when the hostname check can't be set, and each connect has a time limit.
- A CI workflow for Linux and Windows, with a Rust 1.85.1 build too.
- The browser is Apache-2.0, the libraries are MPL-2.0, and the engine has its own license.

## v.21

- CSS custom properties (`var()`), `calc()` lengths and `display: grid`.
- More of the script interpreter: newer syntax, `try`/`catch` recovery, and DOM and CSSOM changes from script.
- Uses anvil-ttf v.07 for font subroutines.

## v.20

- A numeric loop that sets `textContent` stays in machine code, with the page write going back through the interpreter.
- A parallel strip rasterizer, a dirty-tile grid, wider span fills and a three-pass box blur.
- CFF fonts (anvil-ttf v.06) and an audio output bridge (anvil-video v.07) in the video player.
- Fixed the Windows x64 machine code build and unified the `profile.anvil` file header.

## v.19

- The project and its crates were renamed to Anvil, and the Anvil mark is on the home page.
- Numeric `while` and `for` loops on a page run as x86-64 machine code.
- Shadows, rounded boxes, gradients and video frames are drawn through the software draw device.

## v.18

- http and https loads go through a tab worker. If no answer arrives within five seconds, the tab shows an error page.
- Windows can upload the CPU framebuffer through Direct3D 11. Pages are still drawn on the CPU.

## v.17

- A C++20 bytecode VM, a C++20 raster compositor, a script prelude and front-end scripts.
- Release docs and the version list.

## v.16

- Linux profiles are sealed with AES-256-GCM from anvil-cipher. The key file stays with its profile directory, and a bad tag is rejected.
- OpenSSL is only used for HTTPS.
- A tab worker stops if its 256 MiB address cap can't be applied.

## v.15

- C++20 parts next to the Rust code, and a switch for the machine code compiler.
- A two-column settings page.
- The first license split between the browser, the engine and the libraries, plus SECURITY.md.

## v.13

- Encrypted demo video with local keys, and a download list that records each file's SHA-256.

## v.11

- Profiles are saved locally, the video player has a scrubber, and the built-in pages were tidied up.

## v.08

- The window and the tab worker run under OS sandbox limits, and the browser won't start if the sandbox can't be set up.
- Content Security Policy, a cross-origin resource policy, and a cookie jar.

## v.07

- A Linux X11 window, with OpenSSL for HTTPS on Linux.
- CSS `@media` queries, HTML tables, and `<textarea>` and `<select>`.
- A Windows setup program and uninstaller, and the browser logo.

## v.06

- `<video>` playback and the settings and history pages.

## v.05

- The built-in home and security pages and an installer.

## v.04

- Forms you can fill in, positioned layout, bookmarks, and more scripting.

## v.03

- The first companion libraries: gzip and deflate for responses, `<img>` decoding, TrueType text measuring, rounded corners and JSON for scripts.

## v.02

- A rasterizer, hit testing and a Windows window, then HTTPS through Schannel, redirects, linked style sheets, CSS selectors, flexbox, TrueType fonts and the first script runtime.

## v.01

- The first page engine: the document tree, style, layout, paint, plain HTTP and tabs.
