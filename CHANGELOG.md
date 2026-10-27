# Changelog

What changed in each Anvil release, newest first. Every entry comes from the commits in this repo.

Anvil has two release lines. `main` holds the latest release (v.23.2 right now). `testing` is a separate line where the next release gets built and tried first, so `testing` can be ahead of `main` and its version isn't on `main` until it ships there. Version numbers read like `v.23.2`, and the crates use `0.23.2`.

The engine and the eleven libraries are their own repos with their own version numbers. When a release needs a newer one, the lock file here moves with it, and the entry says so.

## v.23.2 (main)

JavaScript and CSS:

- Self-hosted `anvil-js` (0.2.0) ES2022+ JavaScript compiler, lowering pass, standard library, and Web/DOM runtime (`AnvilPromise`, `AnvilMap`, `AnvilSet`, `AnvilURL`, `AnvilURLSearchParams`, `AnvilDOMNode`, `AnvilDOMDocument`, `AnvilCustomElementRegistry`, `AnvilShadowRoot`, `AnvilMutationObserver`, `AnvilIntersectionObserver`, `AnvilResizeObserver`, `AnvilMediaSource`, `AnvilAbortController`, `AnvilHeaders`, `AnvilRequest`, `AnvilResponse`, `AnvilXMLHttpRequest`, `AnvilHistory`, `AnvilLocation`, `AnvilStorage`) written in Core JS and executing on the C++20 `AnvilVM` register bytecode VM (`compile_es2022_with_anvil_js`, `eval_es2022_with_anvil_js`, `eval_webapp_with_anvil_js`).
- `anvil-css` (0.1.0) CSS Syntax Level 3 tokenizer, Selectors Level 4 matcher, 261-property value/color/`calc()` parser, and media/container/`@supports` evaluator wired into `anvil-style`.

Style sheets:

- `<style>` blocks and `<link rel="stylesheet">` sheets now apply in the order they appear in the page. Before, every linked sheet was put ahead of every `<style>` block, so a link written later lost ties it should have won.
- Each sheet is parsed on its own, so a broken sheet no longer swallows the one after it.
- A sheet's `media` attribute works the same way as `@media`, so a print sheet doesn't apply on screen.
- Restyling a page now respects a policy that blocks inline style, the same as the first render.
- The page's CSS is parsed once and only parsed again when a sheet changes. Animation frames and window resizes reuse it.
- Uses anvil-engine 0.23.2, which honours `!important`, drops a whole rule when one selector in its list is invalid, reads media types, `not`, `only` and comma lists in `@media`, skips strings, escapes and unquoted `url()` bodies when scanning a sheet, and lists sheets in document order with their media.

Tab processes:

