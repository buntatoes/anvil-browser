# flux-browser

[![Release](https://img.shields.io/badge/release-v.04-e8c37a)](https://github.com/buntatoes/flux-browser)
[![License: Proprietary](https://img.shields.io/badge/license-Proprietary-8b3a3a.svg)](./LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://github.com/buntatoes/flux-browser)

`flux-browser` is a ground-up web browser and custom rendering engine written in Rust with a native Win32 window shell.

It parses HTML into an arena-allocated document tree, loads linked and embedded CSS stylesheets, decodes `gzip`/`deflate` HTTP responses and `<img src="...">` raster images (PNG, JPEG, GIF89a, BMP, PPM/PGM), computes cascaded styles (tag, `.class`, `#id`, compound, and descendant selectors, inline `style` attributes, positioned layout, `z-index`, `overflow: hidden`, `box-shadow`, and `linear-gradient(...)`), lays out block, inline, Flexbox, and interactive form controls with TrueType font metrics, executes `<script>` blocks (including `for`/`while` loops, arrays, `classList`, `.value`/`.checked`, `setTimeout`, `fetch()`, `JSON.parse`/`JSON.stringify`, and DOM `click` listeners), emits a display list, rasterizes SDF rounded corners, box shadows, linear gradients, images, and anti-aliased TrueType text into an RGBA framebuffer, and presents interactive multi-tab pages with bookmarks, vertical scrollbar, and asynchronous `GET`/`POST` networking in a native window.

---

## Release Highlights (`v.04`)

- **Interactive HTML Forms, `<input>` Typing, Checkboxes, & HTTP `POST`**:
  - Live focusable `<input>` controls (text, password `*` masking, placeholder text, gold focus ring, and `|` caret), clickable `[X]`/`[ ]` checkboxes, `<button>`/`<input type="submit">` controls, `application/x-www-form-urlencoded` form serialization (`flux-dom`), and HTTP/1.1 `POST` requests with `301`/`302`/`303`/`307`/`308` redirect semantics (`flux-net`).
- **Positioned Layout, `z-index`, `overflow: hidden` Clipping, & Interactive Scrollbar**:
  - CSS `position: static | relative | absolute | fixed`, `top`/`right`/`bottom`/`left` offsets, `z-index` paint/hit-test stacking, `overflow: hidden` scissor clip stacks, scroll-pinned `fixed` layers, and a right-edge vertical scrollbar with click-to-jump navigation.
- **Bookmarks Bar & Persistent Session/Bookmark JSON Storage (`flux-json`)**:
  - Navigation-bar `*` Bookmark toggle button, dynamic Bookmarks Bar for one-click navigation, and JSON profile persistence (`Session::export_json`, `Session::import_bookmarks_from_json`, `Session::save_profile`, `Session::load_profile`).
- **Richer Scripting Engine (`flux-script`)**:
  - `for` and `while` loops, array literals (`[1, 2, 3]`) and bracket indexing (`arr[i]`), `.length`, `element.value` and `element.checked` getters/setters, `element.classList.add/remove/toggle/contains`, `setTimeout`, and `fetch(url)` (`.status`, `.ok`, `.text()`, `.json()`).
- **Upgraded Standalone Engine Libraries (`v.02`)**:
  - **`flux-ttf v.02`**: TrueType `loca`/`glyf` simple and compound contour decoding plus 4×4 supersampled scanline anti-aliased glyph outline rasterization (`TtfFont::rasterize_glyph` / `Framebuffer::draw_ttf_glyph`).
  - **`flux-image v.02`**: Baseline (`SOF0`) JPEG and GIF87a/GIF89a (LZW + transparency) decoders and encoders alongside PNG, BMP, and Netpbm.
  - **`flux-gpu v.02`**: SDF soft box shadows (`sdf_box_shadow_alpha`) and arbitrary-angle linear gradients (`sample_linear_gradient`) in both the software compositor and HLSL Shader Model 5.0 (`shaders/compositor.hlsl`), wired into CSS `box-shadow` and `linear-gradient(...)`.

---

## Rendering & Execution Pipeline

```text
URL / File Input
  -> flux-net + flux-compress (HTTP/1.1 GET & POST, redirects, chunked, gzip/deflate, Schannel TLS)
  -> flux-dom + flux-image    (Arena Document tree, HTML forms/inputs, PNG/JPEG/GIF/BMP/PPM decoding)
  -> flux-script + flux-json  (Loops, arrays, classList, fetch, timers, JSON.parse/stringify, click events)
  -> flux-style               (CSS cascade, position/z-index/overflow, border-radius, box-shadow, gradients)
  -> flux-layout + flux-ttf   (Block, Inline, Flexbox, & Out-of-Flow layout, TrueType metrics, hit-testing)
  -> flux-paint               (DisplayList: clips, fixed layers, shadows, gradients, rounded rects, text)
  -> flux-raster + flux-gpu   (RGBA framebuffer, scissor stack, SDF shadows/gradients/corners, TTF glyphs)
  -> flux-browser             (Multi-tab Session, Bookmarks Bar, scrollbar, async GET/POST worker, Win32 shell)
```

---

## Workspace & Standalone Dependency Architecture

| Crate / Dependency | Responsibility |
| :--- | :--- |
| [`flux-dom`](./crates/flux-dom) | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations, DOM query/mutation helpers, `classList`, form input/checkbox state, and `application/x-www-form-urlencoded` serialization), HTML parser, `<link rel="stylesheet">`, `<script>`, and `<img src>` discovery. |
| [`flux-style`](./crates/flux-style) | CSS stylesheet parser (tag, `.class`, `#id`, compound, and descendant selectors), inline `style="..."` attributes, user-agent defaults, property inheritance, `(ids, classes, tags)` specificity cascade, `position`/`top`/`right`/`bottom`/`left`/`z-index`/`overflow`, `border-radius`, `box-shadow`, `linear-gradient(...)`, and Flexbox properties. |
| [`flux-layout`](./crates/flux-layout) | Block, inline, Flexbox (`row` / `column`, `gap`, `justify-content`, `align-items`), and out-of-flow (`relative`, `absolute`, `fixed`) layout, interactive `<input>`/checkbox/button layout, intrinsic `<img>` sizing, TrueType proportional text measurement (`flux-ttf`), and `z-index`/clip-aware hit testing. |
| [`flux-paint`](./crates/flux-paint) | Converts the layout box tree into a `DisplayList` of `PushClip`/`PopClip`, `PushFixed`/`PopFixed`, `BoxShadow`, `LinearGradientRect`, `RoundedRect`, `Rect`, `Image`, underline, and `Text` commands ordered by `z-index`. |
| [`flux-raster`](./crates/flux-raster) | Software RGBA framebuffer rasterizer with scissor clip stack, fixed-layer scroll pinning, `flux-gpu` SDF rounded-corner, box-shadow, and linear-gradient rendering, `flux-image` bilinear image blitting, `flux-ttf` outline & Win32 GDI TrueType glyph rasterization, GDI BGRA output, and binary PPM export. |
| [`flux-script`](./crates/flux-script) | Proprietary scripting interpreter and DOM bindings (`let`/`const`, `if`/`else`, `while`, `for`, arrays, `document.getElementById`, `document.querySelector`, `document.createElement`, `textContent`, `value`, `checked`, `classList`, `style.*`, `setAttribute`, `appendChild`, `setTimeout`, `fetch`, `JSON.parse`, `JSON.stringify`, and `click` listeners). |
| [`flux-net`](./crates/flux-net) | URL parser, relative HTTP/HTTPS/file URL resolver, HTTP/1.1 `GET` and `POST` client with redirect following, chunked transfer decoding, `gzip`/`deflate` content decompression (`flux-compress`), and native Windows Schannel (`secur32.dll`) TLS. |
| [`flux-browser`](./crates/flux-browser) | Multi-tab session, history, and bookmarks controller, `flux-json` profile export/import, interactive form input & submission, Flux top header/bookmarks/scrollbar UI, asynchronous background networking worker thread, Win32 window shell, and `flux` CLI binary. |
| [`flux-compress`](https://github.com/buntatoes/flux-compress) | Standalone DEFLATE (RFC 1951), `zlib` (Adler-32), and `gzip` (CRC-32) compression and decompression library. |
| [`flux-image`](https://github.com/buntatoes/flux-image) | Standalone PNG, Baseline JPEG, GIF87a/GIF89a, BMP, and Netpbm (PPM/PGM) image decoder, encoder, and bilinear resampler. |
| [`flux-json`](https://github.com/buntatoes/flux-json) | Standalone zero-dependency RFC 8259 JSON parser, value tree, and compact/pretty serializer. |
| [`flux-ttf`](https://github.com/buntatoes/flux-ttf) | Standalone TrueType/OpenType SFNT font table parser (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, `kern`, `loca`, `glyf`), outline decoder, and anti-aliased scanline glyph rasterizer. |
| [`flux-gpu`](https://github.com/buntatoes/flux-gpu) | Standalone 2D signed-distance-field (SDF) quad compositor, linear gradients, box shadows, shelf-packing RGBA texture atlas, and HLSL Shader Model 5.0 pipeline. |

---

## Building and Testing

```powershell
cargo test
```

---

## CLI Usage

```powershell
# Open the interactive Win32 browser window
cargo run -p flux-browser -- open examples/hello.html
cargo run -p flux-browser -- open https://example.com

# Print the laid-out display list to stdout
cargo run -p flux-browser -- render examples/hello.html

# Rasterize a page directly to a binary PPM image
cargo run -p flux-browser -- raster --width 960 --height 720 examples/hello.html frame.ppm

# Print release version
cargo run -p flux-browser -- version
```

---

## Interactive Window Controls

| Input | Action |
| :--- | :--- |
| Click tab / `x` / `+` | Switch active tab, close tab, or open a new tab |
| Click `<` / `>` / `R` / `*` | Navigate back, navigate forward, reload the active page, or bookmark the active page |
| Click bookmark pill | Open the bookmarked page in the active tab |
| Click address field or `F6` | Focus the address field to type a URL or file path (`Enter` to navigate, `Esc` to cancel) |
| Click page `<input>` / checkbox / button | Focus text input to type (`Enter` to submit enclosing `<form>`, `Esc` to blur), toggle checkbox, or submit form (`GET` / `POST`) |
| Click page element / link | Dispatch script `click` listeners, or follow `<a href="...">` link (`http://`, `https://`, or relative file) |
| Mouse wheel / Scrollbar / `Up` / `Down` / `PageUp` / `PageDown` | Scroll the active page vertically |
| `Backspace` | Delete character in focused address/page input, or navigate back in tab history |
| `F5` | Reload the active page |
| `Esc` | Cancel address/input editing, or close the window |

---

## License

Proprietary — All Rights Reserved. See [`LICENSE`](./LICENSE) for details.

## Author

**Buntos**
