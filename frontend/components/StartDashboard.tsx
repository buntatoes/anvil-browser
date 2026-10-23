// Flux Browser Built-In Start Dashboard Component (flux://home)
// Author: Buntos

export interface StartDashboardProps {
  version: string;
  defaultQuery: string;
}

export function StartDashboard(props: StartDashboardProps) {
  return (
    <div className="page">
      <div className="hero">
        <p className="badge">
          FLUX BROWSER {props.version} — REACT TSX SHELL &amp; HARDENED SANDBOX ACTIVE
        </p>
        <h1>Flux Browser</h1>
        <p className="subtitle">
          Zero-telemetry native engine with a full React TSX front-end, SDF GPU compositing, TrueType anti-aliasing, and deterministic security caps.
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
          <p className="card-title">Settings &amp; GPU Video</p>
          <p className="card-body">
            Configure NV12/BT.709 GPU video encode/decode, HTTPS-Only mode, and script execution.
          </p>
          <p>
            <a href="flux://settings">Open Settings</a>
          </p>
        </div>
        <div className="card">
          <p className="card-title">Security Center</p>
          <p className="card-body">
            Inspect active TLS, Same-Origin Policy, DOM depth, and zip-bomb protections.
          </p>
          <p>
            <a href="flux://security">Open Security Center</a>
          </p>
        </div>
        <div className="card">
          <p className="card-title">Browsing History</p>
          <p className="card-body">
            Review visited pages in the local session log or clear browsing history.
          </p>
          <p>
            <a href="flux://history">Open History</a>
          </p>
        </div>
      </div>

      <p>
        <a href="https://example.com">Visit example.com (HTTPS Test)</a> | <a href="examples/hello.html">Open Local Demo (examples/hello.html)</a>
      </p>

      <div className="shield-bar">
        <p className="shield-text">
          Shields ON: TLS Strong Crypto | 32 MiB Decompression Cap | 16 MP Image Cap | 256 DOM Depth Cap | 100K Script Step Cap | No-Referrer
        </p>
      </div>
    </div>
  );
}
