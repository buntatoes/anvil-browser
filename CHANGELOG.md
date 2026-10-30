# Changelog

What changed in each Anvil release, newest first. Every entry comes from the commits in this repo.

Anvil has two release lines. `main` holds the latest stable release (`v.24.3` right now). `testing` is a separate line where the next release gets built and tried first, so `testing` can be ahead of `main` and its version isn't on `main` until it ships there. Version numbers read like `v.24.4`, and the crates use `0.24.4`.

The engine and the nine companion libraries are their own repos with their own version numbers, while the `Prism` (`engines/prism`) and `Cinder` (`engines/cinder`) engines live in-tree inside this repository and link directly to the Rust core (`anvil-style` and `anvil-script`). When a release needs a newer sibling repo, the lock file here moves with it, and the entry says so.

## v.24.4

- **Official engine identities (`Forge`, `Cinder`, and `Prism`)**: Renamed the rendering engine to **Forge** (`anvil-engine`, `RENDERING_ENGINE_NAME = "Forge"`, `forge_raster_accel`), the in-tree JavaScript bytecode VM and JIT to **Cinder** (`engines/cinder`, `JS_ENGINE_NAME = "Cinder"`, `CinderVM`, `CinderDomHost`, `cinder-run`), and the in-tree CSS engine to **Prism** (`engines/prism`, `CSS_ENGINE_NAME = "Prism"`, `prism_css_engine`), surfaced across the UI, `anvil://` dashboards, and `F12` DevTools.
- **Anti-aliased `4x4` SSAA icon pipeline and multi-level mipmap downsampling (`anvil-image` 0.8.0, `logo.rs`, `ui.rs`, `ui/anvil_ui_d2d.cpp`)**: Added `downsample_2x2` and `resize_rgba_mipmapped` in `anvil-image` 0.8.0 (`v.08`), upgraded `render_anvil_logo` and `render_search_engine_logo` with `4x4` SSAA supersampling and multi-level mipmap downsampling, upgraded `anvil://logo.png` to `192x192` and `anvil://engine-*.png` to `96x96`, and eliminated the 5-rectangle cutout seam around the top-left tab strip icon in `ui/anvil_ui_d2d.cpp`.
- **End-to-end GPU hardware acceleration (`anvil-gpu` 0.11.0, `anvil-raster` 0.24.4, `present.rs`, `shell.rs`)**: Added rounded-corner SDF gradient evaluation in `PSGradient` (`shaders/passes.hlsl` and `shaders/passes.glsl`), hardware-accelerated rounded `SolidRect` and `RoundedGradient` execution in `execute_batch_on_pass_device`, Direct3D 11 staging texture pooling in `sys_d3d.rs`, `GpuSceneBatch` routing for rounded/bordered `LinearGradientRect` in `anvil-raster`, and zero-copy + `D3D11_BOX` dirty-row partial SwapChain presentation (`present.rs` & `shell.rs`).
- **Royalty-free open media codecs (`Opus` RFC 6716, `VP8` RFC 6386, `VP9`, and `AV1` OBU/CDEF in `anvil-video` 0.9.0)**: Added 100% clean-room specification encoders, decoders, and bitstream parsers in `anvil-video` 0.9.0 (`v.09`) for **Opus** (`RFC 6716` TOC + range coder + CELT/SILK synthesis), **VP8** (`RFC 6386` keyframe `[0x9D, 0x01, 0x2A]` + BoolDecoder + 4x4 WHT/IDCT + loop filter), **VP9** (`[0x49, 0x83, 0x42]` sync code + superframe index), and **AV1** (`AOMedia OBU` LEB128 + CDEF directional filter), wired into `GpuVideoSession::decode_video_auto` and `WebCodecsVideoDecoder` / `WebCodecsVideoEncoder`.
- **Responsive 1080p, 1440p, and 4K browser chrome and `anvil://` frontend redesign (`ui.rs`, `shell.rs`, `main.rs`, `frontend/`)**: Added monitor-aware default window sizing (`default_window_dimensions`) and first-launch high-DPI zoom (`125%` on 1440p, `150%` on 4K), adaptive tab strip widths, and responsive `@media (min-width: 1400px)`, `2000px`, and `3000px` multi-column card layouts across `anvil://home`, `anvil://settings`, `anvil://security`, and `anvil://inspector`.
- **Multi-font typography, `@font-face`, and text decoration pipeline (`anvil-ttf` 0.10.0, `anvil-style`, `anvil-layout`, `anvil-paint`, `anvil-raster`)**: Added `FontFamilySlot` (`Sans`, `Serif`, `Mono`, `SansBold`, `SansItalic`), `load_system_font_slot`, and `rasterize_glyph_styled` (horizontal bold stem dilation and 12-degree italic shear) in `anvil-ttf` 0.10.0 (`v.10`). Added `@font-face` (`FontFaceRule`), `font-family` (`FontFamily::Sans`, `Serif`, `Mono`), `text-transform` (`uppercase`, `lowercase`, `capitalize`), `letter-spacing`, `word-spacing`, and `text-decoration: line-through` end-to-end across `anvil-style`, `anvil-layout`, `anvil-paint`, and `anvil-raster`.
- **Scrollable overflow containers (`overflow: auto | scroll`), per-side borders, `object-fit`, and live DOM/CSSOM geometry (`anvil-dom`, `anvil-style`, `anvil-layout`, `anvil-paint`, `anvil-script`, `anvil-browser`)**: Added per-element scroll offsets (`node_scroll_offset`, `set_node_scroll_offset`), `scrollable_overflow` layout containers (`find_scrollable_container`), mouse-wheel container scroll routing (`Session::scroll_at`), and 6px overlay scrollbar track/thumb painting. Added `object-fit` (`fill`, `contain`, `cover`, `none`, `scale-down`) image placement/clipping, per-side border colors (`border-top-color`, `border-right-color`, `border-bottom-color`, `border-left-color`), and `outline` / `outline-offset`. Wired live `getBoundingClientRect()`, `getComputedStyle(el, pseudoElt)` (including `::before` and `::after`), `offsetWidth`/`offsetHeight`, `clientWidth`/`clientHeight`, `scrollWidth`/`scrollHeight`, `scrollTop`/`scrollLeft`, and `scrollTo`/`scrollBy` to real `anvil-style` + `anvil-layout` geometry on `CinderDomHost`.
- **CSS visual effects and W3C stacking context order (`engines/prism`, `anvil-style`, `anvil-layout`, `anvil-paint`, `anvil-raster`)**: Added `conic-gradient(...)`, `repeating-linear-gradient(...)`, and `repeating-radial-gradient(...)` parsing and pixel interpolation, `backdrop-filter: blur(...)` and `filter` (`blur`, `brightness`, `contrast`, `grayscale`, `invert`, `sepia`, `saturate`) post-processing passes, and W3C stacking context painting order (`z-index < 0`, normal-flow children, `z-index: 0`, and `z-index > 0`).
- **HTML5 `<dialog>` & constraint validation, ES2018+ async generators, live observers, and `File`/`Clipboard` WebAPIs (`engines/cinder`, `anvil-script`)**: Added `async function*` and `for await...of` (`[Symbol.asyncIterator]`), live scroll/resize `IntersectionObserver` and `ResizeObserver` re-evaluation (`ScriptRuntime::notify_layout_and_scroll`), HTML5 `<dialog>` (`show()`, `showModal()`, `close()`, `returnValue`, `close` event), HTML5 Form Constraint Validation (`validity`, `validationMessage`, `willValidate`, `checkValidity()`, `reportValidity()`, `setCustomValidity()`, `invalid` event), `File`, `FileList`, `FileReader`, `DataTransfer`, and `navigator.clipboard`.
- **5-tab `F12` DevTools dock and GitHub Actions CI synchronization (`anvil-browser`, `anvil-engine`, `anvil-ttf`)**: Expanded the `F12` DevTools dock to five tabs (`Elements`, `Styles` with live inline CSS override input `Session::apply_inspected_style_override`, `Network` waterfall log `Session::active_network_log`, `Console` REPL, and `Storage` inspector), and updated `.github/workflows/ci.yml` across `anvil-browser`, `anvil-engine`, and `anvil-ttf` for in-tree `engines/prism` and `engines/cinder` layout.

