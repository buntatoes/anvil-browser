// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

// Self-hosted Core JS Web APIs and DOM runtime for modern web applications
// (Google Search, YouTube Polymer custom elements/Shadow DOM/MSE, Gmail SPA).

import {
  AnvilEventTarget,
  AnvilCustomEvent,
  AnvilPromise,
  anvilPromiseResolve,
  anvilStringToLowerCase,
  anvilStringTrim,
  anvilStringSplit,
  anvilStringStartsWith,
  anvilArrayIndexOf,
  anvilArrayJoin,
  anvilStructuredClone,
} from "./builtins.js";
import {
  anvilJsonParse,
  anvilEncodeURIComponent,
  anvilDecodeURIComponent,
  anvilBtoa,
  anvilAtob,
} from "./json_codec.js";

function toStr(x) {
  return "" + x;
}

function toNum(x) {
  return +x;
}

export class AnvilHeaders {
  constructor(entries) {
    this._map = new Map();
    if (entries && typeof entries.length === "number") {
      for (let i = 0; i < entries.length; i++) {
        const pair = entries[i];
        if (pair && pair.length >= 2) {
          this.set(pair[0], pair[1]);
        }
      }
    }
  }

  _norm(name) {
    return anvilStringToLowerCase(anvilStringTrim(toStr(name)));
  }

  set(name, value) {
    this._map.set(this._norm(name), toStr(value));
  }

  get(name) {
    const k = this._norm(name);
    if (!this._map.has(k)) {
      return null;
    }
    return this._map.get(k);
  }

  has(name) {
    return this._map.has(this._norm(name));
  }

  append(name, value) {
    const k = this._norm(name);
    if (this._map.has(k)) {
      this._map.set(k, this._map.get(k) + ", " + toStr(value));
    } else {
      this._map.set(k, toStr(value));
    }
  }

  delete(name) {
    this._map.delete(this._norm(name));
  }
}

export class AnvilResponse {
  constructor(bodyText, options) {
    const opts = options || {};
    this._body = bodyText === undefined || bodyText === null ? "" : toStr(bodyText);
    this.status = opts.status ? toNum(opts.status) : 200;
    this.statusText = opts.statusText ? toStr(opts.statusText) : "OK";
    this.ok = this.status >= 200 && this.status < 300;
    this.url = opts.url ? toStr(opts.url) : "";
    this.headers =
      opts.headers instanceof AnvilHeaders
        ? opts.headers
        : new AnvilHeaders(opts.headers || []);
  }

  text() {
    return anvilPromiseResolve(this._body);
  }

  json() {
    const body = this._body;
    return new AnvilPromise((resolve, reject) => {
      try {
        resolve(anvilJsonParse(body));
      } catch (e) {
        reject(e);
      }
    });
  }

  clone() {
    return new AnvilResponse(this._body, {
      status: this.status,
      statusText: this.statusText,
      url: this.url,
      headers: this.headers,
    });
  }
}

export class AnvilRequest {
  constructor(input, options) {
    const opts = options || {};
    if (input instanceof AnvilRequest) {
      this.url = input.url;
      this.method = opts.method ? toStr(opts.method) : input.method;
      this.headers = new AnvilHeaders(opts.headers || []);
      this.body = opts.body !== undefined ? toStr(opts.body) : input.body;
    } else {
      this.url = toStr(input);
      this.method = opts.method ? toStr(opts.method) : "GET";
      this.headers = new AnvilHeaders(opts.headers || []);
      this.body = opts.body !== undefined ? toStr(opts.body) : null;
    }
  }
}

export class AnvilAbortSignal extends AnvilEventTarget {
  constructor() {
    super();
    this.aborted = false;
    this.reason = undefined;
    this.onabort = null;
  }

  _trigger(reason) {
    if (this.aborted) return;
    this.aborted = true;
    this.reason = reason !== undefined ? reason : new Error("AbortError");
    const ev = new AnvilCustomEvent("abort", { detail: this.reason });
    if (typeof this.onabort === "function") {
      const fn = this.onabort;
      fn(ev);
    }
    this.dispatchEvent(ev);
  }
}

export class AnvilAbortController {
  constructor() {
    this.signal = new AnvilAbortSignal();
  }

  abort(reason) {
    this.signal._trigger(reason);
  }
}

export class AnvilStorage {
  constructor(ownerDoc, storageKind) {
    this._doc = ownerDoc || null;
    this._kind = storageKind ? toStr(storageKind) : "localStorage";
    this._keys = [];
    this._vals = new Map();
    this.length = 0;
  }

  _seedItem(key, value) {
    const k = toStr(key);
    const v = toStr(value);
    if (!this._vals.has(k)) {
      this._keys.push(k);
      this.length = this._keys.length;
    }
    this._vals.set(k, v);
  }

  getItem(key) {
    const k = toStr(key);
    if (!this._vals.has(k)) {
      return null;
    }
    return this._vals.get(k);
  }

  setItem(key, value) {
    const k = toStr(key);
    const v = toStr(value);
    if (!this._vals.has(k)) {
      this._keys.push(k);
      this.length = this._keys.length;
    }
    this._vals.set(k, v);
    if (this._doc && this._kind === "localStorage") {
      this._doc._recordDomOp({
        op: "localStorageSet",
        name: k,
        value: v,
      });
    }
  }

  removeItem(key) {
    const k = toStr(key);
    if (!this._vals.has(k)) {
      return;
    }
    this._vals.delete(k);
    const next = [];
    for (let i = 0; i < this._keys.length; i++) {
      if (this._keys[i] !== k) {
        next.push(this._keys[i]);
      }
    }
    this._keys = next;
    this.length = this._keys.length;
    if (this._doc && this._kind === "localStorage") {
      this._doc._recordDomOp({
        op: "localStorageRemove",
        name: k,
      });
    }
  }

  clear() {
    this._keys = [];
    this._vals = new Map();
    this.length = 0;
    if (this._doc && this._kind === "localStorage") {
      this._doc._recordDomOp({
        op: "localStorageClear",
      });
    }
  }

  key(index) {
    const i = toNum(index);
    if (i < 0 || i >= this._keys.length) {
      return null;
    }
    return this._keys[i];
  }
}

export class AnvilLocation {
  constructor(initialHref, ownerDoc) {
    this._doc = null;
    this.protocol = "https:";
    this.hostname = "localhost";
    this.port = "";
    this.host = "localhost";
    this.origin = "https://localhost";
    this.pathname = "/";
    this.search = "";
    this.hash = "";
    this.href = "https://localhost/";
    this.assign(initialHref || "https://localhost/");
    this._doc = ownerDoc || null;
  }

  assign(url) {
    const raw = anvilStringTrim(toStr(url));
    this.href = raw;
    let rest = raw;
    const hashIdx = rest.indexOf("#");
    if (hashIdx >= 0) {
      this.hash = rest.slice(hashIdx);
      rest = rest.slice(0, hashIdx);
    } else {
      this.hash = "";
    }
    const qIdx = rest.indexOf("?");
    if (qIdx >= 0) {
      this.search = rest.slice(qIdx);
      rest = rest.slice(0, qIdx);
    } else {
      this.search = "";
    }
    const schemeIdx = rest.indexOf("://");
    if (schemeIdx > 0) {
      this.protocol = anvilStringToLowerCase(rest.slice(0, schemeIdx)) + ":";
      const after = rest.slice(schemeIdx + 3);
      const slashIdx = after.indexOf("/");
      if (slashIdx >= 0) {
        this.host = after.slice(0, slashIdx);
        this.pathname = after.slice(slashIdx);
      } else {
        this.host = after;
        this.pathname = "/";
      }
      const colonIdx = this.host.indexOf(":");
      if (colonIdx >= 0) {
        this.hostname = this.host.slice(0, colonIdx);
        this.port = this.host.slice(colonIdx + 1);
      } else {
        this.hostname = this.host;
        this.port = "";
      }
      this.origin = this.protocol + "//" + this.host;
    } else if (anvilStringStartsWith(rest, "/")) {
      this.pathname = rest;
      this.href = this.origin + this.pathname + this.search + this.hash;
    }
    if (this._doc) {
      this._doc._recordDomOp({
        op: "setLocation",
        value: this.href,
      });
    }
  }

  replace(url) {
    this.assign(url);
  }

  reload() {}

  toString() {
    return this.href;
  }
}

export class AnvilHistory {
  constructor(locationObj) {
    this._loc = locationObj;
    this._entries = [{ state: null, title: "", url: locationObj.href }];
    this._index = 0;
    this.state = null;
    this.length = 1;
  }

  pushState(stateObj, title, url) {
    const nextUrl = url ? toStr(url) : this._loc.href;
    if (url) {
      this._loc.assign(nextUrl);
    }
    while (this._entries.length > this._index + 1) {
      this._entries.pop();
    }
    this._entries.push({
      state: stateObj,
      title: toStr(title || ""),
      url: this._loc.href,
    });
    this._index = this._entries.length - 1;
    this.state = stateObj;
    this.length = this._entries.length;
  }

  replaceState(stateObj, title, url) {
    if (url) {
      this._loc.assign(toStr(url));
    }
    this._entries[this._index] = {
      state: stateObj,
      title: toStr(title || ""),
      url: this._loc.href,
    };
    this.state = stateObj;
  }

  back() {
    this.go(-1);
  }

  forward() {
    this.go(1);
  }

  go(delta) {
    const next = this._index + toNum(delta || 0);
    if (next >= 0 && next < this._entries.length) {
      this._index = next;
      const entry = this._entries[next];
      this.state = entry.state;
      this._loc.assign(entry.url);
    }
  }
}

export class AnvilClassList {
  constructor(ownerNode) {
    this._node = ownerNode;
    this._tokens = [];
    this.length = 0;
  }

  _syncFromAttr(raw) {
    const parts = anvilStringSplit(anvilStringTrim(toStr(raw || "")), " ");
    const out = [];
    for (let i = 0; i < parts.length; i++) {
      const p = anvilStringTrim(parts[i]);
      if (p.length > 0 && anvilArrayIndexOf(out, p) < 0) {
        out.push(p);
      }
    }
    this._tokens = out;
    this.length = out.length;
  }

  _syncToAttr() {
    this.length = this._tokens.length;
    const cls = anvilArrayJoin(this._tokens, " ");
    this._node._className = cls;
    this._node._attrs.set("class", cls);
    if (this._node.ownerDocument) {
      this._node.ownerDocument._recordDomOp({
        op: "setAttribute",
        nodeId: this._node._hostNodeId,
        name: "class",
        value: cls,
      });
    }
  }

