// SPDX-License-Identifier: LicenseRef-PolyForm-Noncommercial-1.0.0-OR-Commercial
// Copyright (c) 2026 Buntato
// Stored text for anvil://settings. frontend.rs
// settings_html_with_profile_state_and_engine compiles it.
// Only the markup block is kept. SETTINGS_JS is added later as a script
// element and is not part of this file.
// Author: Buntato

export interface SettingsPageProps {
  version: string;
  activeSearchEngineLabel: string;
  googleCardClass: string;
  ddgCardClass: string;
  bingCardClass: string;
  activeThemeLabel: string;
  darkThemeClass: string;
  midnightThemeClass: string;
  lightThemeClass: string;
  nordicThemeClass: string;
  activeZoomLabel: string;
  scriptsEnabledStatus: string;
  scriptsToggleLabel: string;
  scriptsToggleClass: string;
  jitEnabledStatus: string;
  jitToggleLabel: string;
  jitToggleClass: string;
  httpsOnlyStatus: string;
  httpsToggleLabel: string;
  httpsToggleClass: string;
  gpuBackendLabel: string;
  gpuVideoStatus: string;
  gpuToggleLabel: string;
  gpuToggleClass: string;
  framesDecoded: string;
  framesEncoded: string;
  profileStatus: string;
  bookmarkCount: string;
  historyCount: string;
  cookieCount: string;
}

// Markup below is extracted. Do not put page-visible notes in that block.