## v.24.3

- **Multi-layer HTML5, CSS Syntax Level 3, and JavaScript error recovery**: Added active formatting element reconstruction (`<a>`, `<b>`, `<strong>`, `<i>`, `<em>`, `<u>`, `<s>`, `<code>`, `<small>`, `<mark>`, `<span>`, `<font>`) across misnested closing tags, table foster parenting (`<table>`/`<thead>`/`<tbody>`/`<tfoot>`/`<tr>` non-table content fostered before the table), implicit closure of `<p>`, `<h1>`–`<h6>`, `<li>`, `<dt>`/`<dd>`, `<option>`/`<optgroup>`, and `<td>`/`<th>`/`<tr>`, and foreign `<svg>`/`<math>` self-closing tags in `anvil-dom`. Added CSS declaration/at-rule fault isolation, `@supports` selector/declaration recovery, and recursive `var(--missing, fallback)` resolution in `anvil-style`. Added per-`<script>` fault isolation, lenient statement-level fallback (`recover_script_statements_for_cinder`), `window.onerror`, `ErrorEvent`, and `PromiseRejectionEvent` (`unhandledrejection`) in `anvil-script`.
- **In-tree `Cinder` engine ES2022–ES2024 standards expansion (`engines/cinder` & `anvil-script`)**: Added ES6+ Generators (`function*`, `yield`, `yield*`, `.next()`, `.return()`, `.throw()`), arbitrary-precision `BigInt` (`123n`, `BigInt()`, arithmetic/bitwise/comparison ops, `BigInt.asIntN`, `BigInt.asUintN`), ES Module `import`/`export`/`import.meta`/`import()` lowering and module registry, `Proxy` (`new Proxy`, `Proxy.revocable`) and `Reflect` (`get`, `set`, `has`, `deleteProperty`, `ownKeys`, `getOwnPropertyDescriptor`, `defineProperty`, `getPrototypeOf`, `setPrototypeOf`, `apply`, `construct`), `WeakMap`, `WeakSet`, `WeakRef`, `FinalizationRegistry`, `DataView`, `ArrayBuffer`, `TypedArray`s, `Symbol.for`/`Symbol.keyFor`, `Error(..., { cause })`, `AggregateError`, and `Intl` (`NumberFormat`, `DateTimeFormat`, `Collator`, `PluralRules`, `Segmenter`).
- **In-tree `Prism` engine, pseudo-elements, and layout standards compliance (`engines/prism`, `anvil-style`, `anvil-layout`)**: Added `::before` and `::after` pseudo-element cascade and generated content layout boxes (`content: "..." | attr(...)`), universal CSS Values & Units Level 4 (`em`, `rem`, `%`, `vw`, `vh`, `vmin`, `vmax`, `ch`, `calc()`, `min()`, `max()`, `clamp()`) across `margin`, `padding`, `border-width`, `border-radius`, `font-size`, and `line-height`, `radial-gradient(...)` parsing and interpolation, `text-overflow: ellipsis` and `overflow-wrap: break-word` / `word-break: break-all` grapheme wrapping, and `<table>` intrinsic column sizing with a multi-row `rowspan` / `colspan` occupancy grid.
- **DOM & WebAPI standards compliance (`anvil-dom` & `anvil-script`)**: Upgraded `Document::query_selector`, `query_selector_all`, `matches`, and `closest` to full Selectors Level 4 (`>`, `+`, `~`, `:is()`, `:where()`, `:not()`, `:has()`, `:nth-child(An+B)`, `:first-child`, `:last-child`, `:empty`, `:checked`, `:disabled`, `[attr^=...]`, `[attr$=...]`, `[attr*=...]`). Added W3C 3-phase DOM event propagation (`Capture -> Target -> Bubble`) with `once`, `capture`, `passive`, `signal`, `stopImmediatePropagation()`, and `composedPath()`, complete `Element`/`ChildNode`/`ParentNode` mutation methods (`prepend`, `append`, `before`, `after`, `replaceWith`, `replaceChildren`, `toggleAttribute`, `getAttributeNames`, `hasAttributes`, `insertAdjacentElement`, `insertAdjacentText`), `TreeWalker`, `NodeIterator`, `Range`, `Selection`, `fetch` + `Headers` + `Request` + `Response` + `AbortController`/`AbortSignal`, `history.pushState`/`replaceState`/`popstate`, `crypto.getRandomValues`/`randomUUID`/`subtle.digest('SHA-256')`, `performance.mark`/`measure`, and 2D `<canvas>` state stacks (`save`/`restore`, transforms, `quadraticCurveTo`, `bezierCurveTo`, `createLinearGradient`, polygon `fill()`, and real RGBA `getImageData`/`putImageData`).
- **Multi-language GitHub Linguist registration, built-in `.css` stylesheets, and C++20 CSS engine kernels**: Added `.gitattributes` in `anvil-browser` and `anvil-engine`, extracted the WebAPI/DOM JavaScript bootstrap to `engines/cinder/js/src/runtime/dom_web_bootstrap.js`, added built-in user-agent and UI stylesheets (`engines/prism/stylesheets/*.css` and `frontend/styles/*.css`), and expanded the native C++20 CSS engine (`engines/prism/cpp/prism_css_engine.cpp`) with selector specificity, `An+B` matcher, sRGB/linear/OKLab/Display-P3 color conversion, cubic-bezier Newton-Raphson solver, and CSS `calc()` bytecode evaluator kernels.

