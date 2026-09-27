# ksync Kindle OPDS catalog sync.

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Host-testable daemon logic. Do not natively link ksyncd (liblipc).
test:
    cargo test -p ksync --lib
    node tests/e2e/waf_e2e.js

# Exercise the shipped WAF in the VM harness and a real browser.
e2e:
    python3 tests/e2e/ksync_e2e.py

# Capture safe WAF navigation on a real Kindle, then encode the framebuffer tour.
demo:
    ./tools/build-kindle-xinput.sh
    scp -q target/kindlehf-x11/bin/kindle_xinput "${KINDLE_E2E_HOST:-kindle}:/tmp/kindle_xinput"
    ssh "${KINDLE_E2E_HOST:-kindle}" 'chmod +x /tmp/kindle_xinput'
    python3 tests/e2e/ksync_e2e.py --record

demo-build:
    python3 tools/demo_build.py

# Install kindlehf gcc (~/x-tools) and the liblipc link stub.
toolchain:
    ./tools/setup-toolchain.sh

# Cross-compile ksyncd and write a .kpkg to dist/.
# Usage: just package [version]
package version="": toolchain
    ./tools/pkg-build.sh {{version}}

# Publish an existing GitHub Release download URL to the shared KPM catalog.
# Requires KINDLE_CATALOG_TOKEN when the catalog remote uses HTTPS.
publish version="":
    ./tools/publish.sh {{version}}