  _addOne(token) {
    if (token === undefined || token === null) return;
    const t = anvilStringTrim(toStr(token));
    if (t.length === 0) return;
    if (anvilArrayIndexOf(this._tokens, t) < 0) {
      this._tokens.push(t);
    }
  }

  add(t1, t2, t3, t4) {
    this._addOne(t1);
    this._addOne(t2);
    this._addOne(t3);
    this._addOne(t4);
    this._syncToAttr();
  }

  remove(token) {
    const t = anvilStringTrim(toStr(token));
    const next = [];
    for (let i = 0; i < this._tokens.length; i++) {
      if (this._tokens[i] !== t) {
        next.push(this._tokens[i]);
      }
    }
    this._tokens = next;
    this._syncToAttr();
  }

  contains(token) {
    return anvilArrayIndexOf(this._tokens, anvilStringTrim(toStr(token))) >= 0;
  }

  toggle(token, force) {
    const has = this.contains(token);
    if (force !== undefined) {
      if (force) {
        this.add(token);
        return true;
      }
      this.remove(token);
      return false;
    }
    if (has) {
      this.remove(token);
      return false;
    }
    this.add(token);
    return true;
  }

  replace(oldToken, newToken) {
    if (!this.contains(oldToken)) return false;
    this.remove(oldToken);
    this.add(newToken);
    return true;
  }

  toString() {
    return anvilArrayJoin(this._tokens, " ");
  }
}

function attrToDatasetKey(attrName) {
  const rest = attrName.slice(5);
  let out = "";
  let upperNext = false;
  for (let i = 0; i < rest.length; i++) {
    const c = rest.charCodeAt(i);
    if (c === 45) {
      upperNext = true;
    } else if (upperNext) {
      upperNext = false;
      if (c >= 97 && c <= 122) {
        out = out + String.fromCharCode(c - 32);
      } else {
        out = out + String.fromCharCode(c);
      }
    } else {
      out = out + String.fromCharCode(c);
    }
  }
  return out;
}

export class AnvilImageData {
  constructor(widthOrData, heightOrWidth, maybeHeight) {
    if (typeof widthOrData === "number") {
      this.width = toNum(widthOrData);
      this.height = toNum(heightOrWidth || 1);
      const len = this.width * this.height * 4;
      const arr = [];
      for (let i = 0; i < len; i++) {
        arr.push(0);
      }
      this.data = arr;
    } else {
      this.data = widthOrData || [];
      this.width = toNum(heightOrWidth || 1);
      this.height = toNum(maybeHeight || 1);
    }
  }
}

export class AnvilCanvasRenderingContext2D {
  constructor(canvasNode) {
    this.canvas = canvasNode;
    this.fillStyle = "#000000";
    this.strokeStyle = "#000000";
    this.lineWidth = 1;
    this.font = "14px sans-serif";
    this.globalAlpha = 1;
    this._ops = [];
    this._path = [];
    this._lastImageData = null;
  }

  _record(cmd) {
    this._ops.push(cmd);
    if (this.canvas && this.canvas.ownerDocument) {
      this.canvas.ownerDocument._recordDomOp({
        op: "canvasDraw",
        nodeId: this.canvas._hostNodeId,
        cmd: cmd,
      });
    }
  }

  fillRect(x, y, w, h) {
    this._record({
      kind: "fillRect",
      x: toNum(x),
      y: toNum(y),
      w: toNum(w),
      h: toNum(h),
      style: toStr(this.fillStyle),
    });
  }

  strokeRect(x, y, w, h) {
    this._record({
      kind: "strokeRect",
      x: toNum(x),
      y: toNum(y),
      w: toNum(w),
      h: toNum(h),
      style: toStr(this.strokeStyle),
      lineWidth: toNum(this.lineWidth),
    });
  }

  clearRect(x, y, w, h) {
    this._record({
      kind: "clearRect",
      x: toNum(x),
      y: toNum(y),
      w: toNum(w),
      h: toNum(h),
    });
  }

  fillText(text, x, y) {
    this._record({
      kind: "fillText",
      text: toStr(text),
      x: toNum(x),
      y: toNum(y),
      style: toStr(this.fillStyle),
      font: toStr(this.font),
    });
  }

  strokeText(text, x, y) {
    this._record({
      kind: "strokeText",
      text: toStr(text),
      x: toNum(x),
      y: toNum(y),
      style: toStr(this.strokeStyle),
      font: toStr(this.font),
    });
  }

  measureText(text) {
    const s = toStr(text);
    return { width: s.length * 8 };
  }

  beginPath() {
    this._path = [];
  }

  moveTo(x, y) {
    this._path.push({ p: "M", x: toNum(x), y: toNum(y) });
  }

  lineTo(x, y) {
    this._path.push({ p: "L", x: toNum(x), y: toNum(y) });
  }

  arc(x, y, radius, startAngle, endAngle, counterclockwise) {
    this._path.push({
      p: "A",
      x: toNum(x),
      y: toNum(y),
      r: toNum(radius),
      sa: toNum(startAngle || 0),
      ea: toNum(endAngle || 6.283185),
      ccw: !!counterclockwise,
    });
  }

  rect(x, y, w, h) {
    this._path.push({ p: "R", x: toNum(x), y: toNum(y), w: toNum(w), h: toNum(h) });
  }

  closePath() {
    this._path.push({ p: "Z" });
  }

  fill() {
    const copy = [];
    for (let i = 0; i < this._path.length; i++) {
      copy.push(this._path[i]);
    }
    this._record({
      kind: "fillPath",
      style: toStr(this.fillStyle),
      path: copy,
    });
  }

  stroke() {
    const copy = [];
    for (let i = 0; i < this._path.length; i++) {
      copy.push(this._path[i]);
    }
    this._record({
      kind: "strokePath",
      style: toStr(this.strokeStyle),
      lineWidth: toNum(this.lineWidth),
      path: copy,
    });
  }

  save() {}
  restore() {}
  scale() {}
  rotate() {}
  translate() {}

  createImageData(w, h) {
    return new AnvilImageData(toNum(w), toNum(h));
  }

  getImageData(x, y, w, h) {
    const iw = toNum(w || 1);
    const ih = toNum(h || 1);
    if (
      this._lastImageData &&
      this._lastImageData.width === iw &&
      this._lastImageData.height === ih
    ) {
      return this._lastImageData;
    }
    return new AnvilImageData(iw, ih);
  }

  putImageData(imageData, dx, dy) {
    if (!imageData) return;
    this._lastImageData = imageData;
    this._record({
      kind: "putImageData",
      dx: toNum(dx || 0),
      dy: toNum(dy || 0),
      width: toNum(imageData.width || 0),
      height: toNum(imageData.height || 0),
      data: imageData.data || [],
    });
  }

  drawImage(source, dx, dy, dw, dh) {
    this._record({
      kind: "drawImage",
      dx: toNum(dx || 0),
      dy: toNum(dy || 0),
      dw: toNum(dw || (source && source.width ? source.width : 100)),
      dh: toNum(dh || (source && source.height ? source.height : 100)),
    });
  }
}

export class AnvilDOMNode extends AnvilEventTarget {
  constructor(nodeType, tagName, ownerDoc) {
    super();
    this.nodeType = nodeType;
    this.tagName = tagName ? anvilStringToLowerCase(toStr(tagName)) : "";
    this.nodeName = tagName ? toStr(tagName) : "#text";
    this.ownerDocument = ownerDoc || null;
    this._hostNodeId = ownerDoc ? ownerDoc._allocNodeId() : 0;
    this._domNodeId = 0;
    this.parentNode = null;
    this.parentElement = null;
    this.childNodes = [];
    this.children = [];
    this.firstChild = null;
    this.lastChild = null;
    this.nextSibling = null;
    this.previousSibling = null;
    this.id = "";
    this._className = "";
    this.className = "";
    this._lastSyncedClass = "";
    this._attrs = new Map();
    this._attrKeys = [];
    this.style = { display: "", visibility: "", opacity: "", cssText: "" };
    this.dataset = {};
    this.classList = new AnvilClassList(this);
    this._text = "";
    this.textContent = "";
    this._lastSyncedText = "";
    this._html = "";
    this.innerHTML = "";
    this._lastSyncedHtml = "";
    this.value = "";
    this._lastSyncedValue = "";
    this.checked = false;
    this.disabled = false;
    this.hidden = false;
    this.shadowRoot = null;
    this.isConnected = false;
    this.width = 300;
    this.height = 150;
    this._ctx2d = null;
    this.src = "";
    this.paused = true;
    this.currentTime = 0;
    this.duration = 0;
    this.volume = 1;
    this.muted = false;
    this.playbackRate = 1;
    this.readyState = 4;
    this.buffered = {
      length: 1,
      start: () => 0,
      end: () => this.duration || 10,
    };
  }

  getContext(contextType) {
    const kind = anvilStringToLowerCase(anvilStringTrim(toStr(contextType || "")));
    if (kind === "2d") {
      if (!this._ctx2d) {
        this._ctx2d = new AnvilCanvasRenderingContext2D(this);
      }
      return this._ctx2d;
    }
    return null;
  }

  toDataURL() {
    return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
  }