## v.24.2
Desktop UI engine, in-tree CSS and JS engines, and internal pages:

- Replaces the 8x8 bitmap chrome font and Win32 GDI shell chrome with a native C++20 desktop UI engine (`ui/anvil_core_abi.h`, `ui/anvil_ui_d2d.cpp`, `ui/anvil_ui_window.cpp`, linked via `anvil_ui_shell.lib` with `d2d1`, `dwrite`, `dwmapi`, `gdi32`, `user32`, `ole32`) using hardware-accelerated Direct2D primitives, DirectWrite ClearType proportional typography (`Segoe UI`, `Segoe UI Semibold`, `Cascadia Mono` / `Consolas`), and Win32/DWM frameless window controls (`anvil_ui_configure_dark_titlebar`, `anvil_ui_extend_frame_into_client_area`, `anvil_ui_begin_window_drag`, `anvil_ui_window_action`).
- Integrates `Prism` (`engines/prism`) and `Cinder` (`engines/cinder`) directly into `anvil-browser/engines/` as non-crate in-tree engines with zero `Cargo.toml` manifests, compiled into static C++ libraries (`prism_css_engine.lib` and `cinder_jit_accel.lib`) and mounted directly into `anvil-style` (`pub mod prism`) and `anvil-script` (`pub mod cinder`) via `build.rs` and `#[path]`.
- Redesigns the top browser chrome in `ui.rs` and `shell.rs`: a 38px integrated top tab strip (`TAB_STRIP_HEIGHT = 38.0`) with Anvil emblem, curved tab shoulders, `+` New Tab button, draggable titlebar area (`UiHit::TitlebarDrag`), and Windows 11 Minimize/Maximize/Close caption buttons (`UiHit::WindowMinimize`, `UiHit::WindowMaximize`, `UiHit::WindowClose`); a 42px navigation bar with pill-shaped Omnibox (`radius = 14.0px`), inline Security badge, domain-highlighted URL (`split_url_for_domain_highlight` rendering scheme/path muted and the registrable domain bold/bright), inline Shields counter pill (`UiHit::ShieldsButton`), Clear `×` button, and Bookmark Star; and a floating bottom-left link-hover/loading status pill replacing the permanent bottom status bar (`STATUS_BAR_HEIGHT = 0.0`).
- Redesigns all built-in `anvil://` pages (`frontend/styles/browser.css` and `frontend/components/{StartDashboard,SettingsPage,SecurityCenter,DownloadsPage,HistoryPage,InspectorPage}.tsx` plus `frontend.rs`): `anvil://home` features a centered hero search card, responsive Speed-Dial tile grid, and 4-card Shields telemetry bar; `anvil://settings`, `anvil://security` (`anvil://shields`), `anvil://downloads`, `anvil://history`, and `anvil://inspector` share a 2-column left sidebar navigation rail (`230px` fixed rail) and structured content cards.
- Uses `anvil-engine` 0.24.2 (`v.24.2`), which links the in-tree `engines/prism` and `engines/cinder` engines directly into `anvil-style` and `anvil-script` and releases idle C++ `CinderVM` allocations (`VmOwner::release`) when page script execution finishes.

