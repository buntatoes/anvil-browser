// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Self-hosted Core JS runtime library: array/string/object helpers, JSON
// parser and serializer, Promise, URL, URLSearchParams, EventTarget and
// CustomEvent, plus the runtime prelude source for lowered programs.

export const RUNTIME_PRELUDE_SOURCE =
  "function __anvil_slice(a, s) {\n" +
  "  const out = [];\n" +
  "  if (a === null || a === undefined) { return out; }\n" +
  "  const n = a.length;\n" +
  "  let i = s < 0 ? 0 : s;\n" +
  "  while (i < n) {\n" +
  "    out.push(a[i]);\n" +
  "    i = i + 1;\n" +
  "  }\n" +
  "  return out;\n" +
  "}\n" +
  "function __anvil_assign(dst, src) {\n" +
  "  if (src === null || src === undefined) { return dst; }\n" +
  "  if (src._keys !== undefined) {\n" +
  "    let i = 0;\n" +
  "    while (i < src._keys.length) {\n" +
  "      const k = src._keys[i];\n" +
  "      dst[k] = src[k];\n" +
  "      i = i + 1;\n" +
  "    }\n" +
  "  }\n" +
  "  return dst;\n" +
  "}\n" +
  "function __anvil_keys(obj) {\n" +
  "  const out = [];\n" +
  "  if (obj === null || obj === undefined) { return out; }\n" +
  "  if (obj._keys !== undefined) { return obj._keys; }\n" +
  "  const n = obj.length;\n" +
  "  if (typeof n === \"number\") {\n" +
  "    let i = 0;\n" +
  "    while (i < n) {\n" +
  "      out.push(\"\" + i);\n" +
  "      i = i + 1;\n" +
  "    }\n" +
  "  }\n" +
  "  return out;\n" +
  "}\n";

export function anvilSlice(arr, start, end) {
  const out = [];
  if (arr === null || arr === undefined) {
    return out;
  }
  const len = arr.length;
  let s = start === undefined ? 0 : start;
  if (s < 0) {
    s = len + s;
    if (s < 0) {
      s = 0;
    }
  }
  let e = end === undefined ? len : end;
  if (e < 0) {
    e = len + e;
  }
  if (e > len) {
    e = len;
  }
  let i = s;
  while (i < e) {
    out.push(arr[i]);
    i += 1;
  }
  return out;
}

export function anvilArrayMap(arr, fn) {
  const out = [];
  let i = 0;
  while (i < arr.length) {
    out.push(fn(arr[i], i, arr));
    i += 1;
  }
  return out;
}

export function anvilArrayFilter(arr, fn) {
  const out = [];
  let i = 0;
  while (i < arr.length) {
    const item = arr[i];
    if (fn(item, i, arr)) {
      out.push(item);
    }
    i += 1;
  }
  return out;
}

export function anvilArrayReduce(arr, fn, initial) {
  let i = 0;
  let acc = initial;
  if (acc === undefined && arr.length > 0) {
    acc = arr[0];
    i = 1;
  }
  while (i < arr.length) {
    acc = fn(acc, arr[i], i, arr);
    i += 1;
  }
  return acc;
}

export function anvilArrayForEach(arr, fn) {
  let i = 0;
  while (i < arr.length) {
    fn(arr[i], i, arr);
    i += 1;
  }
}

export function anvilArrayFind(arr, fn) {
  let i = 0;
  while (i < arr.length) {
    const item = arr[i];
    if (fn(item, i, arr)) {
      return item;
    }
    i += 1;
  }
  return undefined;
}

export function anvilArrayFindIndex(arr, fn) {
  let i = 0;
  while (i < arr.length) {
    if (fn(arr[i], i, arr)) {
      return i;
    }
    i += 1;
  }
  return -1;
}

export function anvilArraySome(arr, fn) {
  let i = 0;
  while (i < arr.length) {
    if (fn(arr[i], i, arr)) {
      return true;
    }
    i += 1;
  }
  return false;
}

export function anvilArrayEvery(arr, fn) {
  let i = 0;
  while (i < arr.length) {
    if (!fn(arr[i], i, arr)) {
      return false;
    }
    i += 1;
  }
  return true;
}