  play() {
    this.paused = false;
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "mediaPlay",
        nodeId: this._hostNodeId,
        src: this.src || this.getAttribute("src") || "",
      });
    }
    this.dispatchEvent(new AnvilCustomEvent("play", {}));
    return anvilPromiseResolve(undefined);
  }

  pause() {
    this.paused = true;
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "mediaPause",
        nodeId: this._hostNodeId,
      });
    }
    this.dispatchEvent(new AnvilCustomEvent("pause", {}));
  }

  load() {
    this.readyState = 4;
    this.dispatchEvent(new AnvilCustomEvent("loadedmetadata", {}));
    this.dispatchEvent(new AnvilCustomEvent("canplay", {}));
  }

  canPlayType(mimeType) {
    const m = anvilStringToLowerCase(toStr(mimeType || ""));
    if (
      m.indexOf("mp4") >= 0 ||
      m.indexOf("webm") >= 0 ||
      m.indexOf("ogg") >= 0 ||
      m.indexOf("wav") >= 0 ||
      m.indexOf("flac") >= 0 ||
      m.indexOf("avc1") >= 0 ||
      m.indexOf("vp09") >= 0 ||
      m.indexOf("vp9") >= 0 ||
      m.indexOf("av01") >= 0 ||
      m.indexOf("opus") >= 0
    ) {
      return "probably";
    }
    return "";
  }

  _relinkChildren() {
    const elems = [];
    const n = this.childNodes.length;
    this.firstChild = n > 0 ? this.childNodes[0] : null;
    this.lastChild = n > 0 ? this.childNodes[n - 1] : null;
    for (let i = 0; i < n; i++) {
      const ch = this.childNodes[i];
      ch.previousSibling = i > 0 ? this.childNodes[i - 1] : null;
      ch.nextSibling = i + 1 < n ? this.childNodes[i + 1] : null;
      if (ch.nodeType === 1) {
        elems.push(ch);
      }
    }
    this.children = elems;
  }

  _setConnected(connected) {
    const wasConnected = this.isConnected;
    this.isConnected = connected;
    if (!wasConnected && connected && typeof this.connectedCallback === "function") {
      this.connectedCallback();
    } else if (
      wasConnected &&
      !connected &&
      typeof this.disconnectedCallback === "function"
    ) {
      this.disconnectedCallback();
    }
    for (let i = 0; i < this.childNodes.length; i++) {
      this.childNodes[i]._setConnected(connected);
    }
  }

  appendChild(child) {
    if (!child) return null;
    if (child.nodeType === 11) {
      const copy = [];
      for (let i = 0; i < child.childNodes.length; i++) {
        copy.push(child.childNodes[i]);
      }
      child.childNodes = [];
      child._relinkChildren();
      for (let i = 0; i < copy.length; i++) {
        this.appendChild(copy[i]);
      }
      return child;
    }
    if (child.parentNode) {
      child.parentNode.removeChild(child);
    }
    if (this.ownerDocument && child.ownerDocument !== this.ownerDocument) {
      this.ownerDocument._adoptExternalSubtree(child);
    }
    child.parentNode = this;
    child.parentElement = this.nodeType === 1 ? this : null;
    this.childNodes.push(child);
    this._relinkChildren();
    child._setConnected(this.isConnected);
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "appendChild",
        parentId: this._hostNodeId,
        childId: child._hostNodeId,
        tagName: child.tagName,
        nodeType: child.nodeType,
      });
      this.ownerDocument._notifyMutation("childList", this, [child], []);
    }
    return child;
  }

  removeChild(child) {
    const next = [];
    let found = false;
    for (let i = 0; i < this.childNodes.length; i++) {
      if (this.childNodes[i] === child) {
        found = true;
      } else {
        next.push(this.childNodes[i]);
      }
    }
    if (!found) return child;
    this.childNodes = next;
    child.parentNode = null;
    child.parentElement = null;
    child.previousSibling = null;
    child.nextSibling = null;
    this._relinkChildren();
    child._setConnected(false);
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "removeChild",
        parentId: this._hostNodeId,
        childId: child._hostNodeId,
      });
      this.ownerDocument._notifyMutation("childList", this, [], [child]);
    }
    return child;
  }

  insertBefore(newNode, refNode) {
    if (!refNode) {
      return this.appendChild(newNode);
    }
    if (newNode.parentNode) {
      newNode.parentNode.removeChild(newNode);
    }
    if (this.ownerDocument && newNode.ownerDocument !== this.ownerDocument) {
      this.ownerDocument._adoptExternalSubtree(newNode);
    }
    const next = [];
    for (let i = 0; i < this.childNodes.length; i++) {
      if (this.childNodes[i] === refNode) {
        next.push(newNode);
      }
      next.push(this.childNodes[i]);
    }
    newNode.parentNode = this;
    newNode.parentElement = this.nodeType === 1 ? this : null;
    this.childNodes = next;
    this._relinkChildren();
    newNode._setConnected(this.isConnected);
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "insertBefore",
        parentId: this._hostNodeId,
        childId: newNode._hostNodeId,
        refId: refNode._hostNodeId,
      });
    }
    return newNode;
  }

  replaceChild(newChild, oldChild) {
    this.insertBefore(newChild, oldChild);
    return this.removeChild(oldChild);
  }

  remove() {
    if (this.parentNode) {
      this.parentNode.removeChild(this);
    }
  }

  addEventListener(type, callback) {
    if (!this.listeners.has(type)) {
      this.listeners.set(type, []);
    }
    const list = this.listeners.get(type);
    list.push(callback);
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "addEventListener",
        nodeId: this._hostNodeId,
        childId: this._domNodeId,
        name: toStr(type),
      });
    }
  }

  setAttribute(name, value) {
    const k = anvilStringToLowerCase(toStr(name));
    const oldVal = this.getAttribute(k);
    const v = toStr(value);
    if (!this._attrs.has(k)) {
      this._attrKeys.push(k);
    }
    this._attrs.set(k, v);
    if (k === "id") {
      this.id = v;
    } else if (k === "class") {
      this._className = v;
      this.className = v;
      this.classList._syncFromAttr(v);
    } else if (k === "value") {
      this.value = v;
      this._lastSyncedValue = v;
    } else if (k === "src") {
      this.src = v;
    } else if (k === "width") {
      this.width = toNum(v);
    } else if (k === "height") {
      this.height = toNum(v);
    } else if (anvilStringStartsWith(k, "data-")) {
      this.dataset[attrToDatasetKey(k)] = v;
    }
    if (typeof this.attributeChangedCallback === "function") {
      this.attributeChangedCallback(k, oldVal, v);
    }
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "setAttribute",
        nodeId: this._hostNodeId,
        name: k,
        value: v,
      });
      this.ownerDocument._notifyMutation("attributes", this, [], []);
    }
  }

  getAttribute(name) {
    const k = anvilStringToLowerCase(toStr(name));
    if (k === "id" && this.id.length > 0) return this.id;
    if (k === "class" && this._className.length > 0) return this._className;
    if (!this._attrs.has(k)) return null;
    return this._attrs.get(k);
  }

  hasAttribute(name) {
    const k = anvilStringToLowerCase(toStr(name));
    return this._attrs.has(k) || (k === "id" && this.id.length > 0);
  }

  removeAttribute(name) {
    const k = anvilStringToLowerCase(toStr(name));
    this._attrs.delete(k);
    if (k === "id") this.id = "";
    if (k === "class") {
      this._className = "";
      this.className = "";
      this.classList._syncFromAttr("");
    }
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "removeAttribute",
        nodeId: this._hostNodeId,
        name: k,
      });
    }
  }

  setTextContent(text) {
    this._text = toStr(text);
    this.textContent = this._text;
    this._lastSyncedText = this._text;
    this.childNodes = [];
    this._relinkChildren();
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "setTextContent",
        nodeId: this._hostNodeId,
        text: this._text,
      });
      this.ownerDocument._notifyMutation("childList", this, [], []);
    }
  }

  setInnerHTML(html) {
    this._html = toStr(html);
    this.innerHTML = this._html;
    this._lastSyncedHtml = this._html;
    this._text = "";
    this.textContent = "";
    this._lastSyncedText = "";
    this.childNodes = [];
    this._relinkChildren();
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "setInnerHTML",
        nodeId: this._hostNodeId,
        value: this._html,
      });
      this.ownerDocument._notifyMutation("childList", this, [], []);
    }
  }

  setValue(val) {
    const v = toStr(val);
    this.value = v;
    this._lastSyncedValue = v;
    this._attrs.set("value", v);
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "setValue",
        nodeId: this._hostNodeId,
        value: v,
      });
    }
  }

  getTextContent() {
    if (this.childNodes.length === 0) {
      if (this.textContent && this.textContent.length > 0 && this._text.length === 0) {
        return toStr(this.textContent);
      }
      return this._text;
    }
    let out = this._text;
    for (let i = 0; i < this.childNodes.length; i++) {
      out = out + this.childNodes[i].getTextContent();
    }
    return out;
  }

  getElementById(id) {
    return this.querySelector("#" + toStr(id));
  }

  attachShadow(init) {
    const mode = init && init.mode ? toStr(init.mode) : "open";
    const root = new AnvilDOMNode(11, "#shadow-root", this.ownerDocument);
    root.mode = mode;
    root.host = this;
    root.isConnected = this.isConnected;
    if (mode === "open") {
      this.shadowRoot = root;
    }
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "attachShadow",
        hostId: this._hostNodeId,
        shadowRootId: root._hostNodeId,
        mode: mode,
      });
    }
    return root;
  }

  matches(selector) {
    if (this.nodeType !== 1) return false;
    const sel = anvilStringTrim(toStr(selector));
    if (sel === "" || sel === "*") return true;
    if (anvilStringStartsWith(sel, "#")) {
      return this.id === sel.slice(1);
    }
    if (anvilStringStartsWith(sel, ".")) {
      return this.classList.contains(sel.slice(1));
    }
    if (anvilStringStartsWith(sel, "[") && sel[sel.length - 1] === "]") {
      const inner = sel.slice(1, sel.length - 1);
      const eq = inner.indexOf("=");
      if (eq < 0) {
        return this.hasAttribute(inner);
      }
      const attr = anvilStringTrim(inner.slice(0, eq));
      let val = anvilStringTrim(inner.slice(eq + 1));
      if (
        val.length >= 2 &&
        ((val[0] === '"' && val[val.length - 1] === '"') ||
          (val[0] === "'" && val[val.length - 1] === "'"))
      ) {
        val = val.slice(1, val.length - 1);
      }
      return this.getAttribute(attr) === val;
    }
    const dot = sel.indexOf(".");
    if (dot > 0) {
      const tag = anvilStringToLowerCase(sel.slice(0, dot));
      const cls = sel.slice(dot + 1);
      return this.tagName === tag && this.classList.contains(cls);
    }
    return this.tagName === anvilStringToLowerCase(sel);
  }

  closest(selector) {
    let cur = this;
    while (cur) {
      if (cur.nodeType === 1 && cur.matches(selector)) {
        return cur;
      }
      cur = cur.parentElement || cur.parentNode;
    }
    return null;
  }

  querySelector(selector) {
    const all = this.querySelectorAll(selector);
    return all.length > 0 ? all[0] : null;
  }

  querySelectorAll(selector) {
    const out = [];
    const parts = anvilStringSplit(toStr(selector), ",");
    const selectors = [];
    for (let i = 0; i < parts.length; i++) {
      const p = anvilStringTrim(parts[i]);
      if (p.length > 0) selectors.push(p);
    }
    this._collectMatching(selectors, out);
    return out;
  }

  _collectMatching(selectors, out) {
    for (let i = 0; i < this.childNodes.length; i++) {
      const ch = this.childNodes[i];
      if (ch.nodeType === 1) {
        for (let j = 0; j < selectors.length; j++) {
          if (ch.matches(selectors[j])) {
            out.push(ch);
            break;
          }
        }
        ch._collectMatching(selectors, out);
      }
    }
  }

  getElementsByTagName(tagName) {
    const wanted = anvilStringToLowerCase(anvilStringTrim(toStr(tagName)));
    const out = [];
    this._collectByTag(wanted, out);
    return out;
  }

  _collectByTag(wanted, out) {
    for (let i = 0; i < this.childNodes.length; i++) {
      const ch = this.childNodes[i];
      if (ch.nodeType === 1) {
        if (wanted === "*" || ch.tagName === wanted) {
          out.push(ch);
        }
        ch._collectByTag(wanted, out);
      }
    }
  }

  getElementsByClassName(classNames) {
    const parts = anvilStringSplit(anvilStringTrim(toStr(classNames)), " ");
    const tokens = [];
    for (let i = 0; i < parts.length; i++) {
      const p = anvilStringTrim(parts[i]);
      if (p.length > 0) tokens.push(p);
    }
    if (tokens.length === 0) return [];
    const out = [];
    this._collectByClasses(tokens, out);
    return out;
  }

  _collectByClasses(tokens, out) {
    for (let i = 0; i < this.childNodes.length; i++) {
      const ch = this.childNodes[i];
      if (ch.nodeType === 1) {
        let allMatch = true;
        for (let j = 0; j < tokens.length; j++) {
          if (!ch.classList.contains(tokens[j])) {
            allMatch = false;
            break;
          }
        }
        if (allMatch) {
          out.push(ch);
        }
        ch._collectByClasses(tokens, out);
      }
    }
  }

  insertAdjacentElement(position, element) {
    if (!element) return null;
    const pos = anvilStringToLowerCase(anvilStringTrim(toStr(position)));
    if (pos === "beforebegin") {
      if (!this.parentNode) return null;
      return this.parentNode.insertBefore(element, this);
    }
    if (pos === "afterbegin") {
      return this.insertBefore(element, this.firstChild);
    }
    if (pos === "beforeend") {
      return this.appendChild(element);
    }
    if (pos === "afterend") {
      if (!this.parentNode) return null;
      return this.parentNode.insertBefore(element, this.nextSibling);
    }
    return null;
  }

  insertAdjacentText(position, text) {
    if (!this.ownerDocument) return;
    const tn = this.ownerDocument.createTextNode(text);
    this.insertAdjacentElement(position, tn);
  }

  insertAdjacentHTML(position, html) {
    const pos = anvilStringToLowerCase(anvilStringTrim(toStr(position)));
    const raw = toStr(html);
    if (this.ownerDocument) {
      this.ownerDocument._recordDomOp({
        op: "insertAdjacentHTML",
        nodeId: this._hostNodeId,
        name: pos,
        value: raw,
      });
      this.ownerDocument._notifyMutation("childList", this, [], []);
    }
  }

  getBoundingClientRect() {
    return {
      x: 0,
      y: 0,
      top: 0,
      left: 0,
      right: 800,
      bottom: 600,
      width: 800,
      height: 600,
    };
  }
}