## v.24.1

CSS, JavaScript, typography, and capacity:

- Uses `prism` 0.4.0 (`v.04`), which adds 10x input/token/rule/selector/value/recursion limits (`320 MiB` input cap, `80,000,000` tokens, `10,000,000` rules in `prism`, and `MAX_STYLESHEET_BYTES = 160 MiB` in `anvil-style`), audited `unsafe` ASCII fast-path scanning, 421 recognized CSS properties, CSS Values Level 4 math functions (`round`, `mod`, `rem`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `pow`, `sqrt`, `hypot`, `log`, `exp`, `abs`, `sign`) and constants (`pi`, `e`, `infinity`, `nan`), CSS Color Level 4/5 `color(display-p3 | xyz-d50 | xyz-d65)`, `color-mix(in srgb | oklab | oklch | hsl | display-p3, ...)`, `light-dark()`, and typed `@property`/`@scope`/`@starting-style` at-rules.
- Uses `cinder` 0.7.0 (`v.07`), which links a full Rust register-bytecode JavaScript engine (`value`, `gc`, `bytecode`, `compiler`, `vm`, `stdlib`, `extra`, `regex`) with 64-bit NaN-boxed values, transition-tree hidden classes (`Shape`), 4-way polymorphic inline caches (`IcEntry`), mark-sweep garbage collection (`Heap`), microtask job queue (`drain_jobs`), AST syntax lowering (`compiler::lower` for ES2022–ES2024 classes, `extends`/`super`, `static` fields/blocks, `#private` fields/methods, array/object destructuring, tagged templates, `async`/`await`, and `/regex/` literals), and host-binding API (`Host`, `CinderDomHost` in `anvil-script`), raises compiler/parser/module limits 10x (`DEFAULT_MAX_SOURCE_BYTES = 640 MiB` and `DEFAULT_MAX_DEPTH = 10,000` in the Rust VM; `MAX_SOURCE_BYTES = 80 MiB`, `MAX_TOKENS = 5,242,880`, `MAX_AST_NODES = 2,621,440` in the self-hosted JS compiler), adds audited `unsafe` ASCII identifier/number scanning, ES2023/ES2024 builtins (`toSorted`, `toReversed`, `toSpliced`, `with`, `findLast`, `findLastIndex`, `flat`, `flatMap`, `at`, `Object.groupBy`, `Map.groupBy`, `Promise.withResolvers`, `structuredClone`), and Web/DOM classes (`DOMRect`, `DocumentFragment`, `DOMParser` with nested HTML fragment parsing and host DOM adoption, `FormData`, `Blob`, `TextEncoder`, `TextDecoder`, `Performance`, `BroadcastChannel`, `MessageChannel`, `CSSStyleSheet` with `replaceSync()` and `document.adoptedStyleSheets` host stylesheet sync).
- Uses `anvil-ttf` 0.9.0 (`v.09`), which raises font limits 10x (`MAX_FONT_BYTES = 320 MiB`, `MAX_SFNT_TABLES = 1,280`, `MAX_GLYPH_CONTOURS = 2,560`, `MAX_CONTOUR_POINTS = 40,960`) and adds vertical stem grid fitting (`snap_vertical_grid_y`), horizontal subpixel phase positioning (`rasterize_glyph_subpixel_positioned`), luminance-aware stem contrast enhancement (`enhance_stem_contrast`), and gamma-correct linear-space alpha blending (`srgb_to_linear_u12`, `linear_u12_to_srgb`, `blend_glyph_pixel_gamma_correct`, `blend_subpixel_lcd_gamma_correct`).
- Uses `anvil-engine` 0.24.1 (`v.24.1`), which wires gamma-correct linear-space glyph blending (`Framebuffer::set_glyph_pixel_gamma_correct`) and 4-bin (`0.25 px`) subpixel phase glyph caching (`cached_rasterize_glyph_phased`) with pairwise kerning into `anvil-raster`, implements the full CSS cascade in `anvil-style` (CSS Nesting `&` rule flattening, interactive/validation pseudo-classes `:hover`, `:active`, `:focus`, `:focus-visible`, `:focus-within`, `:target`, `:target-within`, `:valid`, `:invalid`, `:in-range`, `:out-of-range`, `@layer` cascade layers with statement/block/nested/anonymous ordering, `!important` layer inversion, and `revert-layer`, `@scope (<root>) to (<limit>)` scoped cascade boundaries with ancestor hop proximity and `:scope` root matching, `@property` custom property registration, and CSS-wide keywords `initial`/`inherit`/`unset`/`revert`/`revert-layer` plus `all`), `@container` size queries (`min-width`, `max-width`, and range comparisons), CSS Values Level 4 math evaluation (`round()`, `mod()`, `rem()`, `sin()`, `cos()`, `tan()`, `pow()`, `sqrt()`, `hypot()`, `abs()`, `sign()`), CSS Logical Properties (`inline-size`, `block-size`, `min-inline-size`, `max-inline-size`, `min-block-size`, `max-block-size`, `padding-inline*`, `padding-block*`, `margin-inline*`, `margin-block*`, `inset-inline*`, `inset-block*`, `border-inline*`, `border-block*`), standalone `translate`/`scale`/`rotate` transform properties, and `@starting-style`, completes Flexbox (`row`, `row-reverse`, `column`, `column-reverse`, `wrap`, `wrap-reverse`, `flex-flow`, `order`, `align-content`, `align-items: baseline`, iterative `min`/`max` freeze clamping, 4-edge `auto` margins) and CSS Grid (`grid-template-areas`, `grid-auto-flow: row | column | row dense | column dense`, `grid-auto-columns` / `grid-auto-rows` multi-track cycling, `min-content` / `max-content` / `fit-content()`, `justify-items` / `justify-self` / `align-content` / `place-*`, 4-edge `auto` margins) in `anvil-layout`, connects 2D `<canvas>`, timers, observers, `DOMParser` subtree adoption, and `document.adoptedStyleSheets` (`CSSStyleSheet.replaceSync()`) to `CinderDomHost` and `anvil-script`'s live DOM bridge, and raises `CinderVM` (`256 MiB` heap budget, `8,192` bytecode instructions, `1,280` DOM IC slots) and WebAssembly (`160 MiB` / `2,560` pages, `5,000,000` steps, `640` call depth) limits in `anvil-script`.
- Updates the embedded Anvil brand logo (`assets/anvil-mark.png`, `logo.rs`, `anvil://logo.png`, top-bar icon, and Windows `.ico` icon), raises `anvil-net` response limits 10x (`MAX_RESPONSE = 80 MiB`, `MAX_DECOMPRESSED_BODY = 160 MiB`, `MAX_HEADER_BYTES = 640 KiB`, `MAX_HEADER_COUNT = 1,280`), raises `MAX_IMAGE_CACHE_ENTRIES = 2,560`, and enables Per-Monitor High-DPI V2 awareness (`SetProcessDpiAwarenessContext` / `WM_DPICHANGED`) on Windows.

