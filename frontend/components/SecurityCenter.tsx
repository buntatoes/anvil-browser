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
        <h1>Security &amp; Sandbox Status: Active ({props.version})</h1>
        <p className="sub">
          Process isolation, Pre-Socket Tracker/Ad/Malware Shield, Content Security Policy (CSP), Cross-Origin Resource Policy (CORP), and Cookie Jar protections are active.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">1. Process &amp; Origin Isolation</p>
        <p className="sec-desc">
          Isolates page rendering, script execution, and network workloads across independent contexts with strict origin boundaries.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">2. Pre-Socket Tracker, Ad-Tech, &amp; Malicious Domain Shield</p>
        <p className="sec-desc">
          Blocks ad networks, behavioral telemetry/beacon endpoints, browser fingerprinting scripts, and known phishing/cryptomining/malware hosts before any connection is opened.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">3. Content Security Policy (CSP), CORP, &amp; Cookie Protection</p>
        <p className="sec-desc">
          Enforces HTTP and meta Content-Security-Policy directives, Cross-Origin-Resource-Policy headers, and strict cookie scoping (HttpOnly, Secure, SameSite).
        </p>
      </div>

      <div className="section">
        <p className="sec-title">4. Network &amp; Transport Hardening</p>
        <p className="sec-desc">
          HTTPS strong crypto validation, HTTPS-to-HTTP redirect downgrade blocking, phishing URL rejection, header sanitization, and Referrer-Policy: no-referrer.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">5. Media &amp; Payload Verification</p>
        <p className="sec-desc">
          Validates compressed HTTP streams, JSON payloads, and raster/video media surfaces prior to decoding and presentation.
        </p>
      </div>

      <p>
        <a href="flux://home">Return to Flux Start</a>
      </p>
    </div>
  );
}