export class AnvilCustomElementRegistry {
  constructor(doc) {
    this._doc = doc;
    this._defs = new Map();
    this._promises = new Map();
  }

  define(name, ctor) {
    const k = anvilStringToLowerCase(toStr(name));
    this._defs.set(k, ctor);
    if (this._doc) {
      this._doc._recordDomOp({
        op: "customElementDefine",
        name: k,
      });
      this.upgrade(this._doc.documentElement);
    }
  }

  get(name) {
    const k = anvilStringToLowerCase(toStr(name));
    if (!this._defs.has(k)) return undefined;
    return this._defs.get(k);
  }

  whenDefined(name) {
    return anvilPromiseResolve(this.get(name));
  }

  upgrade(rootNode) {
    if (!rootNode) return;
    if (rootNode.nodeType === 1) {
      const ctor = this.get(rootNode.tagName);
      if (ctor && !rootNode._upgraded) {
        rootNode._upgraded = true;
        if (typeof rootNode.connectedCallback === "function" && rootNode.isConnected) {
          rootNode.connectedCallback();
        }
      }
    }
    for (let i = 0; i < rootNode.childNodes.length; i++) {
      this.upgrade(rootNode.childNodes[i]);
    }
  }
}

export class AnvilMutationObserver {
  constructor(callback) {
    this._cb = callback;
    this._records = [];
    this._targets = [];
  }

  observe(target, options) {
    this._targets.push({ target: target, options: options || {} });
    if (target && target.ownerDocument) {
      target.ownerDocument._registerMutationObserver(this);
    }
  }

  disconnect() {
    this._targets = [];
    this._records = [];
  }

  takeRecords() {
    const r = this._records;
    this._records = [];
    return r;
  }

  _notify(record) {
    let match = false;
    for (let i = 0; i < this._targets.length; i++) {
      const entry = this._targets[i];
      if (entry.target === record.target || entry.options.subtree) {
        match = true;
        break;
      }
    }
    if (!match) return;
    this._records.push(record);
    if (typeof this._cb === "function") {
      const batch = this.takeRecords();
      const cb = this._cb;
      cb(batch, this);
    }
  }
}

export class AnvilIntersectionObserver {
  constructor(callback, options) {
    this._cb = callback;
    this.options = options || {};
    this._observed = [];
  }

  observe(target) {
    this._observed.push(target);
    if (typeof this._cb === "function") {
      const entry = {
        target: target,
        isIntersecting: true,
        intersectionRatio: 1,
        boundingClientRect: target ? target.getBoundingClientRect() : null,
      };
      const cb = this._cb;
      cb([entry], this);
    }
  }

  unobserve(target) {
    const next = [];
    for (let i = 0; i < this._observed.length; i++) {
      if (this._observed[i] !== target) next.push(this._observed[i]);
    }
    this._observed = next;
  }

  disconnect() {
    this._observed = [];
  }
}

export class AnvilResizeObserver {
  constructor(callback) {
    this._cb = callback;
    this._observed = [];
  }

  observe(target) {
    this._observed.push(target);
    if (typeof this._cb === "function") {
      const entry = {
        target: target,
        contentRect: { x: 0, y: 0, width: 800, height: 600 },
      };
      const cb = this._cb;
      cb([entry], this);
    }
  }

  unobserve(target) {
    const next = [];
    for (let i = 0; i < this._observed.length; i++) {
      if (this._observed[i] !== target) next.push(this._observed[i]);
    }
    this._observed = next;
  }

  disconnect() {
    this._observed = [];
  }
}

export class AnvilSourceBuffer extends AnvilEventTarget {
  constructor(mimeType) {
    super();
    this.mimeType = toStr(mimeType);
    this.mode = "segments";
    this.timestampOffset = 0;
    this.appendWindowStart = 0;
    this.appendWindowEnd = 1000000;
    this.updating = false;
    this.buffers = [];
    this.segments = this.buffers;
    this.buffered = {
      length: 1,
      start: () => 0,
      end: () => 10,
    };
  }

  appendBuffer(data) {
    this.updating = true;
    this.buffers.push(data);
    this.updating = false;
    this.dispatchEvent(new AnvilCustomEvent("updateend", {}));
  }

  remove(start, end) {
    this.updating = true;
    this._lastRemove = { start: toNum(start), end: toNum(end) };
    this.updating = false;
    this.dispatchEvent(new AnvilCustomEvent("updateend", {}));
  }
}

export class AnvilMediaSource extends AnvilEventTarget {
  constructor() {
    super();
    this.readyState = "open";
    this.sourceBuffers = [];
    this.activeSourceBuffers = this.sourceBuffers;
    this.duration = 0;
  }

  addSourceBuffer(mimeType) {
    const sb = new AnvilSourceBuffer(mimeType);
    this.sourceBuffers.push(sb);
    return sb;
  }

  removeSourceBuffer(sb) {
    const next = [];
    for (let i = 0; i < this.sourceBuffers.length; i++) {
      if (this.sourceBuffers[i] !== sb) {
        next.push(this.sourceBuffers[i]);
      }
    }
    this.sourceBuffers = next;
    this.activeSourceBuffers = next;
  }

  endOfStream() {
    this.readyState = "ended";
    this.dispatchEvent(new AnvilCustomEvent("sourceended", {}));
  }

  static isTypeSupported(mimeType) {
    const m = anvilStringToLowerCase(toStr(mimeType || ""));
    return (
      m.indexOf("mp4") >= 0 ||
      m.indexOf("webm") >= 0 ||
      m.indexOf("avc1") >= 0 ||
      m.indexOf("vp09") >= 0 ||
      m.indexOf("vp9") >= 0 ||
      m.indexOf("av01") >= 0 ||
      m.indexOf("opus") >= 0 ||
      m.indexOf("flac") >= 0
    );
  }
}

export class AnvilVideoFrame {
  constructor(data, init) {
    const opts = init || data || {};
    this.codedWidth = toNum(opts.codedWidth || opts.width || 64);
    this.codedHeight = toNum(opts.codedHeight || opts.height || 64);
    this.displayWidth = toNum(opts.displayWidth || this.codedWidth);
    this.displayHeight = toNum(opts.displayHeight || this.codedHeight);
    this.timestamp = toNum(opts.timestamp || 0);
    this.duration = toNum(opts.duration || 33333);
    this.format = opts.format ? toStr(opts.format) : "RGBA";
    this.data = data && typeof data.length === "number" ? data : [];
    this._closed = false;
  }

  allocationSize() {
    return this.codedWidth * this.codedHeight * 4;
  }

  copyTo(dest) {
    if (dest && typeof dest.length === "number") {
      const n = dest.length < this.data.length ? dest.length : this.data.length;
      for (let i = 0; i < n; i++) {
        dest[i] = this.data[i];
      }
    }
    return anvilPromiseResolve([
      { offset: 0, stride: this.codedWidth * 4 },
    ]);
  }

  close() {
    this._closed = true;
  }
}

export class AnvilEncodedVideoChunk {
  constructor(init) {
    const opts = init || {};
    this.type = opts.type ? toStr(opts.type) : "key";
    this.timestamp = toNum(opts.timestamp || 0);
    this.duration = toNum(opts.duration || 33333);
    this.data = opts.data || [];
    this.byteLength = this.data.length ? toNum(this.data.length) : 0;
  }

