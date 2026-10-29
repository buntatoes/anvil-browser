// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
//! JavaScript engine (`anvil-js`) combining a Rust register-bytecode compiler,
//! 64-bit NaN-boxed value representation, hidden-class (`Shape`) and 4-way
//! polymorphic inline-cache (`IcEntry`) object model, mark-sweep garbage
//! collector (`Heap`), ES2022–ES2024 standard library (`vm::stdlib`, `vm::extra`),
//! and the 26-module self-hosted Core JS compiler and Web/DOM runtime (`js/src/`).
//!
//! ## Rust Bytecode Compiler, GC, and Virtual Machine
//! - [`lexer`] & [`parser`]: Bounded ES2022+ lexer and recursive-descent/Pratt
//!   parser producing [`ast::Program`].
//! - [`compiler`]: Two-pass lexical scope/capture analyzer ([`compiler::scope`])
//!   and register bytecode generator ([`compile_script`], [`compile_eval`]).
//! - [`value`] & [`gc`]: 64-bit NaN-boxed [`Value`], transition-tree hidden
//!   classes ([`value::shape::Shape`]), [`ObjectClass::Host`] wrapper handles,
//!   and mark-sweep [`gc::Heap`] with persistent and stack roots.
//! - [`bytecode`] & [`vm`]: 32-bit fixed-width register bytecode ([`bytecode::Op`]),
//!   [`Vm`] execution engine with 4-way polymorphic property inline caches,
//!   microtask job queue ([`Vm::drain_jobs`]), [`Host`] trait for embedding DOM
//!   bindings, and ES2022–ES2024 built-ins (`Object`, `Function`, `Array` including
//!   `toSorted`/`toReversed`/`toSpliced`/`with`/`findLast`/`findLastIndex`/`flat`/`flatMap`/`at`,
//!   `Object.groupBy`, `Map.groupBy`, `String`, `Number`, `Boolean`, `Math`, `JSON`,
//!   `Error`, `RegExp`, `Date`, `Map`, `Set`, `WeakMap`, `WeakSet`, `Symbol`,
//!   `Promise` with `Promise.withResolvers`, `Proxy`, `Reflect`, `ArrayBuffer`,
//!   `DataView`, `Uint8Array`, `Int32Array`, `Float64Array`, `structuredClone`).
//!
//! ## Self-Hosted Core JS Compiler & Web/DOM Runtime (`js/src/`)
//! The 26 modules in `js/MODULES.txt` also provide the self-hosted ES2022+ to
//! Core JS lowering pass (`#private` fields, `static {}` blocks, destructuring,
//! rest/spread, default parameters, logical assignment) and Web/DOM runtime
//! (`self_hosted_bundle`, `self_hosted_runtime_bundle`) used by `anvil-script`.

#![warn(missing_docs)]

use std::sync::OnceLock;

#[allow(dead_code, missing_docs)]
#[path = "../js/tools/corejs/mod.rs"]
pub mod corejs;

#[allow(dead_code, missing_docs)]
#[path = "../js/tools/bundle/src/link.rs"]
pub mod link;

#[allow(dead_code, missing_docs)]
#[path = "../js/tools/bundle/src/modules.rs"]
pub mod modules;

pub mod ast;
pub mod bytecode;
pub mod compiler;
pub mod error;
pub mod gc;
pub mod lexer;
pub mod limits;
pub mod parser;
pub mod value;
pub mod vm;

pub use compiler::{
    CompileError, compile_eval, compile_program, compile_script, install, run_script,
};
pub use error::{Span, SyntaxError};
pub use limits::{LimitKind, Limits};
pub use parser::{
    DEFAULT_MAX_DEPTH, DEFAULT_MAX_SOURCE_BYTES, ParseOptions, parse_module, parse_script,
};
pub use value::Value;
pub use vm::{BasicHost, CodeId, ErrorKind, Host, IcStats, Job, NativeCx, NativeFn, RunError, Vm};

/// Raw `js/MODULES.txt` manifest listing all self-hosted JavaScript modules in load order.
pub const MODULES_MANIFEST: &str = include_str!("../js/MODULES.txt");

