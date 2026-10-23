// Flux Browser Settings, GPU Video & Encrypted Profile Dashboard Component (flux://settings)
// Author: Buntos

export interface SettingsPageProps {
  version: string;
  gpuBackendLabel: string;
  gpuVideoStatus: string;
  httpsOnlyStatus: string;
  scriptsEnabledStatus: string;
  framesDecoded: string;
  framesEncoded: string;
  profileStatus: string;
  bookmarkCount: string;
  historyCount: string;
  cookieCount: string;
}

export function SettingsPage(props: SettingsPageProps) {
  return (
    <div className="wrap">
      <div className="brand-bar">
        <img id="flux-brand-logo" src="flux://logo.png" width="38" height="38" alt="Flux" />
        <p className="brand-title">Flux Settings</p>
        <p className="brand-nav">
          <a href="flux://home">Start</a> | <a href="flux://settings">Settings</a> | <a href="flux://downloads">Downloads</a> | <a href="flux://history">History</a> | <a href="flux://security">Security</a>
        </p>
      </div>
      <div className="hero">
        <p className="badge">FLUX BROWSER {props.version} — ENGINE, EME/CENC MEDIA &amp; ENCRYPTED PROFILE</p>
        <h1>Settings, GPU Video, EME/CDM &amp; Local Profile</h1>
        <p className="subtitle">
          Configure hardware NV12/BT.709 video encode/decode, W3C Encrypted Media Extensions (EME / CENC), transport security policies, and Windows DPAPI local profile storage.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">1. GPU Hardware Video &amp; W3C EME / CENC Decryption (flux-video v.03)</p>
        <p className="sec-desc">
          Active Backend: {props.gpuBackendLabel} | Status: {props.gpuVideoStatus} | Frames Decoded: {props.framesDecoded} | Frames Encoded: {props.framesEncoded}
        </p>
        <p className="sec-desc">
          W3C EME / CDM Host: org.w3.clearkey (ISO/IEC 23001-7 CENC + OS Media Foundation DRM Bridge) — Interactive NV12 BT.709 Video Player:
        </p>
        <video id="gpu-demo-video" src="flux://demo.mp4" width="240" height="120"></video>
        <table>
          <tr>
            <th>Platform</th>
            <th>Window Shell</th>
            <th>TLS Stack</th>
            <th>GPU Video</th>
          </tr>
          <tr>
            <td>Windows x64</td>
            <td>Win32 + DWM Dark</td>
            <td>Schannel</td>
            <td>D3D11 DXVA2 NV12</td>
          </tr>
          <tr>
            <td>Linux x64</td>
            <td>X11 (libX11)</td>
            <td>OpenSSL 1.2/1.3</td>
            <td>VA-API NV12</td>
          </tr>
        </table>
        <p>
          <a href="flux://settings?toggle=gpu_video">Toggle GPU Hardware Video Acceleration</a>
        </p>
      </div>

      <div className="section">
        <p className="sec-title">2. Strict HTTPS-Only Transport Mode (flux-net)</p>
        <p className="sec-desc">
          Current Policy: {props.httpsOnlyStatus} — Automatically upgrades or blocks cleartext HTTP requests while enforcing TLS strong crypto and downgrade protection.
        </p>
        <p>
          <a href="flux://settings?toggle=https_only">Toggle Strict HTTPS-Only Mode</a>
        </p>
      </div>

      <div className="section">
        <p className="sec-title">3. Script Execution Sandbox (flux-script)</p>
        <p className="sec-desc">
          Current Policy: {props.scriptsEnabledStatus} — Executes page scripts within the 100,000-step instruction budget, 64-frame recursion cap, and Same-Origin Policy.
        </p>
        <p>
          <a href="flux://settings?toggle=scripts">Toggle Script Execution</a>
        </p>
      </div>

      <div className="section">
        <p className="sec-title">4. Encrypted Local Profile &amp; Session Persistence (FLXP Container)</p>
        <p className="sec-desc">
          Container Status: {props.profileStatus} | Saved Bookmarks: {props.bookmarkCount} | History Log: {props.historyCount} | Cookies: {props.cookieCount}
        </p>
        <p className="sec-desc">
          Persists bookmarks, browsing history, cookies, zoom level, and engine settings in an authenticated, zlib-compressed 256-bit stream-encrypted local container (profile.flx).
        </p>
        <p>
          <a href="flux://settings?action=save_profile">Save Encrypted Profile Snapshot Now</a> | <a href="flux://settings?action=clear_profile">Clear All Local Profile Data</a>
        </p>
      </div>

      <p>
        <a href="flux://home">Return to Flux Start</a> | <a href="flux://history">Open Browsing History</a> | <a href="flux://security">Open Security Center</a>
      </p>
    </div>
  );
}
