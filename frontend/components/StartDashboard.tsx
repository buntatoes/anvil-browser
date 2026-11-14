// SPDX-License-Identifier: Apache-2.0
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
      <div className="home-top-links">
        <a href="anvil://bookmarks">Bookmarks</a>
        <a href="anvil://history">History</a>
        <a href="anvil://downloads">Downloads</a>
        <a href="anvil://security">Shields</a>
        <a href="anvil://settings">Settings</a>
      </div>

      <div className="home-brand-row">
        <img id="anvil-brand-logo" src="anvil://logo.png" width="72" height="72" alt="Anvil" />
        <h1 className="home-title">Anvil</h1>
      </div>

      <div className="home-search-wrap">
        <form className="search-row" action="anvil://navigate" method="GET">
          <img
            id="home-engine-badge"
            src={props.activeEngineLogo}
            width="24"
            height="24"
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

      <div id="home-speed-dial" className="speed-dial-grid">
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="https://en.wikipedia.org/">Wikipedia</a></p>
          <p className="speed-dial-sub">wikipedia.org</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="https://github.com/">GitHub</a></p>
          <p className="speed-dial-sub">github.com</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="https://doc.rust-lang.org/std/">Rust Docs</a></p>
          <p className="speed-dial-sub">doc.rust-lang.org</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="https://news.ycombinator.com/">Hacker News</a></p>
          <p className="speed-dial-sub">news.ycombinator.com</p>
        </div>
      </div>

      <div className="speed-dial-grid">
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="https://www.youtube.com/">YouTube</a></p>
          <p className="speed-dial-sub">youtube.com</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="https://www.reddit.com/">Reddit</a></p>
          <p className="speed-dial-sub">reddit.com</p>
        </div>
        <div className="speed-dial-tile">
          <p className="speed-dial-label"><a href="anvil://security">Security &amp; Shields</a></p>
          <p className="speed-dial-sub">anvil://security</p>
        </div>
        <div className="speed-dial-tile">
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
