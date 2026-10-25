# Security

Version 0.22. These are the limits that are actually in the code. This is not a multi-process browser sandbox.

Report problems in private:

https://github.com/buntatoes/anvil-browser/security/advisories/new

Do not put the details in a public issue.

## What is on

- Linux: `PR_SET_NO_NEW_PRIVS`, core dumps off, and an attempt to cap address space at 2 GiB. `--tab-worker` asks for 256 MiB and exits if that cap does not stick.
- An http or https navigation starts one `--tab-worker` and waits 5 seconds. The window process sends the session cookie jar on that GET or POST and stores `Set-Cookie`. The worker does not receive the jar. The window paints the returned document only when the frame says it succeeded. Otherwise it shows an error page and does not paint that document. `anvil://` pages and files stay in the window process. This is not a multi-process sandbox.
- Windows: a job object on the window process with a memory cap. The worker job, when used, also limits process count and UI rights.
- A panic in `execute_isolated`, or in a background page load, is caught.
- HTTPS on Linux goes through OpenSSL, with certificate checks. Windows uses Schannel.
- Linux profiles (`profile.anvil`, including bookmarks, history, cookies, HSTS, settings, and origin-partitioned `localStorage`) are AES-256-GCM from `anvil-cipher`. The key is `profile.key` (mode 0600) in the same directory, written before the profile. Windows profiles use DPAPI. `sessionStorage` is not in the profile. Each origin can store 64 KiB of `localStorage` or `sessionStorage` key and value bytes. A write past that cap leaves the previous value in place.
- Encrypted video samples are AES-128-CTR from `anvil-cipher`. CTR has no tag.
- Parsers here and in the sibling libraries reject oversized input.
- `anvil-net` blocks a fixed list of tracker host names before it connects.
- SHA-256 is used for subresource checks and download records.
- PNG chunk checksums are checked in `anvil-image`.
- Stylesheet `width`, `height`, `max-width`, and offsets resolve `px`, `%`, `em`, `rem`, and `calc()` combinations of those units. Custom properties (`--*` and `var()`) stop after 8 nested substitutions. Any other unit becomes `auto`. A percentage height is `auto` when the containing block height is not set. `flex-grow` only distributes positive free space.
- Script `fetch()` is same-origin only. The Rust interpreter performs it. A cross-origin URL returns status 403 and does not connect. The numeric compiler does not perform `fetch()`. The interpreter keeps at most 64 KiB of a response body. `anvil-net` still stops at its raw and decompressed caps.
- YouTube, Gmail, Facebook, Instagram, X, Outlook, TikTok, LinkedIn, Google Maps, Reddit, other social hosts, and bank and payment hosts load through the normal HTTP path. External scripts on those hosts are not fetched. A script runs only when the code outside its quotes is at most 8 KiB. A `script` type that is not JavaScript is not executed. No API key is stored.
- Numeric x86-64 machine code. A `while` or `for` whose body is only numbers is compiled and the numbers are written back into the page script. The same loop may assign `document.getElementById("...").textContent` to a number. That one write calls back into the page. Other property reads stay on the interpreter. A whole script with no DOM, `fetch()`, or storage can take the numeric path. On Linux the page is mapped writable, filled, then marked read and execute, and calls use the System V argument order. On Windows the page is allocated with `VirtualAlloc`, and calls use `rcx`, `rdx`, and `xmm2`, with 32 bytes of shadow space. The caller owns the VM context. The worker process has its own context. This is not a general compiler.

## What is not on

- One OS process per tab. A remote navigation may start one worker and wait for it. The window process still owns the tabs.
- The real YouTube player bundle, the Gmail inbox application, the Facebook feed, the current Reddit application, and bank web apps. Those are script bundles this interpreter does not run. The HTML document is still loaded. A YouTube watch page adds a player built from YouTube's published JPEG frames, or a storyboard sheet when the page includes one. That player is not the H.264 stream, and no codec library was added. After Gmail's HTML sign-in form is posted, Google's own page says JavaScript is required. Reddit stays on the address you opened, including `www.reddit.com`, and is not sent to `old.reddit.com`. Requests to a Reddit host use a longer User-Agent that names this browser. A network-policy 403 is Reddit's, and this browser does not bypass it. No password is stored for a bank or a social site.
- A hardware GPU page rasterizer. Shadows, rounded boxes, and gradients are drawn by `anvil-gpu`'s software device. The bytes match the CPU compositor. Square fills, text, and ordinary images stay on the older path. Windows can upload the finished CPU framebuffer to Direct3D 11 and compile `compositor.hlsl` to sample that upload. Linux still copies pixels with X11. There is no GPU video decoder. The VA-API and Direct3D names in `anvil-video` are labels. Video frames are decoded on the CPU, then drawn through the same software device before the controls are painted.
