// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Stored text for anvil://home. frontend.rs home_html_with_state compiles it.
// Only the markup block is kept. HOME_JS is added later as a script element
// and is not part of this file.
// Props filled by the compiler: version, defaultQuery, activeEngineName,
// activeEngineLogo, searchPlaceholder, googlePillClass, ddgPillClass, bingPillClass.
// Slot replaced: HOME_ACTIVITY_SLOT.
// Lines above the markup block are not copied into the page.
// Author: Buntato

export interface StartDashboardProps {
  version: string;
  defaultQuery: string;
  activeEngineName: string;
  activeEngineLogo: string;
  searchPlaceholder: string;
  googlePillClass: string;
  ddgPillClass: string;
  bingPillClass: string;
}

// Markup below is extracted. Do not put page-visible notes in that block.

export function StartDashboard(props: StartDashboardProps) {
  return (
    <div className="home-blank">
      <div className="home-brand-row">
        <img id="anvil-brand-logo" src="anvil://logo.png" width="84" height="84" alt="Anvil" />
        <h1 className="home-title">Anvil</h1>
      </div>

      <div className="home-search-wrap">
        <form className="search-row" action="anvil://navigate" method="GET">
          <img
            id="home-engine-badge"
            src={props.activeEngineLogo}
            width="28"
            height="28"
            alt={props.activeEngineName}
          />
          <input
            id="home-q"
            type="text"
            name="q"
            value={props.defaultQuery}
            placeholder={props.searchPlaceholder}
          />
          <button id="home-go" type="submit">
            Search
          </button>
        </form>
      </div>

      <div id="home-engine-switcher" className="home-engine-pills">
        <a className={props.googlePillClass} href="anvil://home?engine=google">Google Search</a>
        <a className={props.ddgPillClass} href="anvil://home?engine=duckduckgo">DuckDuckGo</a>
        <a className={props.bingPillClass} href="anvil://home?engine=bing">Microsoft Bing</a>
      </div>

      <div id="home-shields-telemetry" className="shields-telemetry-bar">
        <div className="shields-stat-card">
          <p className="shields-stat-val">Adblock Active</p>
          <p className="shields-stat-label">Pre-Socket Tracker Shield</p>
        </div>
        <div className="shields-stat-card">
          <p className="shields-stat-val">X25519MLKEM768</p>
          <p className="shields-stat-label">Post-Quantum Hybrid TLS</p>
        </div>
        <div className="shields-stat-card">
          <p className="shields-stat-val">HSTS + SRI</p>
          <p className="shields-stat-label">SHA-256 Subresource Check</p>
        </div>
        <div className="shields-stat-card">
          <p className="shields-stat-val">Isolated</p>
          <p className="shields-stat-label">Per-Tab Renderer Sandbox</p>
        </div>
      </div>

      <div id="home-speed-dial" className="speed-dial-grid">
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">ENCYCLOPEDIA</p>
          <p className="speed-dial-label"><a href="https://en.wikipedia.org/">Wikipedia</a></p>
          <p className="speed-dial-sub">en.wikipedia.org</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">CODE</p>
          <p className="speed-dial-label"><a href="https://github.com/">GitHub</a></p>
          <p className="speed-dial-sub">github.com</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">DOCS</p>
          <p className="speed-dial-label"><a href="https://doc.rust-lang.org/std/">Rust Docs</a></p>
          <p className="speed-dial-sub">doc.rust-lang.org</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">NEWS</p>
          <p className="speed-dial-label"><a href="https://news.ycombinator.com/">Hacker News</a></p>
          <p className="speed-dial-sub">news.ycombinator.com</p>
        </div>
      </div>

      <div className="speed-dial-grid">
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">MEDIA</p>
          <p className="speed-dial-label"><a href="https://www.youtube.com/">YouTube</a></p>
          <p className="speed-dial-sub">youtube.com</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">FORUM</p>
          <p className="speed-dial-label"><a href="https://www.reddit.com/">Reddit</a></p>
          <p className="speed-dial-sub">reddit.com</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">SHIELDS</p>
          <p className="speed-dial-label"><a href="anvil://security">Security Center</a></p>
          <p className="speed-dial-sub">anvil://security</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-badge">SYSTEM</p>
          <p className="speed-dial-label"><a href="anvil://settings">Settings</a></p>
          <p className="speed-dial-sub">anvil://settings</p>
        </div>
      </div>

      <div id="home-activity-container" className="home-activity">
        {{HOME_ACTIVITY_SLOT}}
      </div>
    </div>
  );
}