/// Self-hosted Core JS standard library module (`js/src/runtime/builtins.js`).
pub const RUNTIME_BUILTINS_JS: &str = include_str!("../js/src/runtime/builtins.js");

/// Self-hosted Core JS JSON, Base64, and URI codec module (`js/src/runtime/json_codec.js`).
pub const RUNTIME_JSON_CODEC_JS: &str = include_str!("../js/src/runtime/json_codec.js");

/// Self-hosted Core JS Web APIs and DOM runtime module (`js/src/runtime/web_apis.js`).
pub const RUNTIME_WEB_APIS_JS: &str = include_str!("../js/src/runtime/web_apis.js");

/// Look up the embedded source of a module path from `js/MODULES.txt`.
pub fn embedded_module_source(path: &str) -> Option<&'static str> {
    match path {
        "src/core/limits.js" => Some(include_str!("../js/src/core/limits.js")),
        "src/core/unicode_tables.js" => Some(include_str!("../js/src/core/unicode_tables.js")),
        "src/core/text.js" => Some(include_str!("../js/src/core/text.js")),
        "src/core/positions.js" => Some(include_str!("../js/src/core/positions.js")),
        "src/core/errors.js" => Some(include_str!("../js/src/core/errors.js")),
        "src/syntax/tokens.js" => Some(include_str!("../js/src/syntax/tokens.js")),
        "src/syntax/numeric.js" => Some(include_str!("../js/src/syntax/numeric.js")),
        "src/syntax/lexer.js" => Some(include_str!("../js/src/syntax/lexer.js")),
        "src/syntax/lex_messages.js" => Some(include_str!("../js/src/syntax/lex_messages.js")),
        "src/syntax/token_dump.js" => Some(include_str!("../js/src/syntax/token_dump.js")),
        "src/syntax/words.js" => Some(include_str!("../js/src/syntax/words.js")),
        "src/syntax/ast.js" => Some(include_str!("../js/src/syntax/ast.js")),
        "src/syntax/names.js" => Some(include_str!("../js/src/syntax/names.js")),
        "src/syntax/early_errors.js" => Some(include_str!("../js/src/syntax/early_errors.js")),
        "src/syntax/parser.js" => Some(include_str!("../js/src/syntax/parser.js")),
        "src/syntax/parse_messages.js" => Some(include_str!("../js/src/syntax/parse_messages.js")),
        "src/syntax/ast_dump.js" => Some(include_str!("../js/src/syntax/ast_dump.js")),
        "src/syntax/scope.js" => Some(include_str!("../js/src/syntax/scope.js")),
        "src/syntax/scope_dump.js" => Some(include_str!("../js/src/syntax/scope_dump.js")),
        "src/syntax/scope_check.js" => Some(include_str!("../js/src/syntax/scope_check.js")),
        "src/lower/emit.js" => Some(include_str!("../js/src/lower/emit.js")),
        "src/lower/lower.js" => Some(include_str!("../js/src/lower/lower.js")),
        "src/runtime/builtins.js" => Some(RUNTIME_BUILTINS_JS),
        "src/runtime/json_codec.js" => Some(RUNTIME_JSON_CODEC_JS),
        "src/runtime/web_apis.js" => Some(RUNTIME_WEB_APIS_JS),
        "src/index.js" => Some(include_str!("../js/src/index.js")),
        _ => None,
    }
}

/// Load all self-hosted JavaScript engine modules listed in `js/MODULES.txt`.
pub fn load_embedded_modules() -> Result<Vec<link::Module>, String> {
    let entries = modules::parse_list(MODULES_MANIFEST)?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(src) = embedded_module_source(&entry.path) else {
            return Err(format!("missing embedded source for `{}`", entry.path));
        };
        out.push(link::Module {
            path: entry.path,
            level: entry.level,
            builtins: entry.builtins,
            src: src.to_string(),
        });
    }
    Ok(out)
}