  copyTo(dest) {
    if (dest && typeof dest.length === "number") {
      const n = dest.length < this.data.length ? dest.length : this.data.length;
      for (let i = 0; i < n; i++) {
        dest[i] = this.data[i];
      }
    }
  }
}

export class AnvilVideoEncoder {
  constructor(init) {
    const opts = init || {};
    this._output = opts.output || null;
    this._error = opts.error || null;
    this.state = "unconfigured";
    this.encodeQueueSize = 0;
    this._config = null;
    this._encodedCount = 0;
  }

  configure(config) {
    this._config = config || { codec: "avc1.640028", width: 64, height: 64 };
    this.state = "configured";
  }

  encode(frame, options) {
    if (this.state !== "configured" || !frame) return;
    const isKey =
      this._encodedCount === 0 || !!(options && options.keyFrame);
    this._encodedCount = this._encodedCount + 1;
    const chunk = new AnvilEncodedVideoChunk({
      type: isKey ? "key" : "delta",
      timestamp: frame.timestamp || 0,
      duration: frame.duration || 33333,
      data: [0, 0, 0, 1, 103, 65, 68, 67, 84],
    });
    if (typeof this._output === "function") {
      const cb = this._output;
      cb(chunk, {
        decoderConfig: {
          codec: this._config.codec || "avc1.640028",
          codedWidth: frame.codedWidth || this._config.width || 64,
          codedHeight: frame.codedHeight || this._config.height || 64,
        },
      });
    }
  }

  flush() {
    return anvilPromiseResolve(undefined);
  }

  close() {
    this.state = "closed";
  }

  static isConfigSupported(config) {
    const c = config || {};
    const codec = anvilStringToLowerCase(toStr(c.codec || ""));
    const ok =
      codec.indexOf("avc1") >= 0 ||
      codec.indexOf("vp09") >= 0 ||
      codec.indexOf("vp9") >= 0 ||
      codec.indexOf("av01") >= 0 ||
      codec.indexOf("avid") >= 0;
    return anvilPromiseResolve({ supported: ok, config: c });
  }
}

export class AnvilVideoDecoder {
  constructor(init) {
    const opts = init || {};
    this._output = opts.output || null;
    this._error = opts.error || null;
    this.state = "unconfigured";
    this.decodeQueueSize = 0;
    this._config = null;
    this._decodedCount = 0;
  }

  configure(config) {
    this._config = config || { codec: "avc1.640028", codedWidth: 64, codedHeight: 64 };
    this.state = "configured";
  }

  decode(chunk) {
    if (this.state !== "configured" || !chunk) return;
    this._decodedCount = this._decodedCount + 1;
    const w = this._config.codedWidth || 64;
    const h = this._config.codedHeight || 64;
    const vf = new AnvilVideoFrame(null, {
      codedWidth: w,
      codedHeight: h,
      timestamp: chunk.timestamp || 0,
      duration: chunk.duration || 33333,
    });
    if (typeof this._output === "function") {
      const cb = this._output;
      cb(vf);
    }
  }

  flush() {
    return anvilPromiseResolve(undefined);
  }

  close() {
    this.state = "closed";
  }

  static isConfigSupported(config) {
    return AnvilVideoEncoder.isConfigSupported(config);
  }
}

export class AnvilMediaRecorder extends AnvilEventTarget {
  constructor(stream, options) {
    super();
    const opts = options || {};
    this.stream = stream || null;
    this.mimeType = opts.mimeType ? toStr(opts.mimeType) : "video/webm;codecs=vp9";
    this.state = "inactive";
    this.ondataavailable = null;
    this.onstop = null;
  }

  start() {
    this.state = "recording";
    this.dispatchEvent(new AnvilCustomEvent("start", {}));
  }

  stop() {
    this.state = "inactive";
    const ev = new AnvilCustomEvent("dataavailable", {
      detail: { size: 64, type: this.mimeType },
    });
    ev.data = { size: 64, type: this.mimeType };
    if (typeof this.ondataavailable === "function") {
      const cb = this.ondataavailable;
      cb(ev);
    }
    this.dispatchEvent(ev);
    const stopEv = new AnvilCustomEvent("stop", {});
    if (typeof this.onstop === "function") {
      const scb = this.onstop;
      scb(stopEv);
    }
    this.dispatchEvent(stopEv);
  }

  pause() {
    this.state = "paused";
  }

  resume() {
    this.state = "recording";
  }

  requestData() {
    if (typeof this.ondataavailable === "function") {
      const ev = new AnvilCustomEvent("dataavailable", {
        detail: { size: 32, type: this.mimeType },
      });
      ev.data = { size: 32, type: this.mimeType };
      const cb = this.ondataavailable;
      cb(ev);
    }
  }

  static isTypeSupported(mimeType) {
    return AnvilMediaSource.isTypeSupported(mimeType);
  }
}

export class AnvilAudioContext {
  constructor() {
    this.sampleRate = 48000;
    this.currentTime = 0;
    this.state = "running";
    this.destination = { maxChannelCount: 2 };
  }

  createGain() {
    return {
      gain: { value: 1, setValueAtTime: () => {} },
      connect: (dest) => dest,
      disconnect: () => {},
    };
  }

  createOscillator() {
    return {
      type: "sine",
      frequency: { value: 440, setValueAtTime: () => {} },
      connect: (dest) => dest,
      start: () => {},
      stop: () => {},
    };
  }

  createBuffer(channels, length, sampleRate) {
    const ch = toNum(channels || 2);
    const len = toNum(length || 1024);
    const sr = toNum(sampleRate || 48000);
    const buf = [];
    for (let i = 0; i < len; i++) {
      buf.push(0);
    }
    return {
      numberOfChannels: ch,
      length: len,
      sampleRate: sr,
      duration: len / sr,
      getChannelData: () => buf,
    };
  }

  createBufferSource() {
    return {
      buffer: null,
      loop: false,
      connect: (dest) => dest,
      start: () => {},
      stop: () => {},
    };
  }

  decodeAudioData() {
    return anvilPromiseResolve(this.createBuffer(2, 1024, 48000));
  }

  resume() {
    this.state = "running";
    return anvilPromiseResolve(undefined);
  }

  suspend() {
    this.state = "suspended";
    return anvilPromiseResolve(undefined);
  }

  close() {
    this.state = "closed";
    return anvilPromiseResolve(undefined);
  }
}

export class AnvilDOMDocument extends AnvilEventTarget {
  constructor(initialUrl) {
    super();
    this.nodeType = 9;
    this.title = "";
    this._lastSyncedTitle = "";
    this.readyState = "complete";
    this.visibilityState = "visible";
    this.hidden = false;
    this.cookie = "";
    this.adoptedStyleSheets = [];
    this._lastSyncedAdoptedCss = "";
    this._observers = [];
    this._nextHostNodeId = 1;
    this._domMutations = [];
    this._recordingEnabled = false;
    this.customElements = null;
    this.documentElement = new AnvilDOMNode(1, "html", this);
    this.documentElement.isConnected = true;
    this.head = new AnvilDOMNode(1, "head", this);
    this.body = new AnvilDOMNode(1, "body", this);
    this.documentElement.appendChild(this.head);
    this.documentElement.appendChild(this.body);
    this._recordingEnabled = true;
    this.location = new AnvilLocation(initialUrl || "https://localhost/", this);
    this.URL = this.location.href;
    this.documentURI = this.location.href;
    this.domain = this.location.hostname;
  }

  addEventListener(type, callback) {
    if (!this.listeners.has(type)) {
      this.listeners.set(type, []);
    }
    const list = this.listeners.get(type);
    list.push(callback);
    this._recordDomOp({
      op: "addEventListener",
      nodeId: 3,
      childId: 0,
      name: toStr(type),
    });
  }

  _allocNodeId() {
    const id = this._nextHostNodeId;
    this._nextHostNodeId = id + 1;
    return id;
  }

  _recordDomOp(opObj) {
    if (!this._recordingEnabled) return;
    this._domMutations.push(opObj);
  }

  takeDomMutations() {
    const out = this._domMutations;
    this._domMutations = [];
    return out;
  }

  seedHostNode(
    parentHostNodeId,
    domNodeId,
    tagName,
    id,
    className,
    textContent,
    valueAttr
  ) {
    const prevRec = this._recordingEnabled;
    this._recordingEnabled = false;
    const node = new AnvilDOMNode(1, tagName, this);
    node._domNodeId = toNum(domNodeId || 0);
    if (id && id.length > 0) {
      node.id = toStr(id);
      node._attrs.set("id", node.id);
      node._attrKeys.push("id");
    }
    if (className && className.length > 0) {
      const cls = toStr(className);
      node._className = cls;
      node.className = cls;
      node._lastSyncedClass = cls;
      node._attrs.set("class", cls);
      node._attrKeys.push("class");
      node.classList._syncFromAttr(cls);
    }
    if (textContent && textContent.length > 0) {
      const txt = toStr(textContent);
      node._text = txt;
      node.textContent = txt;
      node._lastSyncedText = txt;
    }
    if (valueAttr && valueAttr.length > 0) {
      const val = toStr(valueAttr);
      node.value = val;
      node._lastSyncedValue = val;
      node._attrs.set("value", val);
      node._attrKeys.push("value");
    }
    let parent = this.body;
    const pid = toNum(parentHostNodeId || 3);
    if (pid === 1) {
      parent = this.documentElement;
    } else if (pid === 2) {
      parent = this.head;
    } else if (pid === 3) {
      parent = this.body;
    } else {
      const foundParent = this._findByHostNodeId(this.documentElement, pid);
      if (foundParent) {
        parent = foundParent;
      }
    }
    parent.appendChild(node);
    this._recordingEnabled = prevRec;
    this._recordDomOp({
      op: "bindSeedNode",
      nodeId: node._hostNodeId,
      childId: node._domNodeId,
      htmlId: node.id,
    });
    return node._hostNodeId;
  }

  seedHostAttr(vmHostNodeId, name, value) {
    const node = this._findByHostNodeId(this.documentElement, toNum(vmHostNodeId));
    if (!node) return;
    const prevRec = this._recordingEnabled;
    this._recordingEnabled = false;
    node.setAttribute(name, value);
    this._recordingEnabled = prevRec;
  }

