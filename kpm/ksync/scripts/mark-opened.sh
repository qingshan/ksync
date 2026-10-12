#!/bin/sh
# Ask the Library scanner to clear the scriptlet's NEW tag after the WAF opens.

_i=0
while [ "$_i" -lt 20 ]; do
    [ "$(lipc-get-prop com.lab126.appmgrd activeApp 2>/dev/null)" = "dev.qingshan.ksync" ] && break
    sleep 1
    _i=$((_i + 1))
done
[ "$_i" -lt 20 ] || exit 0

cover_is_new() {
    [ "$(sqlite3 /var/local/cc.db "SELECT 1 FROM Entries WHERE p_location='/mnt/us/documents/ksync.sh' AND j_displayTags LIKE '%\"NEW\"%' LIMIT 1;" 2>/dev/null)" = 1 ]
}

# A scan may race with the Library database update, so retry once if needed.
_scan_i=0
while [ "$_scan_i" -lt 2 ] && cover_is_new; do
    sleep 3
    touch /mnt/us/documents/ksync.sh || exit 0
    lipc-set-prop com.lab126.scanner doFullScan 1 >/dev/null 2>&1 || exit 0
    sleep 3
    _scan_i=$((_scan_i + 1))
done