const RUNTIME_PUBLIC_API_BRIDGE: &str = r#"
function createBrowserEnv(initialUrl) {
  return runtime_web_apis$createBrowserEnv(initialUrl);
}
class AnvilURLSearchParams extends runtime_builtins$AnvilURLSearchParams {
  constructor(init) {
    super(init);
  }
}
class AnvilURL extends runtime_builtins$AnvilURL {
  constructor(input) {
    super(input);
  }
}
class AnvilPromise extends runtime_builtins$AnvilPromise {
  constructor(executor) {
    super(executor);
  }
}
class AnvilEventTarget extends runtime_builtins$AnvilEventTarget {
  constructor() {
    super();
  }
}
class AnvilCustomEvent extends runtime_builtins$AnvilCustomEvent {
  constructor(type, options) {
    super(type, options);
  }
}
class AnvilHeaders extends runtime_web_apis$AnvilHeaders {
  constructor(init) {
    super(init);
  }
}
class AnvilRequest extends runtime_web_apis$AnvilRequest {
  constructor(input, init) {
    super(input, init);
  }
}
class AnvilResponse extends runtime_web_apis$AnvilResponse {
  constructor(body, init) {
    super(body, init);
  }
}
class AnvilAbortController extends runtime_web_apis$AnvilAbortController {
  constructor() {
    super();
  }
}
class AnvilMutationObserver extends runtime_web_apis$AnvilMutationObserver {
  constructor(callback) {
    super(callback);
  }
}
class AnvilIntersectionObserver extends runtime_web_apis$AnvilIntersectionObserver {
  constructor(callback, options) {
    super(callback, options);
  }
}
class AnvilResizeObserver extends runtime_web_apis$AnvilResizeObserver {
  constructor(callback) {
    super(callback);
  }
}
class AnvilMediaSource extends runtime_web_apis$AnvilMediaSource {
  constructor() {
    super();
  }
  static isTypeSupported(mime) {
    return runtime_web_apis$AnvilMediaSource.isTypeSupported(mime);
  }
}
class MediaSource extends runtime_web_apis$AnvilMediaSource {
  constructor() {
    super();
  }
  static isTypeSupported(mime) {
    return runtime_web_apis$AnvilMediaSource.isTypeSupported(mime);
  }
}
class AnvilImageData extends runtime_web_apis$AnvilImageData {
  constructor(a, b, c) {
    super(a, b, c);
  }
}
class ImageData extends runtime_web_apis$AnvilImageData {
  constructor(a, b, c) {
    super(a, b, c);
  }
}
class AnvilCanvasRenderingContext2D extends runtime_web_apis$AnvilCanvasRenderingContext2D {
  constructor(canvasNode) {
    super(canvasNode);
  }
}
class AnvilVideoFrame extends runtime_web_apis$AnvilVideoFrame {
  constructor(data, init) {
    super(data, init);
  }
}
class VideoFrame extends runtime_web_apis$AnvilVideoFrame {
  constructor(data, init) {
    super(data, init);
  }
}
class AnvilEncodedVideoChunk extends runtime_web_apis$AnvilEncodedVideoChunk {
  constructor(init) {
    super(init);
  }
}
class EncodedVideoChunk extends runtime_web_apis$AnvilEncodedVideoChunk {
  constructor(init) {
    super(init);
  }
}
class AnvilVideoEncoder extends runtime_web_apis$AnvilVideoEncoder {
  constructor(init) {
    super(init);
  }
  static isConfigSupported(cfg) {
    return runtime_web_apis$AnvilVideoEncoder.isConfigSupported(cfg);
  }
}
class VideoEncoder extends runtime_web_apis$AnvilVideoEncoder {
  constructor(init) {
    super(init);
  }
  static isConfigSupported(cfg) {
    return runtime_web_apis$AnvilVideoEncoder.isConfigSupported(cfg);
  }
}
class AnvilVideoDecoder extends runtime_web_apis$AnvilVideoDecoder {
  constructor(init) {
    super(init);
  }
  static isConfigSupported(cfg) {
    return runtime_web_apis$AnvilVideoDecoder.isConfigSupported(cfg);
  }
}
class VideoDecoder extends runtime_web_apis$AnvilVideoDecoder {
  constructor(init) {
    super(init);
  }
  static isConfigSupported(cfg) {
    return runtime_web_apis$AnvilVideoDecoder.isConfigSupported(cfg);
  }
}
class AnvilMediaRecorder extends runtime_web_apis$AnvilMediaRecorder {
  constructor(stream, opts) {
    super(stream, opts);
  }
  static isTypeSupported(mime) {
    return runtime_web_apis$AnvilMediaRecorder.isTypeSupported(mime);
  }
}
class MediaRecorder extends runtime_web_apis$AnvilMediaRecorder {
  constructor(stream, opts) {
    super(stream, opts);
  }
  static isTypeSupported(mime) {
    return runtime_web_apis$AnvilMediaRecorder.isTypeSupported(mime);
  }
}
class AnvilAudioContext extends runtime_web_apis$AnvilAudioContext {
  constructor() {
    super();
  }
}
class AudioContext extends runtime_web_apis$AnvilAudioContext {
  constructor() {
    super();
  }
}
class AnvilDOMRect extends runtime_web_apis$AnvilDOMRect {
  constructor(x, y, width, height) {
    super(x, y, width, height);
  }
}
class DOMRect extends runtime_web_apis$AnvilDOMRect {
  constructor(x, y, width, height) {
    super(x, y, width, height);
  }
}
class AnvilDocumentFragment extends runtime_web_apis$AnvilDocumentFragment {
  constructor(doc) {
    super(doc);
  }
}
class DocumentFragment extends runtime_web_apis$AnvilDocumentFragment {
  constructor(doc) {
    super(doc);
  }
}
class AnvilDOMParser extends runtime_web_apis$AnvilDOMParser {
  constructor() {
    super();
  }
}
class DOMParser extends runtime_web_apis$AnvilDOMParser {
  constructor() {
    super();
  }
}
class AnvilFormData extends runtime_web_apis$AnvilFormData {
  constructor() {
    super();
  }
}
class FormData extends runtime_web_apis$AnvilFormData {
  constructor() {
    super();
  }
}
class AnvilBlob extends runtime_web_apis$AnvilBlob {
  constructor(parts, opts) {
    super(parts, opts);
  }
}
class Blob extends runtime_web_apis$AnvilBlob {
  constructor(parts, opts) {
    super(parts, opts);
  }
}
class AnvilTextEncoder extends runtime_web_apis$AnvilTextEncoder {
  constructor() {
    super();
  }
}
class TextEncoder extends runtime_web_apis$AnvilTextEncoder {
  constructor() {
    super();
  }
}
class AnvilTextDecoder extends runtime_web_apis$AnvilTextDecoder {
  constructor(label) {
    super(label);
  }
}
class TextDecoder extends runtime_web_apis$AnvilTextDecoder {
  constructor(label) {
    super(label);
  }
}
class AnvilPerformance extends runtime_web_apis$AnvilPerformance {
  constructor() {
    super();
  }
}
class Performance extends runtime_web_apis$AnvilPerformance {
  constructor() {
    super();
  }
}
class AnvilBroadcastChannel extends runtime_web_apis$AnvilBroadcastChannel {
  constructor(name) {
    super(name);
  }
}
class BroadcastChannel extends runtime_web_apis$AnvilBroadcastChannel {
  constructor(name) {
    super(name);
  }
}
class AnvilMessageChannel extends runtime_web_apis$AnvilMessageChannel {
  constructor() {
    super();
  }
}
class MessageChannel extends runtime_web_apis$AnvilMessageChannel {
  constructor() {
    super();
  }
}
class AnvilCSSStyleSheet extends runtime_web_apis$AnvilCSSStyleSheet {
  constructor() {
    super();
  }
}
class CSSStyleSheet extends runtime_web_apis$AnvilCSSStyleSheet {
  constructor() {
    super();
  }
}
function arrayAt(arr, index) {
  return runtime_builtins$anvilArrayAt(arr, index);
}
function arrayToReversed(arr) {
  return runtime_builtins$anvilArrayToReversed(arr);
}
function arrayToSorted(arr, compareFn) {
  return runtime_builtins$anvilArrayToSorted(arr, compareFn);
}
function arrayToSpliced(arr, start, deleteCount, items) {
  return runtime_builtins$anvilArrayToSpliced(arr, start, deleteCount, items);
}
function arrayWith(arr, index, value) {
  return runtime_builtins$anvilArrayWith(arr, index, value);
}
function arrayFindLast(arr, fn) {
  return runtime_builtins$anvilArrayFindLast(arr, fn);
}
function arrayFindLastIndex(arr, fn) {
  return runtime_builtins$anvilArrayFindLastIndex(arr, fn);
}
function arrayFlat(arr, depth) {
  return runtime_builtins$anvilArrayFlat(arr, depth);
}
function arrayFlatMap(arr, fn) {
  return runtime_builtins$anvilArrayFlatMap(arr, fn);
}
function objectGroupBy(items, callbackFn) {
  return runtime_builtins$anvilObjectGroupBy(items, callbackFn);
}
function mapGroupBy(items, callbackFn) {
  return runtime_builtins$anvilMapGroupBy(items, callbackFn);
}
function promiseWithResolvers() {
  return runtime_builtins$anvilPromiseWithResolvers();
}
function stringRepeat(str, count) {
  return runtime_builtins$anvilStringRepeat(str, count);
}
function stringPadStart(str, targetLength, padString) {
  return runtime_builtins$anvilStringPadStart(str, targetLength, padString);
}
function stringPadEnd(str, targetLength, padString) {
  return runtime_builtins$anvilStringPadEnd(str, targetLength, padString);
}
function stringAt(str, index) {
  return runtime_builtins$anvilStringAt(str, index);
}
function structuredClone(val) {
  return runtime_builtins$anvilStructuredClone(val);
}
function arrayMap(arr, fn) {
  return runtime_builtins$anvilArrayMap(arr, fn);
}
function arrayFilter(arr, fn) {
  return runtime_builtins$anvilArrayFilter(arr, fn);
}
function arrayReduce(arr, fn, init) {
  return runtime_builtins$anvilArrayReduce(arr, fn, init);
}
function arrayFind(arr, fn) {
  return runtime_builtins$anvilArrayFind(arr, fn);
}
function arrayFindIndex(arr, fn) {
  return runtime_builtins$anvilArrayFindIndex(arr, fn);
}
function arraySome(arr, fn) {
  return runtime_builtins$anvilArraySome(arr, fn);
}
function arrayEvery(arr, fn) {
  return runtime_builtins$anvilArrayEvery(arr, fn);
}
function arrayForEach(arr, fn) {
  return runtime_builtins$anvilArrayForEach(arr, fn);
}
function arrayIncludes(arr, needle) {
  return runtime_builtins$anvilArrayIncludes(arr, needle);
}
function arrayIndexOf(arr, needle) {
  return runtime_builtins$anvilArrayIndexOf(arr, needle);
}
function arraySlice(arr, start, end) {
  return runtime_builtins$anvilSlice(arr, start, end);
}
function arrayJoin(arr, sep) {
  return runtime_builtins$anvilArrayJoin(arr, sep);
}
function stringTrim(str) {
  return runtime_builtins$anvilStringTrim(str);
}
function stringSplit(str, sep) {
  return runtime_builtins$anvilStringSplit(str, sep);
}
function stringIncludes(str, sub) {
  return runtime_builtins$anvilStringIncludes(str, sub);
}
function stringStartsWith(str, prefix) {
  return runtime_builtins$anvilStringStartsWith(str, prefix);
}
function stringEndsWith(str, suffix) {
  return runtime_builtins$anvilStringEndsWith(str, suffix);
}
function stringToLowerCase(str) {
  return runtime_builtins$anvilStringToLowerCase(str);
}
function stringToUpperCase(str) {
  return runtime_builtins$anvilStringToUpperCase(str);
}
function stringReplace(str, pat, rep) {
  return runtime_builtins$anvilStringReplace(str, pat, rep);
}
function stringReplaceAll(str, pat, rep) {
  return runtime_builtins$anvilStringReplaceAll(str, pat, rep);
}
function anvilJsonParse(text) {
  return runtime_json_codec$anvilJsonParse(text);
}
function anvilJsonStringify(value) {
  return runtime_json_codec$anvilJsonStringify(value);
}
function anvilBtoa(input) {
  return runtime_json_codec$anvilBtoa(input);
}
function anvilAtob(input) {
  return runtime_json_codec$anvilAtob(input);
}
function anvilEncodeURIComponent(str) {
  return runtime_json_codec$anvilEncodeURIComponent(str);
}
function anvilDecodeURIComponent(str) {
  return runtime_json_codec$anvilDecodeURIComponent(str);
}
"#;

