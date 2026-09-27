# Agent notes

Standalone **kpm** package for a jailbroken Kindle Oasis 10th gen (`kindlehf`,
firmware 5.16.x “juno”, Mesquite WAF): OPDS catalog sync into Kindle Library
collections.

## Commands

```sh
just test                 # cargo test -p ksync --lib
just package              # cross-compile kindlehf + pack .kpkg into dist/
just package <x.y.z>      # bump kpm package version while packing
just toolchain            # once: ~/x-tools kindlehf gcc + liblipc stub
```

Kindle crates **cannot be linked natively** (`liblipc`). Use
`cargo test -p ksync --lib`; do not build `ksyncd` on the host.

`.kpkg` files under `dist/` are build output.

## Version bumps

KPM only reinstalls when the version changes. After daemon, WAF, or script
edits, bump **all** of:

- `kpm/manifest.json`
- `crates/ksync/Cargo.toml`
- `kpm/waf/index.html` `?v=` on `style.css` and `script.js`

`just package <x.y.z>` rewrites the kpm bits; still bump the crate version.

## Package shape

Rust LIPC daemon + Mesquite WAF.

| Daemon / LIPC | On-device |
| --- | --- |
| `ksyncd` `dev.qingshan.ksyncd` | `/mnt/us/ksync` |

Shared Kindle helpers: [`common/`](common/) (`pkg-lib.sh`, `waf-base.js/css`).
Install scripts are POSIX `sh` (Kindle ash). `KEEP_ON_UPGRADE` (`var/`)
survives reinstall — do not wipe user config (`catalogs.json`).

WAF talks one-way JSON on the LIPC `cmd` property (`sendJsonCmd`) and
**polls** `status.json` in `/var/local/mesquite/ksync/`. No fetch, no sync
LIPC RPC. The daemon writes that file.

## WAF (Mesquite WebKitGTK 1.0.7.2)

- **JS:** ES5 only — `var`, no `const`/`let`, arrows, template literals,
  Promises, `class`, modules.
- **CSS:** CSS2 — no flexbox, grid, or `position:fixed`.
- Prefer `mousedown` over `click` (this WebKit often never delivers
  mouseup/click). Invert a beat (`PRESS_FLASH_MS`) before navigation so
  e-ink shows the tap.
- One overlay at a time (dropdown / modal / error pop).

## Style

- Rust 2021, `cargo fmt`. Host-testable logic in `crates/ksync/src/*.rs`
  (`#[cfg(test)]`); keep `src/bin/ksyncd.rs` as the LIPC/IO shell.
- Commit subject: imperative, what changed (“Add ksync catalog edit form”).
- Comments: short and factual; no changelog narration.
- Do not add a native `cargo build` of Kindle bins to CI-style checks on
  the host.
