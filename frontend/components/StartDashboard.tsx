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
        <img id="anvil-brand-logo" src="anvil://logo.png" width="88" height="88" alt="Anvil" />
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

      <p className="home-apps">
        <a className="setting-action-pill" href="https://en.wikipedia.org/">Wikipedia</a>
        <a className="setting-action-pill" href="https://github.com/">GitHub</a>
        <a className="setting-action-pill" href="https://doc.rust-lang.org/std/">Rust Docs</a>
        <a className="setting-action-pill" href="https://news.ycombinator.com/">Hacker News</a>
        <a className="setting-action-pill" href="https://www.youtube.com/">YouTube</a>
        <a className="setting-action-pill" href="https://www.reddit.com/">Reddit</a>
        <a className="setting-action-pill" href="anvil://settings">Settings</a>
      </p>

      <div id="home-activity-container" className="home-activity">
        {{HOME_ACTIVITY_SLOT}}
      </div>
    </div>
  );
}