  _findByHostNodeId(node, targetHostId) {
    if (!node) return null;
    if (node._hostNodeId === targetHostId) {
      return node;
    }
    if (node.shadowRoot) {
      const inShadow = this._findByHostNodeId(node.shadowRoot, targetHostId);
      if (inShadow) return inShadow;
    }
    for (let i = 0; i < node.childNodes.length; i++) {
      const found = this._findByHostNodeId(node.childNodes[i], targetHostId);
      if (found) return found;
    }
    return null;
  }

  findByDomNodeId(domNodeId) {
    return this._findByDomNodeIdIn(this.documentElement, toNum(domNodeId));
  }

  _findByDomNodeIdIn(node, targetDomId) {
    if (!node) return null;
    if (node._domNodeId === targetDomId && targetDomId > 0) {
      return node;
    }
    if (node.shadowRoot) {
      const inShadow = this._findByDomNodeIdIn(node.shadowRoot, targetDomId);
      if (inShadow) return inShadow;
    }
    for (let i = 0; i < node.childNodes.length; i++) {
      const found = this._findByDomNodeIdIn(node.childNodes[i], targetDomId);
      if (found) return found;
    }
    return null;
  }

  syncDomTree() {
    if (this.title && this.title !== this._lastSyncedTitle) {
      this._lastSyncedTitle = this.title;
      this._recordDomOp({ op: "setTitle", title: this.title });
    }
    if (this.adoptedStyleSheets && this.adoptedStyleSheets.length > 0) {
      let combinedCss = "";
      for (let sIdx = 0; sIdx < this.adoptedStyleSheets.length; sIdx++) {
        const sh = this.adoptedStyleSheets[sIdx];
        if (sh && typeof sh.cssText === "function") {
          const text = sh.cssText();
          if (text.length > 0) {
            if (combinedCss.length > 0) combinedCss = combinedCss + "\n";
            combinedCss = combinedCss + text;
          }
        } else if (sh && sh.cssRules && sh.cssRules.length > 0) {
          for (let rIdx = 0; rIdx < sh.cssRules.length; rIdx++) {
            const r = sh.cssRules[rIdx];
            if (r && r.cssText) {
              if (combinedCss.length > 0) combinedCss = combinedCss + "\n";
              combinedCss = combinedCss + toStr(r.cssText);
            }
          }
        }
      }
      if (combinedCss.length > 0 && combinedCss !== this._lastSyncedAdoptedCss) {
        this._lastSyncedAdoptedCss = combinedCss;
        this._recordDomOp({ op: "adoptStyleSheets", value: combinedCss });
      }
    }
    this._syncNodeRecursive(this.documentElement);
  }

  _adoptExternalSubtree(node) {
    if (!node) return null;
    node.ownerDocument = this;
    node._hostNodeId = this._allocNodeId();
    if (node.nodeType === 3) {
      this._recordDomOp({
        op: "createTextNode",
        nodeId: node._hostNodeId,
        text: node._text || node.textContent || "",
      });
      return node;
    }
    if (node.nodeType === 1) {
      this._recordDomOp({
        op: "createElement",
        nodeId: node._hostNodeId,
        tagName: node.tagName || "div",
      });
      for (let aIdx = 0; aIdx < node._attrKeys.length; aIdx++) {
        const k = node._attrKeys[aIdx];
        if (node._attrs.has(k)) {
          this._recordDomOp({
            op: "setAttribute",
            nodeId: node._hostNodeId,
            name: k,
            value: node._attrs.get(k),
          });
        }
      }
      if (node._className && node._className.length > 0) {
        node._lastSyncedClass = node._className;
        this._recordDomOp({
          op: "setClassName",
          nodeId: node._hostNodeId,
          className: node._className,
        });
      }
      if (node._text && node._text.length > 0 && node.childNodes.length === 0) {
        node._lastSyncedText = node._text;
        this._recordDomOp({
          op: "setTextContent",
          nodeId: node._hostNodeId,
          text: node._text,
        });
      }
      for (let cIdx = 0; cIdx < node.childNodes.length; cIdx++) {
        const ch = node.childNodes[cIdx];
        this._adoptExternalSubtree(ch);
        this._recordDomOp({
          op: "appendChild",
          parentId: node._hostNodeId,
          childId: ch._hostNodeId,
          tagName: ch.tagName,
          nodeType: ch.nodeType,
        });
      }
    }
    return node;
  }

  adoptNode(node) {
    if (!node) return null;
    if (node.parentNode) {
      node.parentNode.removeChild(node);
    }
    return this._adoptExternalSubtree(node);
  }

  importNode(node, deep) {
    if (!node) return null;
    return this._adoptExternalSubtree(node);
  }

  _syncNodeRecursive(node) {
    if (!node) return;
    if (
      typeof node.innerHTML === "string" &&
      node.innerHTML.length > 0 &&
      node.innerHTML !== node._lastSyncedHtml
    ) {
      node._lastSyncedHtml = node.innerHTML;
      this._recordDomOp({
        op: "setInnerHTML",
        nodeId: node._hostNodeId,
        value: node.innerHTML,
      });
    }
    if (
      typeof node.textContent === "string" &&
      node.textContent.length > 0 &&
      node.textContent !== node._lastSyncedText
    ) {
      node._lastSyncedText = node.textContent;
      this._recordDomOp({
        op: "setTextContent",
        nodeId: node._hostNodeId,
        text: node.textContent,
      });
    }
    if (
      typeof node.value === "string" &&
      node.value.length > 0 &&
      node.value !== node._lastSyncedValue
    ) {
      node._lastSyncedValue = node.value;
      this._recordDomOp({
        op: "setValue",
        nodeId: node._hostNodeId,
        value: node.value,
      });
    }
    if (
      typeof node.className === "string" &&
      node.className.length > 0 &&
      node.className !== node._className
    ) {
      node._className = node.className;
      node.classList._syncFromAttr(node.className);
    }
    if (
      typeof node._className === "string" &&
      node._className.length > 0 &&
      node._className !== node._lastSyncedClass
    ) {
      node._lastSyncedClass = node._className;
      this._recordDomOp({
        op: "setClassName",
        nodeId: node._hostNodeId,
        className: node._className,
      });
    }
    if (node.style) {
      const sKeys = [
        "backgroundColor",
        "color",
        "display",
        "visibility",
        "width",
        "height",
        "opacity",
        "border",
        "margin",
        "padding",
        "position",
        "top",
        "left",
        "fontSize",
        "fontWeight",
      ];
      for (let sIdx = 0; sIdx < sKeys.length; sIdx++) {
        const prop = sKeys[sIdx];
        const val = node.style[prop];
        if (typeof val === "string" && val.length > 0) {
          this._recordDomOp({
            op: "setStyle",
            nodeId: node._hostNodeId,
            name: prop,
            value: val,
          });
        }
      }
    }
    const evTypes = ["click", "input", "change", "submit", "keydown"];
    for (let eIdx = 0; eIdx < evTypes.length; eIdx++) {
      const et = evTypes[eIdx];
      if (typeof node["on" + et] === "function") {
        this._recordDomOp({
          op: "addEventListener",
          nodeId: node._hostNodeId,
          childId: node._domNodeId,
          name: et,
        });
      }
    }
    if (node.shadowRoot) {
      this._syncNodeRecursive(node.shadowRoot);
    }
    if (node.childNodes && node.childNodes.length) {
      for (let cIdx = 0; cIdx < node.childNodes.length; cIdx++) {
        this._syncNodeRecursive(node.childNodes[cIdx]);
      }
    }
  }

  dispatchHostEvent(domNodeId, eventType, eventInit) {
    const typeStr = toStr(eventType || "click");
    let target = this.findByDomNodeId(domNodeId);
    if (!target) {
      target = this.body;
    }
    if (eventInit && eventInit.value !== undefined) {
      const nextVal = toStr(eventInit.value);
      target.value = nextVal;
      target._lastSyncedValue = nextVal;
      target._attrs.set("value", nextVal);
    }
    const ev = new AnvilCustomEvent(typeStr, {
      bubbles: true,
      cancelable: true,
      detail: eventInit || null,
    });
    ev.target = target;
    ev.value = target.value;
    ev.key = eventInit && eventInit.key ? toStr(eventInit.key) : "";
    let handled = false;
    let cur = target;
    while (cur) {
      ev.currentTarget = cur;
      const inlineProp = cur["on" + typeStr];
      if (typeof inlineProp === "function") {
        handled = true;
        inlineProp(ev);
      }
      if (cur.listeners && cur.listeners.has(typeStr)) {
        const list = cur.listeners.get(typeStr);
        if (list && list.length > 0) {
          handled = true;
        }
      }
      cur.dispatchEvent(ev);
      if (ev.propagationStopped) {
        break;
      }
      cur = cur.parentNode;
    }
    if (!ev.propagationStopped) {
      ev.currentTarget = this;
      const docProp = this["on" + typeStr];
      if (typeof docProp === "function") {
        handled = true;
        docProp(ev);
      }
      if (this.listeners && this.listeners.has(typeStr)) {
        const list = this.listeners.get(typeStr);
        if (list && list.length > 0) {
          handled = true;
        }
      }
      this.dispatchEvent(ev);
    }
    this._lastEventHandled = handled;
    this._lastEventDefaultPrevented = ev.defaultPrevented;
    if (ev.defaultPrevented) {
      this._recordDomOp({
        op: "preventDefault",
        nodeId: target._hostNodeId,
        childId: target._domNodeId,
        name: typeStr,
      });
    }
    this.syncDomTree();
    return !ev.defaultPrevented;
  }

  exportDomMutationRows() {
    this.syncDomTree();
    const rawMuts = this.takeDomMutations();
    const rows = [];
    for (let mIdx = 0; mIdx < rawMuts.length; mIdx++) {
      const m = rawMuts[mIdx];
      const c = m.cmd;
      if (c) {
        rows.push([
          "canvasDraw",
          m.nodeId || 0,
          c.kind || "",
          c.x || c.dx || 0,
          c.y || c.dy || 0,
          c.w || c.width || 0,
          c.h || c.height || 0,
          c.style || c.text || "",
          c.lineWidth || 1,
        ]);
      } else {
        rows.push([
          m.op || "",
          m.nodeId || m.hostId || m.parentId || 0,
          m.childId || m.shadowRootId || m.refId || 0,
          m.tagName ||
            m.name ||
            m.htmlId ||
            m.title ||
            m.className ||
            m.mode ||
            "",
          m.value || m.text || m.html || "",
          m.refId || 0,
        ]);
      }
    }
    return rows;
  }

  serializeDomTree() {
    return this._serializeNode(this.documentElement);
  }