export function SettingsPage(props: SettingsPageProps) {
  return (
    <div className="settings-shell">
      <div className="brand-bar">
        <img id="anvil-brand-logo" src="anvil://logo.png" width="36" height="36" alt="Anvil" />
        <p className="brand-title">Anvil Settings</p>
        <p className="brand-nav">
          <a href="anvil://home">Home</a> | <a href="anvil://settings">Settings</a> | <a href="anvil://downloads">Downloads</a> | <a href="anvil://history">History</a> | <a href="anvil://security">Security</a> | <a href="anvil://inspect">Inspector</a>
        </p>
      </div>

      <div className="settings-layout">
        <div className="settings-sidebar">
          <p className="sidebar-heading">CATEGORIES</p>
          <div id="nav-sec-all" className="sidebar-item-active">
            <a href="anvil://settings">All Settings</a>
          </div>
          <div id="nav-sec-appearance" className="sidebar-item">
            <a href="anvil://settings?section=appearance">Appearance &amp; Theme</a>
          </div>
          <div id="nav-sec-search" className="sidebar-item">
            <a href="anvil://settings?section=search">Search Engine</a>
          </div>
          <div id="nav-sec-js" className="sidebar-item">
            <a href="anvil://settings?section=js">JavaScript</a>
          </div>
          <div id="nav-sec-privacy" className="sidebar-item">
            <a href="anvil://settings?section=privacy">Privacy &amp; Security</a>
          </div>
          <div id="nav-sec-system" className="sidebar-item">
            <a href="anvil://settings?section=system">System &amp; Hardware</a>
          </div>
        </div>

        <div className="settings-main">
          <div id="sec-appearance" className="settings-card" data-section="appearance">
            <p className="sec-title">Appearance, Theme &amp; Page Zoom</p>
            <p className="sec-desc">
              Active Theme: {props.activeThemeLabel} — Applies across the window chrome, tabs, menus, and built-in pages.
            </p>
            <div className="engine-grid">
              <div className={props.darkThemeClass}>
                <div className="engine-card-text">
                  <p className="engine-name">Obsidian Dark</p>
                  <p className="engine-url">Ember &amp; Slate</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?theme=dark">Use Dark</a>
              </div>

              <div className={props.midnightThemeClass}>
                <div className="engine-card-text">
                  <p className="engine-name">Midnight OLED</p>
                  <p className="engine-url">Pitch Black &amp; Cyan</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?theme=midnight">Use Midnight</a>
              </div>

              <div className={props.lightThemeClass}>
                <div className="engine-card-text">
                  <p className="engine-name">Daylight Light</p>
                  <p className="engine-url">Warm Alabaster</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?theme=light">Use Light</a>
              </div>

              <div className={props.nordicThemeClass}>
                <div className="engine-card-text">
                  <p className="engine-name">Nordic Frost</p>
                  <p className="engine-url">Cool Slate &amp; Teal</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?theme=nordic">Use Nordic</a>
              </div>
            </div>

            <div className="setting-row" style="margin-top: 12px;">
              <div className="setting-info">
                <p className="setting-label">Page Zoom Scale ({props.activeZoomLabel})</p>
                <p className="sec-desc">
                  Scales page typography, layout boxes, and vector icons between 50% and 250% for 1080p, 1440p, and 4K displays.
                </p>
              </div>
              <div className="setting-actions-inline">
                <a className="setting-action-pill" href="anvil://settings?zoom=75">75%</a>
                <a className="setting-action-pill" href="anvil://settings?zoom=90">90%</a>
                <a className="setting-action-pill" href="anvil://settings?zoom=100">100%</a>
                <a className="setting-action-pill" href="anvil://settings?zoom=125">125%</a>
                <a className="setting-action-pill" href="anvil://settings?zoom=150">150%</a>
                <a className="setting-action-pill" href="anvil://settings?zoom=175">175%</a>
                <a className="setting-action-pill" href="anvil://settings?zoom=200">200%</a>
              </div>
            </div>
          </div>

          <div className="settings-card" data-section="search">
            <p className="sec-title">Default Search Engine</p>
            <p className="sec-desc">
              Active Engine: {props.activeSearchEngineLabel} — Used in the address bar and on the home screen search box.
            </p>
            <div className="engine-grid">
              <div className={props.googleCardClass}>
                <img src="anvil://engine-google.png" width="32" height="32" alt="Google" />
                <div className="engine-card-text">
                  <p className="engine-name">Google Search</p>
                  <p className="engine-url">www.google.com</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?engine=google">Use Google</a>
              </div>

              <div className={props.ddgCardClass}>
                <img src="anvil://engine-ddg.png" width="32" height="32" alt="DuckDuckGo" />
                <div className="engine-card-text">
                  <p className="engine-name">DuckDuckGo</p>
                  <p className="engine-url">duckduckgo.com</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?engine=duckduckgo">Use DuckDuckGo</a>
              </div>

              <div className={props.bingCardClass}>
                <img src="anvil://engine-bing.png" width="32" height="32" alt="Microsoft Bing" />
                <div className="engine-card-text">
                  <p className="engine-name">Microsoft Bing</p>
                  <p className="engine-url">www.bing.com</p>
                </div>
                <a className="setting-action-pill" href="anvil://settings?engine=bing">Use Bing</a>
              </div>
            </div>
          </div>

          <div className="settings-card" data-section="js">
            <p className="sec-title">JavaScript (Cinder JS VM &amp; JIT)</p>
            <div className="setting-row">
              <div className="setting-info">
                <p className="setting-label">JavaScript Execution</p>
                <p className="sec-desc">
                  {props.scriptsEnabledStatus} — Allows websites to run interactive scripts and DOM updates inside the 200,000-step Cinder VM sandbox.
                </p>
              </div>
              <div className={props.scriptsToggleClass}>
                <a href="anvil://settings?toggle=scripts">{props.scriptsToggleLabel}</a>
              </div>
            </div>

            <div className="setting-row">
              <div className="setting-info">
                <p className="setting-label">C++ math helper (Cinder JIT Accelerator)</p>
                <p className="sec-desc">
                  {props.jitEnabledStatus}
                </p>
              </div>
              <div className={props.jitToggleClass}>
                <a href="anvil://settings?toggle=jit">{props.jitToggleLabel}</a>
              </div>
            </div>
          </div>

          <div className="settings-card" data-section="privacy">
            <p className="sec-title">Privacy, HTTPS Transport &amp; Encrypted Profile</p>
            <div className="setting-row">
              <div className="setting-info">
                <p className="setting-label">Strict HTTPS-Only Transport Mode</p>
                <p className="sec-desc">
                  {props.httpsOnlyStatus} — When on, every http address is loaded over https. If a site can't do that, you get a warning page and nothing is sent over plain http, unless you allow that site.
                </p>
              </div>
              <div className={props.httpsToggleClass}>
                <a href="anvil://settings?toggle=https_only">{props.httpsToggleLabel}</a>
              </div>
            </div>

            <div className="setting-row">
              <div className="setting-info">
                <p className="setting-label">Site Permissions</p>
                <p className="sec-desc">
                  JavaScript, cookies, and plain HTTP, set per site. Cookies are also kept apart by the site in the address bar, so a site embedded in two others can't link your visits.
                </p>
              </div>
              <div className="setting-actions-inline">
                <a className="setting-action-pill" href="anvil://settings/permissions">Open</a>
              </div>
            </div>

            <div className="setting-row">
              <div className="setting-info">
                <p className="setting-label">Encrypted Local Profile (ANVP Container)</p>
                <p className="sec-desc">
                  {props.profileStatus} | Bookmarks: {props.bookmarkCount} | History: {props.historyCount} | Cookies: {props.cookieCount}
                </p>
              </div>
              <div className="setting-actions-inline">
                <a className="setting-action-pill" href="anvil://settings?action=save_profile">Save Snapshot</a>
                <a className="setting-action-pill" href="anvil://settings?action=clear_profile">Clear Profile</a>
              </div>
            </div>
          </div>

          <div className="settings-card" data-section="system">
            <p className="sec-title">Video &amp; Open Media Codecs (Forge GPU + Opus / VP8 / VP9 / AV1)</p>
            <div className="setting-row">
              <div className="setting-info">
                <p className="setting-label">Video</p>
                <p className="sec-desc">
                  Backend: {props.gpuBackendLabel} | {props.gpuVideoStatus} | Decoded: {props.framesDecoded} | Encoded: {props.framesEncoded} | Codecs: Opus (RFC 6716), VP8 (RFC 6386), VP9, AV1 OBU | C++20 Raster Kernel: Active
                </p>
              </div>
              <div className={props.gpuToggleClass}>
                <a href="anvil://settings?toggle=gpu_video">{props.gpuToggleLabel}</a>
              </div>
            </div>
            <video id="gpu-demo-video" src="anvil://demo.mp4" width="240" height="110"></video>
          </div>
        </div>
      </div>
    </div>
  );
}
