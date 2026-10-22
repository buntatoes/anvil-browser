# flux-browser v.01

flux-browser is a browser. Its page engine reads HTML, applies a style sheet, lays out boxes, and builds a display list.

Nodes live in one list. Parent and child links are ids into that list.

## Crates

- `flux-dom` holds the document tree and the HTML parser.
- `flux-style` reads style sheets and computes a style for each node.
- `flux-layout` places block and inline boxes from those styles.
- `flux-paint` turns the box tree into a display list.
- `flux-net` reads URLs and fetches cleartext HTTP.
- `flux-browser` keeps tabs and history, and builds the `flux` program.

Scripts are stored and not run. Layout uses the viewport width. The viewport height is kept with the session. Vertical margins do not collapse. Text width is an estimate until a font shaper is wired in.

## Commands

```
cargo test
cargo run -p flux-browser -- render examples/hello.html
```

`render` also accepts an `http://` URL. `https://` is refused until the fetch path can speak TLS.

## Author

Buntos