export function anvilArrayIndexOf(arr, target) {
  let i = 0;
  while (i < arr.length) {
    if (arr[i] === target) {
      return i;
    }
    i += 1;
  }
  return -1;
}

export function anvilArrayIncludes(arr, target) {
  return anvilArrayIndexOf(arr, target) >= 0;
}

export function anvilArrayJoin(arr, sep) {
  const glue = sep === undefined ? "," : sep;
  let out = "";
  let i = 0;
  while (i < arr.length) {
    if (i > 0) {
      out += glue;
    }
    const v = arr[i];
    if (v !== null && v !== undefined) {
      out += `${v}`;
    }
    i += 1;
  }
  return out;
}

function isTrimSpace(c) {
  return c === 0x20 || c === 0x09 || c === 0x0a || c === 0x0d || c === 0x0b || c === 0x0c;
}

export function anvilStringTrim(s) {
  const len = s.length;
  let start = 0;
  while (start < len && isTrimSpace(s.charCodeAt(start))) {
    start += 1;
  }
  let end = len;
  while (end > start && isTrimSpace(s.charCodeAt(end - 1))) {
    end -= 1;
  }
  return s.slice(start, end);
}

export function anvilStringStartsWith(s, prefix) {
  if (prefix.length > s.length) {
    return false;
  }
  return s.slice(0, prefix.length) === prefix;
}

export function anvilStringEndsWith(s, suffix) {
  if (suffix.length > s.length) {
    return false;
  }
  return s.slice(s.length - suffix.length, s.length) === suffix;
}

export function anvilStringIncludes(s, sub) {
  return s.indexOf(sub) >= 0;
}

export function anvilStringSplit(s, sep) {
  const out = [];
  if (sep === "") {
    let i = 0;
    while (i < s.length) {
      out.push(s.slice(i, i + 1));
      i += 1;
    }
    return out;
  }
  let pos = 0;
  const slen = sep.length;
  while (pos <= s.length) {
    const idx = s.indexOf(sep, pos);
    if (idx < 0) {
      out.push(s.slice(pos, s.length));
      break;
    }
    out.push(s.slice(pos, idx));
    pos = idx + slen;
  }
  return out;
}

export function anvilStringReplace(s, search, replacement) {
  const idx = s.indexOf(search);
  if (idx < 0) {
    return s;
  }
  return s.slice(0, idx) + replacement + s.slice(idx + search.length, s.length);
}

export function anvilStringReplaceAll(s, search, replacement) {
  if (search === "") {
    return s;
  }
  return anvilArrayJoin(anvilStringSplit(s, search), replacement);
}

export function anvilStringToLowerCase(s) {
  let out = "";
  let i = 0;
  while (i < s.length) {
    const c = s.charCodeAt(i);
    if (c >= 0x41 && c <= 0x5a) {
      out += String.fromCharCode(c + 32);
    } else {
      out += s.slice(i, i + 1);
    }
    i += 1;
  }
  return out;
}

export function anvilStringToUpperCase(s) {
  let out = "";
  let i = 0;
  while (i < s.length) {
    const c = s.charCodeAt(i);
    if (c >= 0x61 && c <= 0x7a) {
      out += String.fromCharCode(c - 32);
    } else {
      out += s.slice(i, i + 1);
    }
    i += 1;
  }
  return out;
}

export class AnvilPromise {
  constructor(executor) {
    this.state = 0;
    this.value = undefined;
    const self = this;
    const resolve = (v) => {
      if (self.state === 0) {
        self.state = 1;
        self.value = v;
      }
    };
    const reject = (r) => {
      if (self.state === 0) {
        self.state = 2;
        self.value = r;
      }
    };
    try {
      executor(resolve, reject);
    } catch (e) {
      reject(e);
    }
  }

  then(onFulfilled, onRejected) {
    if (this.state === 1) {
      if (typeof onFulfilled === "function") {
        try {
          const next = onFulfilled(this.value);
          if (next instanceof AnvilPromise) {
            return next;
          }
          return new AnvilPromise((res) => {
            res(next);
          });
        } catch (err) {
          return new AnvilPromise((res, rej) => {
            rej(err);
          });
        }
      }
      return this;
    }
    if (this.state === 2) {
      if (typeof onRejected === "function") {
        try {
          const rec = onRejected(this.value);
          if (rec instanceof AnvilPromise) {
            return rec;
          }
          return new AnvilPromise((res) => {
            res(rec);
          });
        } catch (err) {
          return new AnvilPromise((res, rej) => {
            rej(err);
          });
        }
      }
      return this;
    }
    return this;
  }

