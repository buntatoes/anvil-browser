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
        <h1>Security &amp; Sandbox Status: Hardcoded &amp; Enforced ({props.version})</h1>
        <p className="sub">
          Fail-closed OS process isolation, Content Security Policy (CSP), Cross-Origin Resource Policy (CORP), and RFC 6265bis Cookie Jar protections are mandatory at launch.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">1. Hardcoded Mandatory OS Process Sandbox (flux-sandbox)</p>
        <p className="sec-desc">
          Fail-closed startup enforcement: Flux refuses to launch unless Win32 Job Objects (256 MiB RAM cap, ActiveProcessLimit=1, UIRestrictions=0xFF clipboard/handle isolation, KillOnJobClose) or Linux prctl(NO_NEW_PRIVS) + rlimit are active, paired with FLUXIPC1 8 MiB framed IPC and per-tab crash containment.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">2. Content Security Policy (CSP), CORP, &amp; Cookie Jar (flux-net)</p>
        <p className="sec-desc">
          Enforces HTTP and meta Content-Security-Policy (default-src, script-src, style-src, img-src, media-src, connect-src), Cross-Origin-Resource-Policy (same-origin / same-site), and RFC 6265bis cookies (HttpOnly hidden from scripts, Secure, SameSite=Strict/Lax/None, and __Host- / __Secure- prefix validation).
        </p>
      </div>

      <div className="section">
        <p className="sec-title">3. Network &amp; Transport Hardening (flux-net)</p>
        <p className="sec-desc">
          HTTPS strong crypto validation, HTTPS-to-HTTP redirect downgrade blocking, userinfo (@) phishing URL rejection, CRLF header injection sanitization, and Referrer-Policy: no-referrer.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">4. Decompression, JSON, &amp; Image Bomb Guards (flux-compress / flux-json / flux-image)</p>
        <p className="sec-desc">
          Bounded DEFLATE/zlib/gzip expansion (16 MiB HTTP ceiling), bounded JSON parsing (MAX_JSON_DEPTH = 64, 16 MiB ceiling), and 8192x8192 / 16-megapixel (64 MiB RGBA) image allocation caps.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">5. DOM &amp; Script Execution Sandbox (flux-dom / flux-script)</p>
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
