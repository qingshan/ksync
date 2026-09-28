# End-to-end tests and demo

`tests/e2e/ksync_e2e.py` is the host test entry point. Like kmux, `just e2e`
runs host scenarios and `just demo` adds a real-Kindle tour and media encoding.
No Kindle binary is linked on the host.

## Setup

Use Node.js 20 or newer, Python 3.9 or newer, and the Rust toolchain:

```sh
npm ci
npx playwright install chromium
just test
just e2e
```

On a minimal Linux machine, Playwright may also require its system browser
dependencies (`npx playwright install-deps chromium`).

`just test` runs Rust library tests and the fast JavaScript VM harness.
`just e2e` runs that harness plus Chromium against the shipped HTML, CSS,
scripts, shared helpers, and three-second status polling. Only status.json
responses and Kindle messaging are fixtures; no OPDS server or daemon runs in
the browser suite. It asserts exact command payloads, not successful downloads.

Coverage includes empty catalogs, three-row paging and page clamping, escaped
catalog titles, manual sync of excluded catalogs, Sync all, Stop and stopping
controls, add/edit/delete confirmation, failed delivery, validation surviving
polling, settings drafts, rebuild commands, focus trapping/restoration, Escape,
status parse failures and recovery, and native-resolution layout. Mouse presses
remain held past the feedback delay to exercise the missing-mouseup path.
Screenshots, including a failure checkpoint, go to `target/e2e/browser/`.
Chromium checks do not substitute for Mesquite compatibility testing.

## Real Kindle demo

Install the current package on a Kindle Oasis and configure passwordless SSH
using the `kindle` alias. Set `KINDLE_E2E_HOST` to use another alias. Install
Pillow, Tesseract OCR (English), and ffmpeg on the host. The input helper needs
the Kindle cross compiler from `just toolchain` and X11 headers on the host.

```sh
just demo
just demo-build  # re-encode verified existing frames without a Kindle
```

The tour requires a 1264×1680 framebuffer and matching installed WAF assets.
It refuses to run during an active sync. It launches through the Library shell integration, requires the app to remain
in the foreground, refreshes,
opens Add catalog, checks missing-field validation, cancels, opens Edit and the
delete confirmation, cancels, opens Settings, and returns to the catalog list.
Edit/delete scenes are omitted when there are no catalogs. Tesseract locates
button labels in current screenshots and checks each scene before accepting it;
there are no hardcoded coordinates to silently drift after a layout change.

The tour shows existing catalog names and URLs. It never saves the form,
confirms deletion, changes settings, or starts downloads. It verifies the
configuration checksum is unchanged and closes any remaining dialog in cleanup.
It leaves ksync open on its main page. Do not interact with the Kindle during
capture. A failed capture saves `target/demo/frames/failure.png` and does not
produce a successful storyboard.

Raw framebuffer PNGs and a manifest of assertions, captions, and SHA-256 hashes
live in `target/demo/`. The encoder verifies those hashes, adds captions over
the system status bar, and writes `ksync-demo.mp4`, `ksync-demo.gif`, and
`ksync-demo-poster.png` to `dist/demo/`. `just demo` also updates
`docs/ksync-demo.gif` and `docs/ksync.png` for the README. It does not publish
anything. To refresh those files from an existing recording, run
`python3 tools/demo_build.py --readme`.
