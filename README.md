# flux-browser v.02

flux-browser is a browser. Its page engine reads HTML, applies a style sheet, lays out boxes, builds a display list, rasters it into an RGBA framebuffer, and presents it in a Win32 window.

Nodes live in one list. Parent and child links are ids into that list.

## Crates

- `flux-dom` holds the document tree and the HTML parser.
- `flux-style` reads style sheets and computes a style for each node.
- `flux-layout` places block and inline boxes from those styles and hit-tests coordinates against the box tree.
- `flux-paint` turns the box tree into a display list.
- `flux-raster` paints a display list and a built-in 5x7 ASCII bitmap font into an RGBA framebuffer.
- `flux-net` reads URLs, resolves relative references, and fetches cleartext HTTP.
- `flux-browser` keeps tabs, history, and scroll state, and builds the `flux` program and Win32 window shell.

Scripts are stored and not run. Vertical margins do not collapse. Text width is an estimate until a font shaper is wired in.

## Commands

```
cargo test
cargo run -p flux-browser -- render examples/hello.html
cargo run -p flux-browser -- raster examples/hello.html frame.ppm
cargo run -p flux-browser -- open examples/hello.html
```

`render`, `raster`, and `open` also accept an `http://` URL. `https://` is refused until the fetch path can speak TLS. Inside the window, click links to navigate, use the mouse wheel or arrow/page keys to scroll, press `Backspace` to go back in history, and press `Esc` to close.

## Author

Buntos
