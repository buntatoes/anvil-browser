// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Stored text for anvil://downloads. frontend.rs downloads_html_with_entries
// compiles it. Only the markup block is kept. There is no script file.
// Props filled by the compiler: version, entryCount.
// The DOWNLOAD_ITEMS_SLOT marker in the markup is replaced with one block
// per download, or the empty-state sentence.
// Lines above the markup block are not copied into the page.
// Author: Buntato

export interface DownloadsPageProps {
  version: string; // Shown in the badge line.
  entryCount: string; // File count, already formatted.
}

// Markup below is extracted. Do not put page-visible notes in that block.

export function DownloadsPage(props: DownloadsPageProps) {
  return (
    <div className="settings-shell">
      <div className="brand-bar">
        <img id="anvil-brand-logo" src="anvil://logo.png" width="38" height="38" alt="Anvil" />
        <p className="brand-title">Anvil Downloads</p>
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
            <a href="anvil://downloads">Downloads Vault</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://history">Browsing History</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://security">Security &amp; Shields</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://settings">Browser Settings</a>
          </div>
          <div className="sidebar-item">
            <a href="anvil://inspect">Developer Inspector</a>
          </div>
        </div>

        <div className="settings-main">
          <div className="hero">
            <p className="badge">ANVIL BROWSER {props.version} — SHA-256 VERIFIER &amp; SAFE QUARANTINE</p>
            <h1>Download Manager ({props.entryCount} Files)</h1>
            <p className="subtitle">
              Every downloaded payload is verified with a FIPS 180-4 SHA-256 checksum, sanitized against path traversal, tagged with Mark-of-the-Web (ZoneId=3), and checked for executable quarantine.
            </p>
          </div>

          <div className="section">
            <p className="sec-title">Quick Verification &amp; Test Downloads</p>
            <p className="sec-desc">
              Generate and verify an in-browser diagnostic bundle or CENC-encrypted MP4 stream with live SHA-256 integrity hashing:
            </p>
            <p>
              <a href="anvil://downloads?action=sample_report">Download Verified Diagnostic Report (JSON)</a> | <a href="anvil://downloads?action=sample_mp4">Download CENC Encrypted Stream (MP4)</a> | <a href="anvil://downloads?action=export_manifest">Export Vault Manifest to Disk</a> | <a href="anvil://downloads?action=clear">Clear Download Log</a>
            </p>
          </div>

          <div id="anvil-downloads-list" className="history-list">
            {{DOWNLOAD_ITEMS_SLOT}}
          </div>

          <p>
            <a href="anvil://home">Return to Anvil Start</a> | <a href="anvil://settings">Open Settings &amp; EME Video</a> | <a href="anvil://security">Open Security Center</a>
          </p>
        </div>
      </div>
    </div>
  );
}
