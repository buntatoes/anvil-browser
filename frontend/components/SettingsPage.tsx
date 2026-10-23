// Flux Browser Settings & GPU Video Acceleration Dashboard Component (flux://settings)
// Author: Buntos

export interface SettingsPageProps {
  version: string;
  gpuBackendLabel: string;
  gpuVideoStatus: string;
  httpsOnlyStatus: string;
  scriptsEnabledStatus: string;
  framesDecoded: string;
  framesEncoded: string;
}

export function SettingsPage(props: SettingsPageProps) {
  return (
    <div className="wrap">
      <div className="hero">
        <p className="badge">FLUX BROWSER {props.version} — ENGINE &amp; GPU MEDIA SETTINGS</p>
        <h1>Settings &amp; GPU Video Pipeline</h1>
        <p className="subtitle">
          Configure hardware NV12/BT.709 video encode/decode acceleration, transport security policies, and script sandbox controls.
        </p>
      </div>

      <div className="section">
        <p className="sec-title">1. GPU Hardware Video Encode &amp; Decode (flux-video + flux-gpu)</p>
        <p className="sec-desc">
          Active Backend: {props.gpuBackendLabel} | Status: {props.gpuVideoStatus} | Frames Decoded: {props.framesDecoded} | Frames Encoded: {props.framesEncoded}
        </p>
        <p className="sec-desc">
          Live NV12 BT.709 Hardware Video Surface Stream Preview (flux://demo.mp4):
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

      <p>
        <a href="flux://home">Return to Flux Start</a> | <a href="flux://history">Open Browsing History</a> | <a href="flux://security">Open Security Center</a>
      </p>
    </div>
  );
}
