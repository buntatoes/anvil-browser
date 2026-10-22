# flux-browser v.02

`flux-browser` is a ground-up web browser and custom rendering engine written in Rust with a native Win32 window shell.

It parses HTML into an arena-allocated document tree, loads linked and embedded CSS stylesheets, computes cascaded styles, lays out block and inline boxes, emits a display list, rasterizes frames into a software RGBA framebuffer, and presents interactive multi-tab pages in a native window.

## Architecture

| Crate | Responsibility |
| :--- | :--- |
| `flux-dom` | Arena-backed document tree (`NodeId` indexing, cycle-safe mutations), HTML parser, and `<link rel="stylesheet">` discovery. |
| `flux-style` | CSS stylesheet parser, user-agent defaults, property inheritance, and specificity/source-order cascade. |
| `flux-layout` | Block and inline formatting context layout, word wrapping, and reverse-paint-order coordinate hit testing. |
| `flux-paint` | Converts the layout box tree into a flat `DisplayList` of rectangle and text commands. |
| `flux-raster` | Software RGBA framebuffer rasterizer with clipping, alpha blending, vertical scroll offset, built-in 5x7 ASCII bitmap font, GDI BGRA output, and binary PPM export. |
| `flux-net` | URL parser, relative HTTP/HTTPS/file URL resolver, HTTP/1.1 client with redirect following and chunked transfer decoding, and native Windows Schannel (`secur32.dll`) TLS. |
| `flux-browser` | Multi-tab session and history controller, Flux top bar UI (tab strip, Back/Forward/Reload controls, editable address field), Win32 window shell, and `flux` CLI binary. |

## Building and Testing

```powershell
cargo test
```

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

## Window Controls

| Input | Action |
| :--- | :--- |
| Click tab / `x` / `+` | Switch active tab, close tab, or open a new tab |
| Click `<` / `>` / `R` | Navigate back, navigate forward, or reload the active page |
| Click address field or `F6` | Focus the address field to type a URL or file path (`Enter` to navigate, `Esc` to cancel) |
| Click page link | Follow `<a href="...">` link (`http://`, `https://`, or relative file) |
| Mouse wheel / `Up` / `Down` / `PageUp` / `PageDown` | Scroll the active page vertically |
| `Backspace` | Navigate back in tab history (when address field is not focused) |
| `F5` | Reload the active page |
| `Esc` | Cancel address editing, or close the window |

## Author

Buntos