## v.24

Shields and adblocking:

- Directly links and runs Brave Browser's unmodified open-source `adblock-rust` crate (`adblock = { path = "../adblock-rust" }`, MPL-2.0) in `anvil-net` and `anvil-browser`.
- Evaluates network requests against `adblock::Engine` (`adblock::lists::FilterSet`, `adblock::request::Request`, `adblock::blocker::BlockerResult`) with Adblock Plus / uBlock Origin filter rules, `@@` exception rules, `$important`, `$third-party`, `$script`/`$image`/`$stylesheet`/`$xmlhttprequest` resource-type masks, and `$domain=...` initiator scoping.
- Web-accessible `$redirect` surrogate execution (`adblock::resources::Resource` for `noop.js`, `noop.css`, `noop.html`, `noop.txt`, `google-analytics_analytics.js`, `googletagmanager_gtm.js`, `googlesyndication_adsbygoogle.js`, `scorecardresearch_beacon.js`) for both in-process `<script src>` loads and `--renderer` tab worker subresource fetches.
- Automatic `$removeparam` tracking query parameter stripping (`utm_source`, `utm_medium`, `utm_campaign`, `utm_term`, `utm_content`, `fbclid`, `gclid`, `msclkid`, `mc_cid`, `mc_eid`, `_hsenc`, `_hsmi`, `yclid`, `twclid`, `igshid`) on navigation via `adblock::Engine::check_network_request`.
- Cosmetic element hiding (`adblock::Engine::url_cosmetic_resources` and `hidden_class_id_selectors` for generic `##.selector` and `###id`, domain-scoped `example.com##.selector`, `#@#` unhide exceptions, and `:style(...)` rules) injected into the page stylesheet cascade during `render_document`.
- Per-site Shields Up/Down toggle (`toggle_shields_for_site`), global Shields modes (`Standard`, `Aggressive`, `Disabled`), custom filter rule addition (`add_shield_filter_rule`), and interactive `anvil://shields` (`anvil://security`) dashboard controls.

