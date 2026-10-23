// Flux Browser Security Center Component (flux://security)
// Author: Buntos

export interface SecurityCenterProps {
  version: string;
}

export function SecurityCenter(props: SecurityCenterProps) {
  return (
    <div className="wrap">
      <div className="brand-bar">
        <img id="flux-brand-logo" src="flux://logo.png" width="38" height="38" alt="Flux" />
        <p className="brand-title">Flux Security Center</p>
        <p className="brand-nav">
          <a href="flux://home">Start</a> | <a href="flux://settings">Settings</a> | <a href="flux://history">History</a> | <a href="flux://security">Security</a>
        </p>
      </div>
      <div className="banner">
        <h1>Security &amp; Sandbox Status: Enforced ({props.version})</h1>
        <p className="sub">
          All network, decompression, image, DOM, and script execution boundaries are active across the React TSX front-end and native engine.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">1. Network &amp; Transport Hardening (flux-net)</p>
        <p className="sec-desc">
          HTTPS strong crypto validation, HTTPS-to-HTTP redirect downgrade blocking, userinfo (@) phishing URL rejection, CRLF header injection sanitization, and Referrer-Policy: no-referrer.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">2. Decompression &amp; Image Bomb Guards (flux-compress / flux-image)</p>
        <p className="sec-desc">
          Bounded DEFLATE/zlib/gzip stream expansion (16 MiB HTTP / 32 MiB stream ceiling) and strict 8192x8192 / 16-megapixel (64 MiB RGBA) image allocation caps across PNG, JPEG, GIF, BMP, and PPM.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">3. DOM &amp; Script Execution Sandbox (flux-dom / flux-script)</p>
        <p className="sec-desc">
          256-level DOM nesting depth limit, 50,000-node document cap, 100,000-step script instruction budget, 64-frame call recursion cap, and Same-Origin Policy (SOP) on fetch().
        </p>
      </div>

      <p>
        <a href="flux://home">Return to Flux Start</a>
      </p>
    </div>
  );
}