  catch(onRejected) {
    return this.then(undefined, onRejected);
  }

  finally(onFinally) {
    if (typeof onFinally === "function") {
      onFinally();
    }
    return this;
  }
}

export function anvilPromiseResolve(v) {
  if (v instanceof AnvilPromise) {
    return v;
  }
  return new AnvilPromise((res) => {
    res(v);
  });
}

export function anvilPromiseReject(r) {
  return new AnvilPromise((res, rej) => {
    rej(r);
  });
}

export class AnvilURLSearchParams {
  constructor(init) {
    this.map = new Map();
    if (typeof init === "string") {
      let q = init;
      if (anvilStringStartsWith(q, "?")) {
        q = q.slice(1, q.length);
      }
      if (q.length > 0) {
        const parts = anvilStringSplit(q, "&");
        for (const part of parts) {
          const eq = part.indexOf("=");
          if (eq >= 0) {
            this.map.set(part.slice(0, eq), part.slice(eq + 1, part.length));
          } else if (part.length > 0) {
            this.map.set(part, "");
          }
        }
      }
    }
  }

  get(key) {
    if (this.map.has(key)) {
      return this.map.get(key);
    }
    return null;
  }

  set(key, val) {
    this.map.set(key, `${val}`);
  }

  has(key) {
    return this.map.has(key);
  }

  delete(key) {
    return this.map.delete(key);
  }

  toString() {
    const pairs = [];
    this.map.forEach((v, k) => {
      pairs.push(`${k}=${v}`);
    });
    return anvilArrayJoin(pairs, "&");
  }
}

export class AnvilURL {
  constructor(url) {
    this.href = url;
    this.protocol = "https:";
    this.hostname = "";
    this.port = "";
    this.pathname = "/";
    this.search = "";
    this.hash = "";

    let rest = url;
    const hashIdx = rest.indexOf("#");
    if (hashIdx >= 0) {
      this.hash = rest.slice(hashIdx, rest.length);
      rest = rest.slice(0, hashIdx);
    }
    const qIdx = rest.indexOf("?");
    if (qIdx >= 0) {
      this.search = rest.slice(qIdx, rest.length);
      rest = rest.slice(0, qIdx);
    }
    const schemeIdx = rest.indexOf("://");
    if (schemeIdx >= 0) {
      this.protocol = rest.slice(0, schemeIdx + 1);
      rest = rest.slice(schemeIdx + 3, rest.length);
    }
    const slashIdx = rest.indexOf("/");
    let authority = rest;
    if (slashIdx >= 0) {
      authority = rest.slice(0, slashIdx);
      this.pathname = rest.slice(slashIdx, rest.length);
    }
    const colonIdx = authority.indexOf(":");
    if (colonIdx >= 0) {
      this.hostname = authority.slice(0, colonIdx);
      this.port = authority.slice(colonIdx + 1, authority.length);
    } else {
      this.hostname = authority;
    }
    this.host = this.port.length > 0 ? `${this.hostname}:${this.port}` : this.hostname;
    this.origin = `${this.protocol}//${this.host}`;
    this.searchParams = new AnvilURLSearchParams(this.search);
  }
}

export class AnvilCustomEvent {
  constructor(type, options) {
    this.type = type;
    this.detail = options !== undefined && options !== null ? options.detail : null;
    this.bubbles = options !== undefined && options !== null ? !!options.bubbles : false;
    this.cancelable =
      options !== undefined && options !== null && options.cancelable !== undefined
        ? !!options.cancelable
        : true;
    this.defaultPrevented = false;
    this.propagationStopped = false;
    this.immediatePropagationStopped = false;
    this.target = null;
    this.currentTarget = null;
  }

  preventDefault() {
    if (this.cancelable) {
      this.defaultPrevented = true;
    }
  }