## v.23.3.2

Window, tabs, and UI:

- Cached page framebuffer compositing: toolbar/menu hovers and address bar typing reuse the last rendered page bitmap (`Host::cached_page_fb` and `Framebuffer::copy_rows_from`) instead of re-rasterizing the document on every mouse move.
- Four built-in themes (`Dark`, `Midnight`, `Light`, `Nordic`), synced across the window chrome, Win32 dark title bar (`DWMWA_USE_IMMERSIVE_DARK_MODE`), `profile.anvil`, and all `anvil://` pages.
- Floating Omnibox suggestion popover that ranks built-in `anvil://` routes, bookmarks, history, and search queries as you type, with `Up`/`Down` arrow selection, `Tab`/`Enter` completion, and click navigation.
- Pinned tabs (`[P]` badge, close protection, and pinned-prefix ordering), tab reordering (`Ctrl+Shift+PageUp`/`PageDown`), `Close Other Tabs`, closed-tab restore (`Ctrl+Shift+T`), tab duplication, middle-click tab close and link opening, and `Ctrl+Tab`/`Ctrl+Shift+Tab`/`Ctrl+1..9` tab switching.
- Full caret and word-level text editing (`Left`, `Right`, `Home`, `End`, forward `Delete`, `Ctrl+Backspace`, `Ctrl+A`, and visible mid-string `|` caret) across the address bar, find bar, DevTools console input, and page `<input>`/`<textarea>` controls.
- Mouse-drag text selection across page text runs with translucent highlight overlays and `Ctrl+C` copy.
- Right-click context menu with link actions (`Open Link in New Tab`, `Copy Link Address`), tab actions (`Pin / Unpin Tab`, `Duplicate Tab`, `Close Other Tabs`, `Reopen Closed Tab`), `Save Page HTML (Ctrl+S)`, and `View Page Source (Ctrl+U)` opening a line-numbered `view-source:` view.
- Live `F12` DevTools dock (`Elements`, `Styles`, and `Console`): clicking or right-clicking on the page inspects the live DOM node, box dimensions, and computed styles, and the `Console` tab provides an interactive JS prompt that evaluates expressions against the active page and restages DOM changes.
- Automatic downloads for `Content-Disposition: attachment` and binary responses, `Ctrl+S` page saving, and one-click disk export on `anvil://downloads`.
- Settings categories (`anvil://settings?section=...`), page zoom buttons (`75%`–`150%`), automatic profile persistence, search engine switcher pills and activity cards on `anvil://home`, and a live filter bar on `anvil://history`.

