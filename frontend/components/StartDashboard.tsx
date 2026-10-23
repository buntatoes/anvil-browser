// Flux Browser Built-In Start Dashboard Component (flux://home)
// Author: Buntos

export interface StartDashboardProps {
  version: string;
  defaultQuery: string;
}

export function StartDashboard(props: StartDashboardProps) {
  return (
    <div className="page">
      <div className="brand-bar">
        <img id="flux-brand-logo" src="flux://logo.png" width="38" height="38" alt="Flux" />
        <p className="brand-title">Flux Browser</p>
        <p className="brand-nav">
          <a href="flux://home">Start</a> | <a href="flux://settings">Settings</a> | <a href="flux://downloads">Downloads</a> | <a href="flux://history">History</a> | <a href="flux://security">Security</a>
        </p>
      </div>
      <div className="hero">
        <p className="badge">
          FLUX BROWSER {props.version} — REACT TSX SHELL &amp; HARDENED SANDBOX ACTIVE
        </p>
        <h1>Flux Browser</h1>
        <p className="subtitle">
          Zero-telemetry native engine with a full React TSX front-end, SDF GPU compositing, interactive NV12 video scrubbing, encrypted local profile storage, and deterministic security caps.
        </p>
        <form className="search-row" action="flux://navigate" method="GET">
          <input id="home-q" type="text" name="q" value={props.defaultQuery} />
          <button id="home-go" type="submit">
            Navigate
          </button>
        </form>
      </div>

      <div className="cards">
        <div className="card">
          <p className="card-title">Settings, Video &amp; Profile</p>
          <p className="card-body">
            Configure NV12/BT.709 GPU video encode/decode, scrub the live video player, and manage your encrypted local profile.
          </p>
          <p>
            <a href="flux://settings">Open Settings</a>
          </p>
        </div>
        <div className="card">
          <p className="card-title">Security Center</p>
          <p className="card-body">
            Inspect active TLS, Tracker/Ad/Malware Shield, Content Security Policy, and media verification status.
          </p>
          <p>
            <a href="flux://security">Open Security Center</a>
          </p>
        </div>
        <div className="card">
          <p className="card-title">Browsing History</p>
          <p className="card-body">
            Review visited pages in the local session log or clear browsing history with one click.
          </p>
          <p>
            <a href="flux://history">Open History</a>
          </p>
        </div>
      </div>

      <div className="section">
        <p className="sec-title">Quick Keyboard &amp; Interactive Controls</p>
        <table>
          <tr>
            <th>Shortcut / Control</th>
            <th>Action</th>
            <th>Shortcut / Control</th>
            <th>Action</th>
          </tr>
          <tr>
            <td>Ctrl+L / F6 (+ Tab)</td>
            <td>Focus Omnibox &amp; Autocomplete</td>
            <td>Ctrl+F</td>
            <td>Find in Page Bar</td>
          </tr>
          <tr>
            <td>Ctrl++ / Ctrl+- / Ctrl+0</td>
            <td>Page Zoom (50% - 250%)</td>
            <td>Ctrl+C / Ctrl+X / Ctrl+V</td>
            <td>System Clipboard</td>
          </tr>
        </table>
      </div>

      <p>
        <a href="https://example.com">Visit example.com (HTTPS Test)</a> | <a href="examples/hello.html">Open Local Demo (examples/hello.html)</a>
      </p>

      <div className="shield-bar">
        <p className="shield-text">
          Shields ON: TLS Strong Crypto | Pre-Socket Tracker Shield | Encrypted Local Profile | 16 MP Image Cap | 256 DOM Depth Cap | No-Referrer
        </p>
      </div>
    </div>
  );
}
