# Security

Flux 0.16 is a from-scratch browser with a short list of limits. It is not a hardened browser sandbox.

Report problems through GitHub private vulnerability reports:

https://github.com/buntatoes/flux-browser/security/advisories/new

Please do not file those details in a public issue.

## What is on

- Linux: `PR_SET_NO_NEW_PRIVS`, core dumps off, and an attempt to cap address space at 2 GiB. `--tab-worker` uses 256 MiB. Normal browsing does not spawn that worker.
- Windows: a job object on the window process with a memory cap. The worker job, if used, also limits process count and UI rights.
- A panic in `execute_isolated`, or in a background page load, is caught.
- HTTPS on Linux goes through OpenSSL with certificate checks. Windows uses Schannel.
- Linux profiles are AES-256-GCM. The key is `profile.key` (mode 0600) beside the profile. Windows profiles use DPAPI.
- Sample encryption in `flux-video` is AES-128-CTR.
- Parsers in this tree and the sibling libraries reject oversized input.
- `flux-net` blocks a fixed list of tracker host names before it connects.
- SHA-256 is used for subresource checks and download records.

## What is not on

- One OS process per tab.
- A GPU video decoder. The VA-API and Direct3D names in `flux-video` are unused.
- A JavaScript JIT. The C++ file next to `flux-script` does arithmetic.
- A 5 second worker timeout. The constant exists. The window does not enforce it.

Only 0.16 is current.
