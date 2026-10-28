// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Stored text for anvil://history. frontend.rs history_html_with_entries
// compiles it. Only the markup block is kept.
// Props filled by the compiler: version, entryCount.
// The HISTORY_ITEMS_SLOT marker in the markup is replaced with one block
// per history entry, or the empty-state sentence.
// Author: Buntato

export interface HistoryPageProps {
  version: string;
  entryCount: string;
}

// Markup below is extracted. Do not put page-visible notes in that block.

export function HistoryPage(props: HistoryPageProps) {
  return (
    <div className="wrap">
      <div className="brand-bar">
        <img id="anvil-brand-logo" src="anvil://logo.png" width="38" height="38" alt="Anvil" />
        <p className="brand-title">Anvil History</p>
        <p className="brand-nav">
          <a href="anvil://home">Start</a> | <a href="anvil://inspect">Inspector</a> | <a href="anvil://settings">Settings</a> | <a href="anvil://downloads">Downloads</a> | <a href="anvil://history">History</a> | <a href="anvil://security">Security</a>
        </p>
      </div>
      <div className="hero">
        <p className="badge">ANVIL BROWSER {props.version} — SESSION HISTORY</p>
        <h1>Browsing History ({props.entryCount} Entries)</h1>
        <p className="subtitle">
          Local zero-telemetry navigation log across active and closed tabs.
        </p>
        <div className="history-filter-bar">
          <input
            id="history-filter"
            type="text"
            value=""
            placeholder="Filter browsing history by title or URL..."
          />
        </div>
      </div>

      <div id="anvil-history-list" className="history-list">
        {{HISTORY_ITEMS_SLOT}}
      </div>

      <p>
        <a className="setting-action-pill" href="anvil://history?action=clear">Clear Browsing History</a> | <a className="setting-action-pill" href="anvil://settings">Open Settings</a> | <a className="setting-action-pill" href="anvil://home">Return to Anvil Start</a>
      </p>
    </div>
  );
}
