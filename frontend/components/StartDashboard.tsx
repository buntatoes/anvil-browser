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
          <p className="card-title">Security Center</p>
          <p className="card-body">
            Inspect active TLS, Same-Origin Policy, DOM depth, and zip-bomb protections.
          </p>
          <p>
            <a href="flux://security">Open Security Center</a>
          </p>
        </div>
        <div className="card">
          <p className="card-title">Live HTTPS Test</p>
          <p className="card-body">
            Verify TLS strong-crypto handshake and remote origin isolation on example.com.
          </p>
          <p>
            <a href="https://example.com">Visit example.com</a>
          </p>
        </div>
        <div className="card">
          <p className="card-title">Local Demo Page</p>
          <p className="card-body">
            Open the bundled local HTML/CSS showcase inside the isolated file viewer.
          </p>
          <p>
            <a href="examples/hello.html">Open examples/hello.html</a>
          </p>
        </div>
      </div>

      <div className="shield-bar">
        <p className="shield-text">
          Shields ON: TLS Strong Crypto | 32 MiB Decompression Cap | 16 MP Image Cap | 256 DOM Depth Cap | 100K Script Step Cap | No-Referrer
        </p>
      </div>
    </div>
  );
}
