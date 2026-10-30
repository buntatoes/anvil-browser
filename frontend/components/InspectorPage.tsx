// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Stored text for anvil://inspect. frontend.rs inspect_html_with_state
// compiles it. Only the markup block is kept. There is no script file.
// Props filled by the compiler: version, tabCount, activeZoom, gpuBackend,
// shieldBlocked, hstsCount, downloadCount, cookieCount.
// Lines above the markup block are not copied into the page.
// Author: Buntato

export interface InspectorPageProps {
  version: string; // Shown in the badge line.
  tabCount: string; // Open tab count, already formatted.
  activeZoom: string; // Zoom percent, already formatted with a percent sign.
  gpuBackend: string; // Video backend label.
  shieldBlocked: string; // Shield block count, already formatted.
  hstsCount: string; // HSTS host count, already formatted.
  downloadCount: string; // Download count, already formatted.
  cookieCount: string; // Cookie count, already formatted.
}

// Markup below is extracted. Do not put page-visible notes in that block.

export function InspectorPage(props: InspectorPageProps) {
  return (
    <div className="settings-shell">
      <div className="brand-bar">
        <img id="anvil-brand-logo" src="anvil://logo.png" width="38" height="38" alt="Anvil" />
        <p className="brand-title">Anvil Developer Inspector</p>
        <p className="brand-nav">
          <a href="anvil://home">Start</a> | <a href="anvil://inspect">Inspector</a> | <a href="anvil://settings">Settings</a> | <a href="anvil://downloads">Downloads</a> | <a href="anvil://history">History</a> | <a href="anvil://security">Security</a>
        </p>
      </div>

      <div className="settings-layout">
        <div className="settings-sidebar">
          <p className="sidebar-heading">NAVIGATION</p>
          <div className="sidebar-item">
            <a href="anvil://home">New Tab Start</a>
          </div>
          <div className="sidebar-item-active">
            <a href="anvil://inspect">Developer Inspector</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://security">Security &amp; Shields</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://settings">Browser Settings</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://downloads">Downloads Vault</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://history">Browsing History</a>
          </div>
        </div>

        <div className="settings-main">
          <div className="hero">
            <p className="badge">
              PROOF OF CONCEPT — DEVTOOLS &amp; ENGINE DIAGNOSTICS ({props.version}) — MOUSE DOCK (F12) &amp; FULL VIEW
            </p>
            <h1>Developer Inspector &amp; Engine Telemetry</h1>
            <p className="sub">
              Inspect live DOM arena budgets, CSS cascade specificity, DisplayList SDF paint commands, HSTS/SRI/CSP transport policies, and W3C EME/NV12 media pipelines. Click the [&lt;&gt;] Inspector button in the top toolbar or press F12 on any page to toggle the live bottom Inspector Dock.
            </p>
            <p>
              <a href="examples/hello.html">Inspect Local Demo DOM (examples/hello.html)</a> | <a href="anvil://settings">Inspect GPU Video &amp; EME Stream</a> | <a href="anvil://downloads?action=sample">Run SHA-256 Download Probe</a>
            </p>
          </div>

          <div className="cards">
            <div className="card">
              <p className="card-title">1. Forge &amp; Prism Engine</p>
              <p className="card-body">
                Forge DOM/layout/paint engine (MAX_DOM_NODES: 50,000, MAX_DOM_DEPTH: 256) and Prism CSS engine with Flexbox, Grid, and W3C stacking order.
              </p>
              <p className="badge">Active Tabs: {props.tabCount} | Viewport Zoom: {props.activeZoom}</p>
            </div>

            <div className="card">
              <p className="card-title">2. Cinder JS, HSTS &amp; SRI</p>
              <p className="card-body">
                Cinder JS bytecode VM &amp; JIT, pre-socket tracker shield, RFC 6797 HSTS preload/dynamic store, and FIPS 180-4 SHA-256 Subresource Integrity.
              </p>
              <p className="badge">Blocked: {props.shieldBlocked} | HSTS Hosts: {props.hstsCount}</p>
            </div>

            <div className="card">
              <p className="card-title">3. GPU, Opus/AV1/VP9 &amp; Storage</p>
              <p className="card-body">
                Direct3D 11 / OpenGL 3.3 SDF compositor, royalty-free Opus (RFC 6716), VP8 (RFC 6386), VP9 &amp; AV1 media codecs, and DPAPI profile vault.
              </p>
              <p className="badge">GPU: {props.gpuBackend} | Downloads: {props.downloadCount}</p>
            </div>
          </div>

          <div className="section">
            <p className="sec-title">Engine Subsystem &amp; Security Budget Matrix (Forge · Cinder · Prism)</p>
            <table id="anvil-inspector-table">
              <tr>
                <th>Subsystem</th>
                <th>Active Implementation</th>
                <th>Hardened Ceiling / Invariant</th>
                <th>Status</th>
              </tr>
              <tr>
                <td>Forge DOM Arena (anvil-dom)</td>
                <td>NodeId Arena + Implicit Table Auto-Close</td>
                <td>50,000 Nodes / 256 Depth / 128 Attrs</td>
                <td>ACTIVE</td>
              </tr>
              <tr>
                <td>Prism CSS &amp; Forge Layout (anvil-style / anvil-layout)</td>
                <td>Specificity Cascade + @media + Flexbox + Grid + Table</td>
                <td>Multi-Font Metrics + W3C Stacking Order</td>
                <td>ACTIVE</td>
              </tr>
              <tr>
                <td>Forge Raster &amp; GPU (anvil-raster / anvil-gpu)</td>
                <td>D3D11 / GL3.3 SDF Rounded Quads, Gradients, Shadows &amp; NV12</td>
                <td>16 MP (64 MiB) Image Allocation Cap</td>
                <td>ACTIVE</td>
              </tr>
              <tr>
                <td>Cinder JS VM &amp; JIT (anvil-script / engines/cinder)</td>
                <td>Register Bytecode VM + C++20 JIT + WebAssembly 1.0 VM</td>
                <td>200,000 Steps / 64 Call Depth / SOP Fetch</td>
                <td>ACTIVE</td>
              </tr>
              <tr>
                <td>Open Media Codecs (anvil-video)</td>
                <td>Royalty-Free Opus (RFC 6716), VP8 (RFC 6386), VP9 &amp; AV1 OBU</td>
                <td>ISO/IEC 23001-7 CENC + W3C EME ClearKey</td>
                <td>ACTIVE</td>
              </tr>
              <tr>
                <td>Transport &amp; Shield (anvil-net)</td>
                <td>Schannel / OpenSSL TLS + HSTS + SRI SHA-256</td>
                <td>8 MiB Raw / 16 MiB Decompressed Cap</td>
                <td>ACTIVE</td>
              </tr>
              <tr>
                <td>Profile &amp; Vault (profile.rs / downloads.rs)</td>
                <td>Windows DPAPI (crypt32.dll) + SHA-256 MOTW</td>
                <td>Cookies: {props.cookieCount} | Downloads: {props.downloadCount}</td>
                <td>ACTIVE</td>
              </tr>
            </table>
          </div>
        </div>
      </div>
    </div>
  );
}
