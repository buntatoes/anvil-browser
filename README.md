# flux-browser

[![Release](https://img.shields.io/badge/release-v.02-e8c37a)](https://github.com/buntatoes/flux-browser)
[![License: Proprietary](https://img.shields.io/badge/license-Proprietary-8b3a3a.svg)](./LICENSE)
[![Edition](https://img.shields.io/badge/rust-2024_edition-orange.svg)](https://github.com/buntatoes/flux-browser)

`flux-browser` is a ground-up web browser and custom rendering engine written in Rust with a native Win32 window shell.

It parses HTML into an arena-allocated document tree, loads linked and embedded CSS stylesheets, computes cascaded styles (tag, `.class`, `#id`, compound, and descendant selectors plus inline `style` attributes), lays out block, inline, and Flexbox containers, executes `<script>` blocks and DOM event listeners, emits a display list, rasterizes frames with anti-aliased TrueType font rendering into a software RGBA framebuffer, and presents interactive multi-tab pages with asynchronous networking in a native window.

---

## Rendering & Execution Pipeline

```text
URL / File Input
  -> flux-net     (HTTP/1.1, redirects, chunked transfer, Windows Schannel TLS)
  -> flux-dom     (Arena Document tree, HTML parser, <link> & <script> discovery)
  -> flux-script  (Script interpreter, DOM queries/mutations, click listeners)
  -> flux-style   (CSS parser, specificity cascade, inline styles, Flexbox & box model)
  -> flux-layout  (Block, Inline word-wrap, & Flexbox layout, hit-testing)
  -> flux-paint   (Flat DisplayList: backgrounds, borders, underlines, text runs)
  -> flux-raster  (RGBA framebuffer, Win32 GDI TrueType anti-aliased glyphs, PPM/BGRA)
  -> flux-browser (Multi-tab Session, top bar UI, async worker thread, Win32 shell)
```

---

## Workspace Architecture

| Crate | Responsibility |
| :--- | :--- |
| [`flux-dom`](./crates/flux-dom) | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations, DOM query/mutation helpers), HTML parser, `<link rel="stylesheet">` discovery, and `<script>` extraction. |
| [`flux-style`](./crates/flux-style) | CSS stylesheet parser (tag, `.class`, `#id`, compound, and descendant selectors), inline `style="..."` attributes, user-agent defaults, property inheritance, `(ids, classes, tags)` specificity cascade, box-model sides/borders, and Flexbox properties. |
| [`flux-layout`](./crates/flux-layout) | Block, inline, and Flexbox (`row` / `column`, `gap`, `justify-content`, `align-items`) formatting context layout, list markers (`<ul>` / `<ol>` / `<li>`), `<hr>` / `<img>` boxes, word wrapping, and reverse-paint-order coordinate hit testing. |
| [`flux-paint`](./crates/flux-paint) | Converts the layout box tree into a flat `DisplayList` of background, border, underline, and text commands. |
| [`flux-raster`](./crates/flux-raster) | Software RGBA framebuffer rasterizer with clipping, alpha blending, vertical scroll offset, native Win32 GDI TrueType anti-aliased glyph rasterizer (with built-in 5x7 ASCII bitmap fallback), GDI BGRA output, and binary PPM export. |
| [`flux-script`](./crates/flux-script) | Proprietary scripting interpreter and DOM bindings (`document.getElementById`, `document.querySelector`, `document.createElement`, `textContent`, `style.*`, `setAttribute`, `appendChild`, and bubbling `click` event listeners). |
| [`flux-net`](./crates/flux-net) | URL parser, relative HTTP/HTTPS/file URL resolver, HTTP/1.1 client with redirect following and chunked transfer decoding, and native Windows Schannel (`secur32.dll`) TLS. |
| [`flux-browser`](./crates/flux-browser) | Multi-tab session and history controller, DOM event restaging, Flux top bar UI (tab strip, Back/Forward/Reload controls, editable address field, loading indicator), asynchronous background networking worker thread, Win32 window shell, and `flux` CLI binary. |

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