- Each tab's pages now run in an `anvil --renderer` process that speaks a small binary format with a size cap on every field. The renderer starts with an empty environment and `/` as its folder, so on Linux it no longer gets `DISPLAY` from the window.
- Every request and answer carries a navigation id. A renderer that is late, exits, sends a bad frame, or answers for another navigation is killed, and the next page in that tab gets a new one.
- After three renderer crashes in a minute, a tab stops starting new ones and shows an error page until the oldest crash is a minute old.
- Between pages each renderer gets a heartbeat every 5 seconds. On Linux it's also killed if it uses more than 1 second of CPU while it has nothing to do.
- Error pages show a fixed text for each kind of failure instead of the renderer's own message.
- The renderer has no network any more. It asks the window for each linked script and each script `fetch()`, and the window applies the shield, mixed content, HTTPS-only, the cross-origin resource policy and the same-origin rule before it fetches. Cookies stay in the window: script `fetch()` sends and stores them in the window's jar, and the renderer never sees them.
- A renderer can ask for at most 512 fetches for one page and has 60 seconds for the whole page, or it's killed.
- The renderer asks the window for the page's style sheets and images too, and decodes the images itself, up to 48 MiB a page. The window still loads them again for the page it draws. Video and map tiles stay with the window.
- `anvil --tab-worker` and its JSON frames are gone. The window stopped starting it when tabs moved to `--renderer`.
- On Linux the renderer is started through `bwrap` when the system has it, or `unshare` when `bwrap` is missing or fails. It gets its own user, pid, network, IPC, UTS and mount namespaces, so it can't see or read the window's process. Under `bwrap` it also sees only a read-only `/usr` and its own executable, wherever that is kept, `/tmp` included. `prlimit`, when present, caps open files at 256 and sets the file size limit to 0, which stops writing data into files but not creating empty files, truncating, renaming or deleting them. Only `bwrap`'s read-only view keeps the host's files unchanged; under `unshare` they stay writable. A tool is only used after it passes a self-test when the window starts. Without one, renderers start as before. On Windows renderers start from a copy of the executable with a low integrity label.
- `anvil://security` has a Renderer confinement section. It lists what a fresh renderer reads about itself from the OS, line by line, marks only what that reading shows as active, says why no tool is used when none is, and has a link to turn confinement off or back on. The choice is kept in the profile.
- The renderer format can carry a painted page: the page's display list, with a cap on every count, length and number, and a check in the window before anything is drawn from it. Very long or very large pages are cut to fit. Text in it is drawn with the system's fonts, so the window never reads a font file from a page. Nothing sends it yet; pages still come back as HTML.
- The renderer now also sends the page it painted, next to the HTML. The window checks it and compares it with the page it drew itself, then drops it; it doesn't draw from it yet. A painted page that fails the check, or a second one for the same page, shows an error page and ends that renderer, and counts as a crash.
- Each link in a painted page also lists the fixed, sticky and transformed boxes it sits in, up to eight, so the window can find a link where it is drawn on screen and not where it was laid out. Each one is checked like a layer.
- `anvil://security` has a switch to draw web pages from the page their renderer painted, once the window has checked it, instead of from the window's own layout. It is off by default and the choice is kept in the profile. When it is on, scrolling, sticky and fixed boxes, find, link clicks and text selection use the painted page, and a page cut to fit only scrolls as far as it was cut. Clicks, typing, resizing and animations still change the window's own copy of the page, and once one does, the window draws its own copy again. `anvil://` pages are always drawn by the window.
- Tests draw a set of pages both ways, from the window's own copy and from the renderer's painted page, at several scroll positions and zoom levels, and require every pixel to match: text and links, rounded boxes, shadows, gradients, see-through and turned boxes, sticky and fixed boxes, flex, grid and clipping, a page its script changed, and images. A page too long for a painted page matches above the cut.
- Links on `anvil://security` now work when clicked, the confinement switch among them. The page listens for clicks on each of its sections, and a click that a page's script handled never followed a link. On Anvil's own pages a link is now followed after the page's listeners run. Web pages are unchanged.

Building and testing:

- Builds on Rust 1.85.1 again. The HTTP/2 `RST_STREAM` handling and anvil-video both used a newer `if let` form that 1.85.1 can't compile. Same behaviour, plain nested `if`s. Uses anvil-video 0.7.1, which has that fix.
- Tests run one at a time. Opening a session caps the test process at 2 GiB of address space, the same cap the window gets, and with tests running side by side a test could sometimes fail with "failed to spawn thread".
- CI now clones anvil-pqc too, which it was missing, and also runs on pushes to `testing`.
- `Cargo.toml` says `rust-version`, every crate links the repo, and the C++20 parts are marked optional.
- The oldest Rust it builds with is now 1.99.1, up from 1.85.1. `rust-version`, the README and the CI job that builds on the oldest toolchain all say 1.99.1, and the engine and the nine libraries moved up with it. Clippy's suggestions that need the newer Rust, like `if let` chains, are now used where it asked for them. Uses anvil-cipher 0.2.1, anvil-compress 0.3.1, anvil-gpu 0.9.1, anvil-hpack 0.1.2, anvil-image 0.7.1, anvil-json 0.3.2, anvil-pqc 0.2.0, anvil-ttf 0.8.1 and anvil-video 0.7.2, which ask for 1.99.1 too.

Docs:

- The README and SECURITY.md now say what page loads really use: OpenSSL on Linux or Schannel on Windows, one connection per request. The in-house TLS 1.3 code, the multi-stream HTTP/2 session and the connection pool are in `anvil-net` but page loads don't use them yet.
- The `MutationObserver`, `IntersectionObserver` and `ResizeObserver` objects are described as placeholders that never call back.
- The line count badge is up to date, the two badges that CI didn't back up are gone, and a live CI badge shows how `testing` is doing.
- Added this changelog.

## v.23.1

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
