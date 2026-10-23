// Flux Browser Full React/TSX Shell Component
// Author: Buntos

export interface TabItemProps {
  index: number;
  title: string;
  activeClass: string;
  closeVisible: boolean;
}

export interface BookmarkItemProps {
  index: number;
  title: string;
  url: string;
}

export interface BrowserShellProps {
  version: string;
  activeUrl: string;
  addressDisplay: string;
  addressClass: string;
  securityLabel: string;
  securityClass: string;
  zoomDisplay: string;
  starClass: string;
  inspectorClass: string;
  appMenuClass: string;
  statusLeft: string;
  statusRight: string;
  canGoBack: boolean;
  canGoForward: boolean;
}

export function FluxBrowserApp(props: BrowserShellProps) {
  return (
    <div id="flux-react-shell" className="flux-shell">
      <div id="flux-tab-strip" className="tab-strip">
        <img
          id="flux-shell-logo"
          src="flux://logo.png"
          width="24"
          height="24"
          alt="Flux"
          data-ui-action="home"
        />
        {{TABS_SLOT}}
        <button id="flux-new-tab" className="new-tab-btn" data-ui-action="new-tab">
          +
        </button>
      </div>

      <div id="flux-nav-bar" className="nav-bar">
        <button id="flux-btn-back" className="nav-btn" data-ui-action="back">
          &lt;
        </button>
        <button id="flux-btn-forward" className="nav-btn" data-ui-action="forward">
          &gt;
        </button>
        <button id="flux-btn-reload" className="nav-btn" data-ui-action="reload">
          R
        </button>
        <button id="flux-btn-home" className="nav-btn" data-ui-action="home">
          H
        </button>

        <div
          id="flux-security-badge"
          className={props.securityClass}
          data-ui-action="security-badge"
        >
          {props.securityLabel}
        </div>

        <div
          id="flux-address-bar"
          className={props.addressClass}
          data-ui-action="address-field"
        >
          {props.addressDisplay}
        </div>

        <button
          id="flux-btn-bookmark"
          className={props.starClass}
          data-ui-action="toggle-bookmark"
        >
          *
        </button>

        <button
          id="flux-zoom-pill"
          className="zoom-pill"
          data-ui-action="zoom-pill"
        >
          {props.zoomDisplay}
        </button>

        <button
          id="flux-btn-settings"
          className="settings-btn"
          data-ui-action="settings-button"
        >
          S
        </button>

        <button
          id="flux-btn-downloads"
          className="nav-btn"
          data-ui-action="downloads-button"
        >
          DL
        </button>

        <button
          id="flux-btn-inspector"
          className={props.inspectorClass}
          data-ui-action="inspector-button"
        >
          DEV
        </button>

        <button
          id="flux-btn-app-menu"
          className={props.appMenuClass}
          data-ui-action="app-menu-button"
        >
          =
        </button>
      </div>

      {{BOOKMARKS_SLOT}}

      {{FIND_BAR_SLOT}}

      {{APP_MENU_SLOT}}

      {{CONTEXT_MENU_SLOT}}

      <div id="flux-viewport-slot" className="viewport-slot">
        {{VIEWPORT_SLOT}}
      </div>

      {{INSPECTOR_DOCK_SLOT}}

      <div id="flux-status-bar" className="status-bar">
        <span id="flux-status-left" className="status-left">
          {props.statusLeft}
        </span>
        <span id="flux-status-right" className="status-right">
          {props.statusRight}
        </span>
      </div>
    </div>
  );
}