  stopPropagation() {
    this.propagationStopped = true;
  }

  stopImmediatePropagation() {
    this.propagationStopped = true;
    this.immediatePropagationStopped = true;
  }
}

export class AnvilEventTarget {
  constructor() {
    this.listeners = new Map();
  }

  addEventListener(type, callback) {
    if (!this.listeners.has(type)) {
      this.listeners.set(type, []);
    }
    const list = this.listeners.get(type);
    list.push(callback);
  }

  removeEventListener(type, callback) {
    if (!this.listeners.has(type)) {
      return;
    }
    const list = this.listeners.get(type);
    const next = [];
    for (let i = 0; i < list.length; i++) {
      const fn = list[i];
      if (fn !== callback) {
        next.push(fn);
      }
    }
    this.listeners.set(type, next);
  }

  dispatchEvent(event) {
    if (!event) {
      return true;
    }
    if (!event.target) {
      event.target = this;
    }
    event.currentTarget = this;
    if (!this.listeners.has(event.type)) {
      return !event.defaultPrevented;
    }
    const list = this.listeners.get(event.type);
    for (let i = 0; i < list.length; i++) {
      if (event.immediatePropagationStopped) {
        break;
      }
      const fn = list[i];
      if (typeof fn === "function") {
        fn(event);
      }
    }
    return !event.defaultPrevented;
  }
}

export function anvilArrayAt(arr, index) {
  if (arr === null || arr === undefined) {
    return undefined;
  }
  const len = arr.length;
  let k = index < 0 ? len + index : index;
  if (k < 0 || k >= len) {
    return undefined;
  }
  return arr[k];
}

export function anvilArrayToReversed(arr) {
  const out = [];
  if (arr === null || arr === undefined) {
    return out;
  }
  for (let i = arr.length - 1; i >= 0; i--) {
    out.push(arr[i]);
  }
  return out;
}

export function anvilArrayToSorted(arr, compareFn) {
  const out = [];
  if (arr === null || arr === undefined) {
    return out;
  }
  for (let i = 0; i < arr.length; i++) {
    out.push(arr[i]);
  }
  const n = out.length;
  for (let i = 1; i < n; i++) {
    const key = out[i];
    let j = i - 1;
    while (j >= 0) {
      let cmp = 0;
      if (typeof compareFn === "function") {
        cmp = compareFn(out[j], key);
      } else {
        const a = `${out[j]}`;
        const b = `${key}`;
        cmp = a < b ? -1 : a > b ? 1 : 0;
      }
      if (cmp <= 0) {
        break;
      }
      out[j + 1] = out[j];
      j = j - 1;
    }
    out[j + 1] = key;
  }
  return out;
}

export function anvilArrayToSpliced(arr, start, deleteCount, items) {
  const out = [];
  if (arr === null || arr === undefined) {
    return out;
  }
  const len = arr.length;
  let actualStart = start === undefined ? 0 : start;
  if (actualStart < 0) {
    actualStart = len + actualStart;
    if (actualStart < 0) {
      actualStart = 0;
    }
  } else if (actualStart > len) {
    actualStart = len;
  }
  let actualDelete =
    deleteCount === undefined ? len - actualStart : deleteCount;
  if (actualDelete < 0) {
    actualDelete = 0;
  }
  if (actualDelete > len - actualStart) {
    actualDelete = len - actualStart;
  }
  for (let i = 0; i < actualStart; i++) {
    out.push(arr[i]);
  }
  if (items !== undefined && items !== null && typeof items.length === "number") {
    for (let i = 0; i < items.length; i++) {
      out.push(items[i]);
    }
  }
  for (let i = actualStart + actualDelete; i < len; i++) {
    out.push(arr[i]);
  }
  return out;
}

export function anvilArrayWith(arr, index, value) {
  if (arr === null || arr === undefined) {
    throw new TypeError("Cannot read properties of null or undefined");
  }
  const len = arr.length;
  const k = index < 0 ? len + index : index;
  if (k < 0 || k >= len) {
    throw new RangeError("Invalid index in Array.prototype.with");
  }
  const out = [];
  for (let i = 0; i < len; i++) {
    out.push(i === k ? value : arr[i]);
  }
  return out;
}

