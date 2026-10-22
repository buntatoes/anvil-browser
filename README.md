# flux-browser

[![Release](https://img.shields.io/badge/release-v.03-e8c37a)](https://github.com/buntatoes/flux-browser)
[![License: Proprietary](https://img.shields.io/badge/license-Proprietary-8b3a3a.svg)](./LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://github.com/buntatoes/flux-browser)

`flux-browser` is a ground-up web browser and custom rendering engine written in Rust with a native Win32 window shell.

It parses HTML into an arena-allocated document tree, loads linked and embedded CSS stylesheets, decodes `gzip`/`deflate` HTTP responses and `<img src="...">` raster images, computes cascaded styles (tag, `.class`, `#id`, compound, and descendant selectors plus inline `style` attributes), lays out block, inline, and Flexbox containers with TrueType font metrics, executes `<script>` blocks (including `JSON.parse`/`JSON.stringify` and DOM `click` listeners), emits a display list, rasterizes SDF rounded corners, images, and anti-aliased TrueType text into an RGBA framebuffer, and presents interactive multi-tab pages with asynchronous networking in a native window.

---

## Release Highlights (`v.03`)

- **HTTP Content-Encoding Decompression (`flux-compress`)**: Transparent `gzip` (RFC 1952) and `deflate` / `zlib` (RFC 1950 / RFC 1951) payload decompression in `flux-net`, advertising `Accept-Encoding: gzip, deflate, identity`.
- **Subresource Image Loading & Bilinear Rasterization (`flux-image`)**: Automatic `<img src="...">` discovery in `flux-dom`, network/file image fetching and PNG/BMP/PPM decoding in `flux-browser`, intrinsic/CSS replaced-box sizing in `flux-layout`, and bilinear-resampled RGBA compositing in `flux-raster`.
- **TrueType / OpenType Font Metrics (`flux-ttf`)**: Proportional text advance width and kerning pair measurement in `flux-layout` backed by `flux-ttf`.
- **Signed-Distance-Field Rounded Corners (`flux-gpu`)**: CSS `border-radius` parsing and cascade in `flux-style`, `DisplayItem::RoundedRect` emission in `flux-paint`, and per-pixel anti-aliased SDF rounded box rendering in `flux-raster`.
- **JSON Scripting & Session Serialization (`flux-json`)**: Native `JSON.parse` and `JSON.stringify` bindings with object property access in `flux-script`, plus multi-tab `Session::export_json` and `Session::parse_exported_urls` state persistence in `flux-browser`.

---

## Rendering & Execution Pipeline

```text
URL / File Input
  -> flux-net + flux-compress (HTTP/1.1, redirects, chunked, gzip/deflate, Schannel TLS)
  -> flux-dom + flux-image    (Arena Document tree, HTML parser, <link>, <script>, & <img> decoding)
  -> flux-script + flux-json  (Script interpreter, JSON.parse/stringify, DOM mutations, click events)
  -> flux-style               (CSS parser, specificity cascade, inline styles, border-radius, Flexbox)
  -> flux-layout + flux-ttf   (Block, Inline, & Flexbox layout, TrueType metrics, hit-testing)
  -> flux-paint               (Flat DisplayList: rects, SDF rounded rects, images, underlines, text)
  -> flux-raster + flux-gpu   (RGBA framebuffer, SDF rounded corners, bilinear images, Win32 GDI glyphs)
  -> flux-browser             (Multi-tab Session, JSON export, top bar UI, async worker, Win32 shell)
```

---

## Workspace & Standalone Dependency Architecture

| Crate / Dependency | Responsibility |
| :--- | :--- |
| [`flux-dom`](./crates/flux-dom) | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations, DOM query/mutation helpers), HTML parser, `<link rel="stylesheet">`, `<script>`, and `<img src>` discovery. |
| [`flux-style`](./crates/flux-style) | CSS stylesheet parser (tag, `.class`, `#id`, compound, and descendant selectors), inline `style="..."` attributes, user-agent defaults, property inheritance, `(ids, classes, tags)` specificity cascade, `border-radius`, box-model sides/borders, and Flexbox properties. |
| [`flux-layout`](./crates/flux-layout) | Block, inline, and Flexbox (`row` / `column`, `gap`, `justify-content`, `align-items`) formatting context layout, intrinsic `<img>` sizing, list markers (`<ul>` / `<ol>` / `<li>`), `<hr>`, TrueType proportional text measurement (`flux-ttf`), word wrapping, and reverse-paint-order coordinate hit testing. |
| [`flux-paint`](./crates/flux-paint) | Converts the layout box tree into a flat `DisplayList` of `Rect`, `RoundedRect`, `Image`, underline, and `Text` commands. |
| [`flux-raster`](./crates/flux-raster) | Software RGBA framebuffer rasterizer with clipping, alpha blending, vertical scroll offset, `flux-gpu` SDF rounded-corner rendering, `flux-image` bilinear image blitting, native Win32 GDI TrueType anti-aliased glyph rasterizer (with 5x7 ASCII bitmap fallback), GDI BGRA output, and binary PPM export. |
| [`flux-script`](./crates/flux-script) | Proprietary scripting interpreter and DOM bindings (`document.getElementById`, `document.querySelector`, `document.createElement`, `textContent`, `style.*`, `setAttribute`, `appendChild`, `JSON.parse`, `JSON.stringify`, and bubbling `click` event listeners). |
| [`flux-net`](./crates/flux-net) | URL parser, relative HTTP/HTTPS/file URL resolver, HTTP/1.1 client with redirect following, chunked transfer decoding, `gzip`/`deflate` content decompression (`flux-compress`), and native Windows Schannel (`secur32.dll`) TLS. |
| [`flux-browser`](./crates/flux-browser) | Multi-tab session and history controller, `flux-json` session export/import, DOM event restaging, Flux top bar UI, asynchronous background networking worker thread, Win32 window shell, and `flux` CLI binary. |
| [`flux-compress`](https://github.com/buntatoes/flux-compress) | Standalone DEFLATE (RFC 1951), `zlib` (Adler-32), and `gzip` (CRC-32) compression and decompression library. |
| [`flux-image`](https://github.com/buntatoes/flux-image) | Standalone PNG, BMP, and Netpbm (PPM/PGM) image decoder, encoder, and bilinear resampler. |
| [`flux-json`](https://github.com/buntatoes/flux-json) | Standalone zero-dependency RFC 8259 JSON parser, value tree, and compact/pretty serializer. |
| [`flux-ttf`](https://github.com/buntatoes/flux-ttf) | Standalone TrueType/OpenType SFNT font table parser (`head`, `hhea`, `maxp`, `cmap`, `hmtx`, `kern`) and proportional text metrics engine. |
| [`flux-gpu`](https://github.com/buntatoes/flux-gpu) | Standalone 2D signed-distance-field (SDF) quad compositor, shelf-packing RGBA texture atlas, and HLSL Shader Model 5.0 pipeline. |

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
| Click `<` / `>` / `R` | Navigate back, navigate forward, or reload the active page |
| Click address field or `F6` | Focus the address field to type a URL or file path (`Enter` to navigate, `Esc` to cancel) |
| Click page element / link | Dispatch script `click` listeners, or follow `<a href="...">` link (`http://`, `https://`, or relative file) |
| Mouse wheel / `Up` / `Down` / `PageUp` / `PageDown` | Scroll the active page vertically |
| `Backspace` | Navigate back in tab history (when address field is not focused) |
| `F5` | Reload the active page |
| `Esc` | Cancel address editing, or close the window |

---

## License

Proprietary — All Rights Reserved. See [`LICENSE`](./LICENSE) for details.

## Author

**Buntos**