const COMPILER_PUBLIC_API_BRIDGE: &str = r#"
function compileSource(sourceText, isModule) {
  return index$compileSource(sourceText, isModule);
}
function transpileToCoreJs(sourceText, isModule) {
  return index$transpileToCoreJs(sourceText, isModule);
}
function parseAndAnalyze(sourceText, isModule) {
  return index$parseAndAnalyze(sourceText, isModule);
}
function createRuntimeToolkit(initialUrl) {
  return index$createRuntimeToolkit(initialUrl);
}
"#;

/// Link and return the complete self-hosted `anvil-js` Core JS bundle.
pub fn self_hosted_bundle() -> &'static str {
    static BUNDLE: OnceLock<String> = OnceLock::new();
    BUNDLE.get_or_init(|| {
        let mods = load_embedded_modules().expect("embedded MODULES.txt must be valid");
        let linked = link::link(&mods).expect("self-hosted JS modules must link cleanly");
        let mut text = linked.text;
        text.push_str(RUNTIME_PUBLIC_API_BRIDGE);
        text.push_str(COMPILER_PUBLIC_API_BRIDGE);
        text
    })
}

/// Link and return only the runtime standard library and Web/DOM modules
/// (`builtins.js`, `json_codec.js`, `web_apis.js`) as a standalone Core JS bundle.
pub fn self_hosted_runtime_bundle() -> &'static str {
    static RUNTIME_BUNDLE: OnceLock<String> = OnceLock::new();
    RUNTIME_BUNDLE.get_or_init(|| {
        let mods = vec![
            link::Module {
                path: "src/core/limits.js".to_string(),
                level: corejs::check::LEVEL_ALL,
                builtins: false,
                src: include_str!("../js/src/core/limits.js").to_string(),
            },
            link::Module {
                path: "src/core/unicode_tables.js".to_string(),
                level: corejs::check::LEVEL_ALL,
                builtins: false,
                src: include_str!("../js/src/core/unicode_tables.js").to_string(),
            },
            link::Module {
                path: "src/core/text.js".to_string(),
                level: corejs::check::LEVEL_ALL,
                builtins: true,
                src: include_str!("../js/src/core/text.js").to_string(),
            },
            link::Module {
                path: "src/runtime/builtins.js".to_string(),
                level: corejs::check::LEVEL_ALL,
                builtins: false,
                src: RUNTIME_BUILTINS_JS.to_string(),
            },
            link::Module {
                path: "src/runtime/json_codec.js".to_string(),
                level: corejs::check::LEVEL_ALL,
                builtins: false,
                src: RUNTIME_JSON_CODEC_JS.to_string(),
            },
            link::Module {
                path: "src/runtime/web_apis.js".to_string(),
                level: corejs::check::LEVEL_ALL,
                builtins: false,
                src: RUNTIME_WEB_APIS_JS.to_string(),
            },
        ];
        let linked = link::link(&mods).expect("self-hosted runtime modules must link cleanly");
        let mut text = linked.text;
        text.push_str(RUNTIME_PUBLIC_API_BRIDGE);
        text
    })
}

/// 64-bit FNV-1a content hash of [`self_hosted_bundle`].
pub fn self_hosted_bundle_hash() -> u64 {
    link::fnv1a(self_hosted_bundle().as_bytes())
}