Page layout, forms, and navigation:

- Uses `anvil-engine` 23.3.2, which adds CSS `aspect-ratio` box sizing, multi-line `<pre>` and `<textarea>` formatting, `<details>`/`<summary>` disclosure accordions, `<input type="radio">` groups, `<label>` click forwarding, `Tab`/`Shift+Tab` form field focus cycling, floating `<select>` option popovers, interactive `<input type="range">` sliders, and `<progress>`/`<meter>` bars.
- Restores each page's scroll offset across `Back`, `Forward`, and tab switches, and offsets `#fragment` jumps when a page has a fixed or sticky top header.
- Formats Bing Search result pages alongside Google Search and DuckDuckGo Search.

## v.23.3.1

JavaScript, DOM, and layout:

- Uses `cinder` 0.5.0 and `anvil-engine` 23.3.1: ES2022+ DOM scripts run on `CinderVM` with a seeded host DOM tree (`seedHostNode`/`seedHostAttr`), live DOM/storage/location mutation sync (`exportDomMutationRows`), and interactive `click`, `input`, `submit`, and `keydown` event dispatch (`dispatchHostEvent`) with `event.preventDefault()`.
- `MutationObserver`, `IntersectionObserver`, and `ResizeObserver` callbacks run on the microtask queue.
- Inline `<svg>` icons and vector paths (`<path>`, `<rect>`, `<circle>`, `<ellipse>`, `<line>`, `<polyline>`, `<polygon>`, `<g>`, `viewBox`, `currentColor`) are rasterized in `anvil-layout`, alongside `line-height`, `colspan` table cells, and decorated inline badges (`<code>`, `<mark>`).
- Default page rasterization coalesces contiguous rects, rounded boxes, shadows, gradients, clips, and fixed-scroll offsets into batched `GpuSceneBatch` passes.
- In-page `#fragment` anchor scrolling (`id`, `<a name>`, `#top`) and DuckDuckGo search result formatting.

Networking:

- Origin-keyed connection pooling for HTTP/1.1 keep-alive sockets and persistent multi-stream HTTP/2 sessions, plus `fetch_batch` multiplexing for same-origin stylesheets and images.

## v.23.3

GPU, media, CSS, and licensing:

- Uses `anvil-gpu` 0.10.0 (`GpuSceneBatch` and `I420Surface`), `anvil-video` 0.8.0 (`ADCT` 8x8 integer DCT/IDCT codec, `FLAC`/`OpusMdct` audio, and H.264/AV1/VP9/fMP4/WebM container framing), `prism` 0.3.0 (Selectors Level 4 `:is()`, `:where()`, `:has()`, `:not()`, `:host`, `@supports`, and CSS Color 4), `cinder` 0.4.0 (2D `<canvas>` and WebCodecs/MSE helpers), and `anvil-engine` 0.23.3 (`<noscript>` isolation, `box-sizing: border-box`, flex/grid alignment, and live DOM script bridge).
- Dual-licensed across all 13 repositories under the PolyForm Noncommercial License 1.0.0 and a Paid Commercial License with a Non-Compete Clause (`LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial`).

## v.23.2

JavaScript and CSS:

- Self-hosted `cinder` (0.2.0) ES2022+ JavaScript compiler, lowering pass, standard library, and Web/DOM runtime (`AnvilPromise`, `AnvilURL`, `AnvilURLSearchParams`, `AnvilDOMNode`, `AnvilDOMDocument`, `AnvilCustomElementRegistry`, `AnvilMutationObserver`, `AnvilIntersectionObserver`, `AnvilResizeObserver`, `AnvilMediaSource`, `AnvilAbortController`, `AnvilHeaders`, `AnvilRequest`, `AnvilResponse`, `AnvilHistory`, `AnvilLocation`, `AnvilStorage`) written in Core JS and executing on the C++20 `CinderVM` register bytecode VM when a C++20 compiler is found at build time (`compile_es2022_with_cinder`, `eval_es2022_with_cinder`, `eval_webapp_with_cinder`).
- `prism` (0.1.0) CSS Syntax Level 3 tokenizer, Selectors Level 4 matcher, 261-property value/color/`calc()` parser, and media/container/`@supports` evaluator, published as its own library. `anvil-style` didn't use it yet in this release.

