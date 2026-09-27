/* One-way LIPC commands; status.json is polled every three seconds. */
var refreshTimer = null;
(function () {
    "use strict";
    var KSYNC_ID = "dev.qingshan.ksyncd";
    var lastData = null;
    var prefixDirty = false;

    function catalogById(id) {
        if (!lastData || !lastData.catalogs) {
            return null;
        }
        for (var i = 0; i < lastData.catalogs.length; i++) {
            if (lastData.catalogs[i].id === id) {
                return lastData.catalogs[i];
            }
        }
        return null;
    }

    function sendCmd(op) {
        if (sendJsonCmd(KSYNC_ID, op)) {
            setError("");
            return true;
        }
        return false;
    }

    function fetchStatusJson() {
        pollStatusJson(receiveStatus, "status.json missing - is ksyncd running? Tap the Home screen icon to reopen the app (it restarts the daemon).");
    }

    function receiveStatus(data) {
        lastData = data;
        KSyncUI.render(data);
        var prefix = byId("f-global-prefix");
        if (!prefixDirty && document.activeElement !== prefix && data.collectionPrefix !== undefined) {
            prefix.value = data.collectionPrefix;
        }
    }

    function submitForm() {
        var command = KSyncForm.command();
        if (command && sendCmd(command)) {
            KSyncForm.clear();
            KSyncUI.showView("catalogs");
        }
    }

    function catalogAction(op, id, opener) {
        if (op === "sync") {
            sendCmd({ op: "sync_start", id: id });
        } else if (op === "edit") {
            var catalog = catalogById(id);
            if (!catalog) {
                setError("Could not read catalog details");
                return;
            }
            KSyncForm.edit(catalog);
            KSyncUI.showView("add", opener);
        }
    }

    document.addEventListener("DOMContentLoaded", function () {
        hookChromeOnGo("dev.qingshan.ksync", "ksync");
        startAutoRefresh(fetchStatusJson, 3000);

        KSyncUI.bindPress("btn-sync-all", function () {
            sendCmd({ op: "sync_all" });
        });
        KSyncUI.bindPress("btn-stop", function () {
            sendCmd({ op: "sync_stop" });
        });
        KSyncUI.bindPress("btn-refresh", fetchStatusJson);
        KSyncUI.bindPress("btn-collections", function () {
            sendCmd({ op: "collections_rebuild" });
        });

        byId("f-global-prefix").addEventListener("input", function () { prefixDirty = true; });
        KSyncUI.bindPress("btn-prefix-apply", function () {
            if (sendCmd({ op: "set_collection_prefix", prefix: byId("f-global-prefix").value })) {
                prefixDirty = false;
            }
        });
        KSyncUI.bindPress("btn-settings", function () { KSyncUI.showView("settings", byId("btn-settings")); });
        KSyncUI.bindPress("btn-prev", function () {
            if (lastData) { KSyncUI.changePage(-1, lastData); }
        });
        KSyncUI.bindPress("btn-next", function () {
            if (lastData) { KSyncUI.changePage(1, lastData); }
        });
        KSyncUI.bindPress("btn-settings-close", function () { KSyncUI.showView("catalogs"); });
        KSyncUI.bindPress("btn-add", function () {
            KSyncForm.clear();
            KSyncUI.showView("add", byId("btn-add"));
        });
        KSyncUI.bindPress("btn-form-delete", function () {
            byId("delete-confirm").style.display = "";
        });
        KSyncUI.bindPress("btn-delete-cancel", function () {
            byId("delete-confirm").style.display = "none";
        });
        KSyncUI.bindPress("btn-delete-confirm", function () {
            var id = KSyncForm.editingId();
            if (id && sendCmd({ op: "catalog_remove", id: id })) {
                KSyncForm.clear();
                KSyncUI.showView("catalogs");
            }
        });
        KSyncUI.bindPress("btn-form-submit", submitForm);
        KSyncUI.bindPress("btn-form-cancel", function () {
            KSyncForm.clear();
            KSyncUI.showView("catalogs");
        });

        document.addEventListener("keydown", KSyncUI.dialogKey);
        KSyncUI.bindCatalogPress(catalogAction);
        KSyncUI.showView("catalogs");
        fetchStatusJson();
    });
}());
