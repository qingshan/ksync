#!/bin/sh
# Installs ksync to /mnt/us/ksync.
#
# Everything is bundled (the ksyncd binary, WAF, scripts, scriptlet), so the
# install is fully local and deterministic - no network needed. Binaries and
# scripts are always refreshed so upgrades pick up new code, but the user's
# catalogs.json (var/) is left untouched if it already exists, so upgrading
# never wipes configured OPDS catalogs.

set -e

SCRIPT_DIR=$(dirname -- "$(readlink -f -- "$0")")
if [ -f "$SCRIPT_DIR/common/pkg-lib.sh" ]; then
    . "$SCRIPT_DIR/common/pkg-lib.sh"
elif [ -f "$SCRIPT_DIR/../common/pkg-lib.sh" ]; then
    . "$SCRIPT_DIR/../common/pkg-lib.sh"
else
    echo "error: pkg-lib.sh not found" >&2
    exit 1
fi
. "$SCRIPT_DIR/pkg.env"

pkg_install

echo "ksync installed. Tap the ksync entry on your Home screen to open the"
echo "WAF and add OPDS catalogs."