Style sheets:

- `<style>` blocks and `<link rel="stylesheet">` sheets now apply in the order they appear in the page. Before, every linked sheet was put ahead of every `<style>` block, so a link written later lost ties it should have won.
- Each sheet is parsed on its own, so a broken sheet no longer swallows the one after it.
- A sheet's `media` attribute works the same way as `@media`, so a print sheet doesn't apply on screen.
- Restyling a page now respects a policy that blocks inline style, the same as the first render.
- The page's CSS is parsed once and only parsed again when a sheet changes. Animation frames and window resizes reuse it.
- Uses `anvil-engine` 0.23.2, which honours `!important`, drops a whole rule when one selector in its list is invalid, reads media types, `not`, `only` and comma lists in `@media`, skips strings, escapes and unquoted `url()` bodies when scanning a sheet, and lists sheets in document order with their media.

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

- The HTTP/2 `RST_STREAM` handling and `anvil-video` used an `if let` form that older toolchains can't compile. Same behaviour, plain nested `if`s. Uses `anvil-video` 0.7.1, which has that fix. (The oldest supported Rust moved to 1.99.1 later in this release; see below.)
- Tests run one at a time. Opening a session caps the test process at 2 GiB of address space, the same cap the window gets, and with tests running side by side a test could sometimes fail with "failed to spawn thread".
- CI now clones `anvil-pqc` too, which it was missing, and also runs on pushes to `testing`.
- `Cargo.toml` says `rust-version`, every crate links the repo, and the C++20 parts are marked optional.
- The oldest Rust it builds with is now 1.99.1, up from 1.85.1. `rust-version`, the README and the CI job that builds on the oldest toolchain all say 1.99.1, and the engine and the nine libraries moved up with it. Clippy's suggestions that need the newer Rust, like `if let` chains, are now used where it asked for them. Uses `anvil-cipher` 0.2.1, `anvil-compress` 0.3.1, `anvil-gpu` 0.9.1, `anvil-hpack` 0.1.2, `anvil-image` 0.7.1, `anvil-json` 0.3.2, `anvil-pqc` 0.2.0, `anvil-ttf` 0.8.1 and `anvil-video` 0.7.2, which ask for 1.99.1 too.

Docs:

- The README and SECURITY.md now say what page loads really use: OpenSSL on Linux or Schannel on Windows, one connection per request. The in-house TLS 1.3 code, the multi-stream HTTP/2 session and the connection pool are in `anvil-net` but page loads don't use them yet.
- The `MutationObserver`, `IntersectionObserver` and `ResizeObserver` objects are described as placeholders that never call back.
- The line count badge is up to date, the two badges that CI didn't back up are gone, and a live CI badge shows how `testing` is doing.
- Added this changelog.

## v.23.1

- A WebAssembly 1.0 runtime for pages, with the `WebAssembly` namespace, typed array views, `ArrayBuffer`, `TextEncoder`, `TextDecoder`, `atob` and `btoa`, `URLSearchParams` and `DOMParser`.
- Click handlers on pages loaded in a tab worker are hooked back up in the window.
- `anvil-net` gained an in-house TLS 1.3 handshake and record layer built on `anvil-pqc` and `anvil-cipher`, a multi-stream HTTP/2 session and a connection pool. These aren't on the page load path yet.
- Windows offers HTTP/2 through ALPN too, and shows the key exchange group of recent connections on `anvil://security`.
- Subresource integrity accepts `sha3-256-` hashes as well as `sha256-`.
- Uses `anvil-pqc` 0.1.1.
- Added the full license texts and a total line count badge.

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
- Added repository license files across the browser, libraries, and engine.

## v.21

- CSS custom properties (`var()`), `calc()` lengths and `display: grid`.
- More of the script interpreter: newer syntax, `try`/`catch` recovery, and DOM and CSSOM changes from script.
- Uses `anvil-ttf` v.07 for font subroutines.

## v.20

- A numeric loop that sets `textContent` stays in machine code, with the page write going back through the interpreter.
- A parallel strip rasterizer, a dirty-tile grid, wider span fills and a three-pass box blur.
- CFF fonts (`anvil-ttf` v.06) and an audio output bridge (`anvil-video` v.07) in the video player.
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

- Linux profiles are sealed with AES-256-GCM from `anvil-cipher`. The key file stays with its profile directory, and a bad tag is rejected.
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