  _serializeNode(node) {
    if (!node) return null;
    const attrs = [];
    for (let i = 0; i < node._attrKeys.length; i++) {
      const k = node._attrKeys[i];
      if (node._attrs.has(k)) {
        attrs.push([k, node._attrs.get(k)]);
      }
    }
    const children = [];
    for (let i = 0; i < node.childNodes.length; i++) {
      children.push(this._serializeNode(node.childNodes[i]));
    }
    const canvasOps =
      node._ctx2d && node._ctx2d._ops ? node._ctx2d._ops : [];
    const shadow = node.shadowRoot ? this._serializeNode(node.shadowRoot) : null;
    return {
      hostNodeId: node._hostNodeId,
      nodeType: node.nodeType,
      tagName: node.tagName,
      id: node.id,
      className: node._className,
      text: node._text,
      attrs: attrs,
      style: node.style,
      width: node.width,
      height: node.height,
      paused: node.paused,
      src: node.src,
      canvasOps: canvasOps,
      shadowRoot: shadow,
      children: children,
    };
  }

  _registerMutationObserver(obs) {
    if (anvilArrayIndexOf(this._observers, obs) < 0) {
      this._observers.push(obs);
    }
  }

  _notifyMutation(type, target, addedNodes, removedNodes) {
    const rec = {
      type: type,
      target: target,
      addedNodes: addedNodes,
      removedNodes: removedNodes,
    };
    for (let i = 0; i < this._observers.length; i++) {
      this._observers[i]._notify(rec);
    }
  }

  createElement(tagName) {
    const node = new AnvilDOMNode(1, tagName, this);
    if (this.customElements) {
      const ctor = this.customElements.get(tagName);
      if (ctor) {
        node._upgraded = true;
      }
    }
    this._recordDomOp({
      op: "createElement",
      nodeId: node._hostNodeId,
      tagName: node.tagName,
    });
    return node;
  }

  createTextNode(text) {
    const node = new AnvilDOMNode(3, "#text", this);
    node._text = toStr(text);
    this._recordDomOp({
      op: "createTextNode",
      nodeId: node._hostNodeId,
      text: node._text,
    });
    return node;
  }

  createDocumentFragment() {
    return new AnvilDOMNode(11, "#document-fragment", this);
  }

  createComment(data) {
    const node = new AnvilDOMNode(8, "#comment", this);
    node._text = toStr(data === undefined ? "" : data);
    return node;
  }

  getElementsByTagName(tagName) {
    const wanted = anvilStringToLowerCase(anvilStringTrim(toStr(tagName)));
    const out = [];
    if (wanted === "*" || wanted === "html") {
      out.push(this.documentElement);
    }
    this.documentElement._collectByTag(wanted, out);
    return out;
  }

  getElementsByClassName(classNames) {
    return this.documentElement.getElementsByClassName(classNames);
  }

  getElementsByName(name) {
    const target = toStr(name);
    const out = [];
    this._collectByName(this.documentElement, target, out);
    return out;
  }

  _collectByName(node, target, out) {
    if (node.nodeType === 1 && node.getAttribute("name") === target) {
      out.push(node);
    }
    for (let i = 0; i < node.childNodes.length; i++) {
      this._collectByName(node.childNodes[i], target, out);
    }
  }

  getElementById(id) {
    const targetId = toStr(id);
    return this._findById(this.documentElement, targetId);
  }

  _findById(node, targetId) {
    if (node.nodeType === 1 && node.id === targetId) {
      return node;
    }
    for (let i = 0; i < node.childNodes.length; i++) {
      const found = this._findById(node.childNodes[i], targetId);
      if (found) return found;
    }
    return null;
  }

  querySelector(selector) {
    return this.documentElement.querySelector(selector);
  }

  querySelectorAll(selector) {
    return this.documentElement.querySelectorAll(selector);
  }
}

export class AnvilDOMRect {
  constructor(x, y, width, height) {
    this.x = toNum(x || 0);
    this.y = toNum(y || 0);
    this.width = toNum(width || 0);
    this.height = toNum(height || 0);
    this.left = this.width < 0 ? this.x + this.width : this.x;
    this.top = this.height < 0 ? this.y + this.height : this.y;
    this.right = this.width < 0 ? this.x : this.x + this.width;
    this.bottom = this.height < 0 ? this.y : this.y + this.height;
  }

  static fromRect(other) {
    if (!other) {
      return new AnvilDOMRect(0, 0, 0, 0);
    }
    return new AnvilDOMRect(other.x, other.y, other.width, other.height);
  }

  toJSON() {
    return {
      x: this.x,
      y: this.y,
      width: this.width,
      height: this.height,
      top: this.top,
      right: this.right,
      bottom: this.bottom,
      left: this.left,
    };
  }
}

export class AnvilDocumentFragment extends AnvilDOMNode {
  constructor(ownerDocument) {
    super(11, "#document-fragment", ownerDocument || null);
  }
}

function parseHtmlAttrsInto(el, attrSrc) {
  let pos = 0;
  const len = attrSrc.length;
  while (pos < len) {
    while (pos < len && attrSrc.charCodeAt(pos) <= 32) {
      pos = pos + 1;
    }
    if (pos >= len) break;
    const nameStart = pos;
    while (
      pos < len &&
      attrSrc.charCodeAt(pos) > 32 &&
      attrSrc[pos] !== "="
    ) {
      pos = pos + 1;
    }
    const attrName = anvilStringToLowerCase(attrSrc.slice(nameStart, pos));
    while (pos < len && attrSrc.charCodeAt(pos) <= 32) {
      pos = pos + 1;
    }
    let attrVal = "";
    if (pos < len && attrSrc[pos] === "=") {
      pos = pos + 1;
      while (pos < len && attrSrc.charCodeAt(pos) <= 32) {
        pos = pos + 1;
      }
      if (pos < len && (attrSrc[pos] === '"' || attrSrc[pos] === "'")) {
        const q = attrSrc[pos];
        pos = pos + 1;
        const valStart = pos;
        while (pos < len && attrSrc[pos] !== q) {
          pos = pos + 1;
        }
        attrVal = attrSrc.slice(valStart, pos);
        if (pos < len) pos = pos + 1;
      } else {
        const valStart = pos;
        while (pos < len && attrSrc.charCodeAt(pos) > 32) {
          pos = pos + 1;
        }
        attrVal = attrSrc.slice(valStart, pos);
      }
    }
    if (attrName.length > 0) {
      el.setAttribute(attrName, attrVal);
    }
  }
}

function parseHtmlFragmentInto(doc, rootParent, htmlStr) {
  const prevRec = doc._recordingEnabled;
  doc._recordingEnabled = false;
  const stack = [rootParent];
  let i = 0;
  const len = htmlStr.length;
  while (i < len) {
    const lt = htmlStr.indexOf("<", i);
    if (lt < 0) {
      const tail = htmlStr.slice(i);
      if (tail.length > 0) {
        const top = stack[stack.length - 1];
        top._text = top._text + tail;
        top.textContent = top._text;
        top._lastSyncedText = top._text;
      }
      break;
    }
    if (lt > i) {
      const txt = htmlStr.slice(i, lt);
      const top = stack[stack.length - 1];
      top._text = top._text + txt;
      top.textContent = top._text;
      top._lastSyncedText = top._text;
    }
    const gt = htmlStr.indexOf(">", lt + 1);
    if (gt < 0) break;
    const tagBody = anvilStringTrim(htmlStr.slice(lt + 1, gt));
    i = gt + 1;
    if (tagBody.length === 0 || tagBody[0] === "!" || tagBody[0] === "?") {
      continue;
    }
    if (tagBody[0] === "/") {
      if (stack.length > 1) {
        stack.pop();
      }
      continue;
    }
    const selfClosing = tagBody[tagBody.length - 1] === "/";
    const cleanTag = selfClosing
      ? anvilStringTrim(tagBody.slice(0, tagBody.length - 1))
      : tagBody;
    let sp = -1;
    for (let k = 0; k < cleanTag.length; k++) {
      if (cleanTag.charCodeAt(k) <= 32) {
        sp = k;
        break;
      }
    }
    const rawName = sp < 0 ? cleanTag : cleanTag.slice(0, sp);
    const tagName = anvilStringToLowerCase(rawName);
    const attrSrc = sp < 0 ? "" : anvilStringTrim(cleanTag.slice(sp + 1));
    if (tagName === "html" || tagName === "body" || tagName === "head") {
      continue;
    }
    if (tagName === "title") {
      const closeTitle = htmlStr.indexOf("<", i);
      if (closeTitle >= i) {
        doc.title = htmlStr.slice(i, closeTitle);
        doc._lastSyncedTitle = doc.title;
      }
      continue;
    }
    const el = new AnvilDOMNode(1, tagName, doc);
    if (attrSrc.length > 0) {
      parseHtmlAttrsInto(el, attrSrc);
    }
    const parent = stack[stack.length - 1];
    parent.appendChild(el);
    const isVoid =
      selfClosing ||
      tagName === "br" ||
      tagName === "hr" ||
      tagName === "img" ||
      tagName === "input" ||
      tagName === "meta" ||
      tagName === "link";
    if (!isVoid) {
      stack.push(el);
    }
  }
  doc._recordingEnabled = prevRec;
}

export class AnvilDOMParser {
  constructor() {}

  parseFromString(htmlText, mimeType) {
    const doc = new AnvilDOMDocument("https://localhost/");
    doc.contentType = mimeType ? toStr(mimeType) : "text/html";
    if (htmlText) {
      const raw = toStr(htmlText);
      doc.body._html = raw;
      doc.body.innerHTML = raw;
      doc.body._lastSyncedHtml = raw;
      parseHtmlFragmentInto(doc, doc.body, raw);
    }
    doc.takeDomMutations();
    return doc;
  }
}

export class AnvilFormData {
  constructor() {
    this._entries = [];
  }

  append(name, value) {
    this._entries.push([toStr(name), toStr(value)]);
  }

  set(name, value) {
    const k = toStr(name);
    const v = toStr(value);
    const next = [];
    let replaced = false;
    for (let i = 0; i < this._entries.length; i++) {
      const pair = this._entries[i];
      if (pair[0] === k) {
        if (!replaced) {
          next.push([k, v]);
          replaced = true;
        }
      } else {
        next.push(pair);
      }
    }
    if (!replaced) {
      next.push([k, v]);
    }
    this._entries = next;
  }

