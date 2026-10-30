// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato

import {
  anvilJsonParse,
  anvilJsonStringify,
  anvilBtoa,
  anvilAtob,
  anvilEncodeURIComponent,
  anvilDecodeURIComponent,
} from "../../src/runtime/json_codec.js";
import {
  AnvilHeaders,
  AnvilResponse,
  AnvilAbortController,
  AnvilMutationObserver,
  AnvilMediaSource,
  createBrowserEnv,
} from "../../src/runtime/web_apis.js";

// 1. JSON parse & stringify
const parsed = anvilJsonParse('{"query":"anvil browser","results":[10,20,true,null]}');
console.log(parsed.query, parsed.results.length, parsed.results[1]);
console.log(anvilJsonStringify(parsed));

// 2. Base64 and URI component codecs
const b64 = anvilBtoa("hello=world&v=1");
console.log(b64, anvilAtob(b64));
const enc = anvilEncodeURIComponent("a b+c?d=e");
console.log(enc, anvilDecodeURIComponent(enc));

// 3. Headers, Response, AbortController
const hdrs = new AnvilHeaders([["Content-Type", "application/json"]]);
hdrs.append("X-Mode", "fast");
const resp = new AnvilResponse('{"status":"ok","items":[1,2,3]}', {
  status: 200,
  headers: hdrs,
});
console.log(resp.ok, resp.headers.get("content-type"), resp.headers.get("x-mode"));
resp.json().then((data) => {
  console.log("json:", data.status, data.items.length);
});

const ac = new AnvilAbortController();
ac.signal.addEventListener("abort", () => {
  console.log("aborted:", ac.signal.aborted);
});
ac.abort();

// 4. Browser environment: Google Search, YouTube Polymer Shadow DOM + MSE, Gmail DOM + Storage
const win = createBrowserEnv("https://www.youtube.com/watch?v=abc123#t=10");
console.log(win.location.hostname, win.location.pathname, win.location.search, win.location.hash);

win.history.pushState({ videoId: "xyz789" }, "Next Video", "/watch?v=xyz789");
console.log(win.history.length, win.location.search, win.history.state.videoId);

win.localStorage.setItem("yt-player-volume", "85");
console.log("vol:", win.localStorage.getItem("yt-player-volume"), win.localStorage.length);

const doc = win.document;
let mutations = 0;
const mo = new AnvilMutationObserver((records) => {
  mutations = mutations + records.length;
});
mo.observe(doc.body, { childList: true });

const app = doc.createElement("ytd-app");
app.id = "yt-main";
app.classList.add("dark-theme");
app.setAttribute("data-page-subtype", "watch");
const shadow = app.attachShadow({ mode: "open" });
const player = doc.createElement("div");
player.setTextContent("Playing xyz789");
shadow.appendChild(player);
doc.body.appendChild(app);

console.log(
  doc.getElementById("yt-main").tagName,
  app.classList.contains("dark-theme"),
  app.dataset.pageSubtype,
  app.shadowRoot.firstChild.getTextContent(),
  mutations
);

const ms = new AnvilMediaSource();
const sb = ms.addSourceBuffer("video/mp4; codecs=\"avc1.640028\"");
sb.appendBuffer(new Uint8Array(16));
ms.endOfStream();
console.log(AnvilMediaSource.isTypeSupported("video/mp4"), sb.buffers.length, ms.readyState);
