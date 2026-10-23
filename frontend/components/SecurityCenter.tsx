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
          <a href="flux://home">Start</a> | <a href="flux://settings">Settings</a> | <a href="flux://downloads">Downloads</a> | <a href="flux://history">History</a> | <a href="flux://security">Security</a>
        </p>
      </div>
      <div className="banner">
        <h1>Security &amp; Sandbox Status: Active ({props.version})</h1>
        <p className="sub">
          Mandatory OS Sandbox Isolation, Windows DPAPI Profile Protection, Strict-Transport-Security (HSTS), Subresource Integrity (SRI SHA-256), CSP/CORP, and Pre-Socket Tracker Shield are active.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">1. Mandatory OS Sandbox &amp; Tab Fault Containment</p>
        <p className="sec-desc">
          Isolates page rendering, script execution, and navigation inside a mandatory OS Job Object / rlimit boundary. Any tab fault or panic is contained to a per-tab recovery card.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">2. Windows DPAPI At-Rest Profile Protection</p>
        <p className="sec-desc">
          Seals bookmarks, history, cookies, and HSTS state via OS-managed CryptProtectData / CryptUnprotectData (crypt32.dll) tied to the active user credential.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">3. Strict-Transport-Security (HSTS) &amp; Subresource Integrity (SRI SHA-256)</p>
        <p className="sec-desc">
          Automatically upgrades HSTS hosts from HTTP to HTTPS and verifies FIPS 180-4 SHA-256 integrity hashes on external scripts and stylesheets before execution.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">4. Pre-Socket Tracker, Ad-Tech, &amp; Malicious Domain Shield</p>
        <p className="sec-desc">
          Blocks ad networks, behavioral telemetry/beacon endpoints, browser fingerprinting scripts, and known phishing/cryptomining/malware hosts before any connection is opened.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">5. Content Security Policy (CSP), CORP, &amp; Cookie Protection</p>
        <p className="sec-desc">
          Enforces HTTP and meta Content-Security-Policy directives, Cross-Origin-Resource-Policy headers, and strict cookie scoping (HttpOnly, Secure, SameSite).
        </p>
      </div>

      <p>
        <a href="flux://home">Return to Flux Start</a>
      </p>
    </div>
  );
}