  get(name) {
    const k = toStr(name);
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === k) {
        return this._entries[i][1];
      }
    }
    return null;
  }

  getAll(name) {
    const k = toStr(name);
    const out = [];
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === k) {
        out.push(this._entries[i][1]);
      }
    }
    return out;
  }

  has(name) {
    return this.get(name) !== null;
  }

  delete(name) {
    const k = toStr(name);
    const next = [];
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] !== k) {
        next.push(this._entries[i]);
      }
    }
    this._entries = next;
  }

  entries() {
    return this._entries;
  }
}

export class AnvilBlob {
  constructor(parts, options) {
    this.type = options && options.type ? toStr(options.type) : "";
    let combined = "";
    if (parts && typeof parts.length === "number") {
      for (let i = 0; i < parts.length; i++) {
        const p = parts[i];
        if (p instanceof AnvilBlob) {
          combined = combined + p._data;
        } else if (p !== null && p !== undefined) {
          combined = combined + toStr(p);
        }
      }
    }
    this._data = combined;
    this.size = combined.length;
  }

  text() {
    return anvilPromiseResolve(this._data);
  }

  arrayBuffer() {
    const buf = new ArrayBuffer(this._data.length);
    const view = new Uint8Array(buf);
    for (let i = 0; i < this._data.length; i++) {
      view[i] = this._data.charCodeAt(i) & 255;
    }
    return anvilPromiseResolve(buf);
  }

  slice(start, end, contentType) {
    const s = start === undefined ? 0 : toNum(start);
    const e = end === undefined ? this._data.length : toNum(end);
    const sub = this._data.slice(s, e);
    return new AnvilBlob([sub], { type: contentType || this.type });
  }
}

export class AnvilTextEncoder {
  constructor() {
    this.encoding = "utf-8";
  }

  encode(input) {
    const str = input === undefined ? "" : toStr(input);
    const bytes = [];
    for (let i = 0; i < str.length; i++) {
      const c = str.charCodeAt(i);
      if (c < 128) {
        bytes.push(c);
      } else if (c < 2048) {
        bytes.push(192 | (c >> 6));
        bytes.push(128 | (c & 63));
      } else {
        bytes.push(224 | (c >> 12));
        bytes.push(128 | ((c >> 6) & 63));
        bytes.push(128 | (c & 63));
      }
    }
    const out = new Uint8Array(bytes.length);
    for (let i = 0; i < bytes.length; i++) {
      out[i] = bytes[i];
    }
    return out;
  }
}

export class AnvilTextDecoder {
  constructor(label) {
    this.encoding = label ? anvilStringToLowerCase(toStr(label)) : "utf-8";
  }

  decode(input) {
    if (!input || typeof input.length !== "number") {
      return "";
    }
    let out = "";
    let i = 0;
    const len = input.length;
    while (i < len) {
      const b0 = input[i] & 255;
      if (b0 < 128) {
        out = out + String.fromCharCode(b0);
        i = i + 1;
      } else if ((b0 & 224) === 192 && i + 1 < len) {
        const b1 = input[i + 1] & 63;
        out = out + String.fromCharCode(((b0 & 31) << 6) | b1);
        i = i + 2;
      } else if ((b0 & 240) === 224 && i + 2 < len) {
        const b1 = input[i + 1] & 63;
        const b2 = input[i + 2] & 63;
        out = out + String.fromCharCode(((b0 & 15) << 12) | (b1 << 6) | b2);
        i = i + 3;
      } else {
        out = out + String.fromCharCode(65533);
        i = i + 1;
      }
    }
    return out;
  }
}

export class AnvilPerformance extends AnvilEventTarget {
  constructor() {
    super();
    this.timeOrigin = 1700000000000;
    this._tick = 16;
    this._entries = [];
  }

  now() {
    this._tick = this._tick + 1;
    return this._tick;
  }

  mark(name) {
    const entry = {
      name: toStr(name),
      entryType: "mark",
      startTime: this.now(),
      duration: 0,
    };
    this._entries.push(entry);
    return entry;
  }

  measure(name) {
    const entry = {
      name: toStr(name),
      entryType: "measure",
      startTime: this.now(),
      duration: 1,
    };
    this._entries.push(entry);
    return entry;
  }

  getEntries() {
    return this._entries;
  }

  getEntriesByName(name) {
    const wanted = toStr(name);
    const out = [];
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i].name === wanted) {
        out.push(this._entries[i]);
      }
    }
    return out;
  }

  getEntriesByType(entryType) {
    const wanted = toStr(entryType);
    const out = [];
    for (let i = 0; i < this._entries.length; i++) {
      if (this._entries[i].entryType === wanted) {
        out.push(this._entries[i]);
      }
    }
    return out;
  }

  clearMarks() {
    this._entries = [];
  }

  clearMeasures() {
    this._entries = [];
  }
}

export class AnvilBroadcastChannel extends AnvilEventTarget {
  constructor(name) {
    super();
    this.name = toStr(name);
    this.onmessage = null;
    this._closed = false;
  }

  postMessage(message) {
    if (this._closed) return;
    const ev = new AnvilCustomEvent("message", { detail: message });
    ev.data = message;
    if (typeof this.onmessage === "function") {
      this.onmessage(ev);
    }
    this.dispatchEvent(ev);
  }

  close() {
    this._closed = true;
  }
}

export class AnvilMessagePort extends AnvilEventTarget {
  constructor() {
    super();
    this.onmessage = null;
    this._peer = null;
  }

  postMessage(message) {
    if (!this._peer) return;
    const ev = new AnvilCustomEvent("message", { detail: message });
    ev.data = message;
    if (typeof this._peer.onmessage === "function") {
      this._peer.onmessage(ev);
    }
    this._peer.dispatchEvent(ev);
  }

  start() {}

  close() {
    this._peer = null;
  }
}

export class AnvilMessageChannel {
  constructor() {
    this.port1 = new AnvilMessagePort();
    this.port2 = new AnvilMessagePort();
    this.port1._peer = this.port2;
    this.port2._peer = this.port1;
  }
}

export class AnvilCSSStyleSheet {
  constructor() {
    this.cssRules = [];
  }

  insertRule(ruleText, index) {
    const idx = index === undefined ? this.cssRules.length : toNum(index);
    const rule = { cssText: toStr(ruleText) };
    if (idx >= this.cssRules.length) {
      this.cssRules.push(rule);
      return this.cssRules.length - 1;
    }
    this.cssRules.push(rule);
    return idx;
  }

  deleteRule(index) {
    const idx = toNum(index);
    const next = [];
    for (let i = 0; i < this.cssRules.length; i++) {
      if (i !== idx) {
        next.push(this.cssRules[i]);
      }
    }
    this.cssRules = next;
  }

  replaceSync(text) {
    this.cssRules = [{ cssText: toStr(text || "") }];
  }

  replace(text) {
    this.replaceSync(text);
    return anvilPromiseResolve(this);
  }

  cssText() {
    let out = "";
    for (let i = 0; i < this.cssRules.length; i++) {
      const r = this.cssRules[i];
      if (r && r.cssText) {
        if (out.length > 0) out = out + "\n";
        out = out + toStr(r.cssText);
      }
    }
    return out;
  }
}

export function createBrowserEnv(initialUrl) {
  const doc = new AnvilDOMDocument(initialUrl || "https://www.google.com/");
  const customElements = new AnvilCustomElementRegistry(doc);
  doc.customElements = customElements;
  const history = new AnvilHistory(doc.location);
  const localStorage = new AnvilStorage(doc, "localStorage");
  const sessionStorage = new AnvilStorage(doc, "sessionStorage");
  const navigator = {
    userAgent: "AnvilBrowser/0.24.1 (Windows NT 10.0; Win64; x64)",
    language: "en-US",
    languages: ["en-US", "en"],
    onLine: true,
    hardwareConcurrency: 8,
    cookieEnabled: true,
    mediaCapabilities: {
      decodingInfo: () =>
        anvilPromiseResolve({ supported: true, smooth: true, powerEfficient: true }),
      encodingInfo: () =>
        anvilPromiseResolve({ supported: true, smooth: true, powerEfficient: true }),
    },
  };
  const performance = new AnvilPerformance();
  const win = {
    document: doc,
    location: doc.location,
    history: history,
    localStorage: localStorage,
    sessionStorage: sessionStorage,
    navigator: navigator,
    customElements: customElements,
    performance: performance,
    Performance: AnvilPerformance,
    DOMRect: AnvilDOMRect,
    DocumentFragment: AnvilDocumentFragment,
    DOMParser: AnvilDOMParser,
    FormData: AnvilFormData,
    Blob: AnvilBlob,
    TextEncoder: AnvilTextEncoder,
    TextDecoder: AnvilTextDecoder,
    BroadcastChannel: AnvilBroadcastChannel,
    MessageChannel: AnvilMessageChannel,
    MessagePort: AnvilMessagePort,
    CSSStyleSheet: AnvilCSSStyleSheet,
    MediaSource: AnvilMediaSource,
    SourceBuffer: AnvilSourceBuffer,
    VideoFrame: AnvilVideoFrame,
    EncodedVideoChunk: AnvilEncodedVideoChunk,
    VideoEncoder: AnvilVideoEncoder,
    VideoDecoder: AnvilVideoDecoder,
    MediaRecorder: AnvilMediaRecorder,
    AudioContext: AnvilAudioContext,
    ImageData: AnvilImageData,
    CanvasRenderingContext2D: AnvilCanvasRenderingContext2D,
    encodeURIComponent: anvilEncodeURIComponent,
    decodeURIComponent: anvilDecodeURIComponent,
    btoa: anvilBtoa,
    atob: anvilAtob,
    structuredClone: anvilStructuredClone,
    queueMicrotask: (cb) => {
      if (typeof cb === "function") cb();
    },
    crypto: {
      randomUUID: () => "00000000-0000-4000-8000-000000000001",
      getRandomValues: (arr) => arr,
    },
    requestAnimationFrame: (cb) => {
      if (typeof cb === "function") cb(16);
      return 1;
    },
    cancelAnimationFrame: () => {},
    innerWidth: 1280,
    innerHeight: 720,
    devicePixelRatio: 1,
    matchMedia: (q) => ({
      matches: true,
      media: toStr(q),
      addListener: () => {},
      removeListener: () => {},
      addEventListener: () => {},
      removeEventListener: () => {},
    }),
    getComputedStyle: (el) => (el && el.style ? el.style : { display: "block" }),
  };
  win.window = win;
  win.globalThis = win;
  win.self = win;
  return win;
}