export function anvilArrayFindLast(arr, fn) {
  if (arr === null || arr === undefined) {
    return undefined;
  }
  for (let i = arr.length - 1; i >= 0; i--) {
    if (fn(arr[i], i, arr)) {
      return arr[i];
    }
  }
  return undefined;
}

export function anvilArrayFindLastIndex(arr, fn) {
  if (arr === null || arr === undefined) {
    return -1;
  }
  for (let i = arr.length - 1; i >= 0; i--) {
    if (fn(arr[i], i, arr)) {
      return i;
    }
  }
  return -1;
}

export function anvilArrayFlat(arr, depth) {
  const out = [];
  if (arr === null || arr === undefined) {
    return out;
  }
  const maxDepth = depth === undefined ? 1 : depth;
  function flattenInto(target, source, currentDepth) {
    for (let i = 0; i < source.length; i++) {
      const el = source[i];
      if (
        currentDepth > 0 &&
        el !== null &&
        el !== undefined &&
        typeof el === "object" &&
        typeof el.length === "number"
      ) {
        flattenInto(target, el, currentDepth - 1);
      } else {
        target.push(el);
      }
    }
  }
  flattenInto(out, arr, maxDepth);
  return out;
}

export function anvilArrayFlatMap(arr, fn) {
  const mapped = anvilArrayMap(arr, fn);
  return anvilArrayFlat(mapped, 1);
}

export function anvilObjectGroupBy(items, callbackFn) {
  const groups = new Map();
  if (items === null || items === undefined) {
    return groups;
  }
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    const key = `${callbackFn(item, i)}`;
    if (!groups.has(key)) {
      groups.set(key, []);
    }
    groups.get(key).push(item);
  }
  return groups;
}

export function anvilMapGroupBy(items, callbackFn) {
  const groups = new Map();
  if (items === null || items === undefined) {
    return groups;
  }
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    const key = callbackFn(item, i);
    if (!groups.has(key)) {
      groups.set(key, []);
    }
    groups.get(key).push(item);
  }
  return groups;
}

export function anvilPromiseWithResolvers() {
  let resolveFn = null;
  let rejectFn = null;
  const promise = new AnvilPromise((resolve, reject) => {
    resolveFn = resolve;
    rejectFn = reject;
  });
  return {
    promise: promise,
    resolve: resolveFn,
    reject: rejectFn,
  };
}

export function anvilStringRepeat(str, count) {
  const s = `${str}`;
  const n = count <= 0 ? 0 : Math.floor(count);
  let out = "";
  for (let i = 0; i < n; i++) {
    out = out + s;
  }
  return out;
}

export function anvilStringPadStart(str, targetLength, padString) {
  const s = `${str}`;
  const maxLen = Math.floor(targetLength);
  if (s.length >= maxLen) {
    return s;
  }
  const fill = padString === undefined ? " " : `${padString}`;
  if (fill.length === 0) {
    return s;
  }
  const padLen = maxLen - s.length;
  let pad = "";
  while (pad.length < padLen) {
    pad = pad + fill;
  }
  return pad.slice(0, padLen) + s;
}

export function anvilStringPadEnd(str, targetLength, padString) {
  const s = `${str}`;
  const maxLen = Math.floor(targetLength);
  if (s.length >= maxLen) {
    return s;
  }
  const fill = padString === undefined ? " " : `${padString}`;
  if (fill.length === 0) {
    return s;
  }
  const padLen = maxLen - s.length;
  let pad = "";
  while (pad.length < padLen) {
    pad = pad + fill;
  }
  return s + pad.slice(0, padLen);
}

export function anvilStringAt(str, index) {
  const s = `${str}`;
  const len = s.length;
  const k = index < 0 ? len + index : index;
  if (k < 0 || k >= len) {
    return undefined;
  }
  return s.slice(k, k + 1);
}

export function anvilStructuredClone(val) {
  if (val === null || val === undefined || typeof val !== "object") {
    return val;
  }
  if (typeof val.length === "number") {
    const out = [];
    for (let i = 0; i < val.length; i++) {
      out.push(anvilStructuredClone(val[i]));
    }
    return out;
  }
  return val;
}
