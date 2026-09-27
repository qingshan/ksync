/* ES5 only: loaded as a plain script by Mesquite. */
var KSyncUI = (function () {
    "use strict";
    var catalogPage = 0;
    var CATALOGS_PER_PAGE = 3;
    var PRESS_FLASH_MS = 300;
    var dialogOpener = null;
    var activeView = "catalogs";

    function flashPressed(el) {
        if (!el) {
            return;
        }
        var active = (" " + (el.className || "") + " ").indexOf(" active ") >= 0 ||
            (" " + (el.className || "") + " ").indexOf(" primary ") >= 0;
        el.style.backgroundColor = active ? "#fff" : "#000";
        el.style.color = active ? "#000" : "#fff";
        setTimeout(function () {
            el.style.backgroundColor = "";
            el.style.color = "";
        }, PRESS_FLASH_MS);
    }

    /* Mesquite can lose mouseup/click; delay actions until the press is visible. */
    function press(el, ev, fn) {
        if (!el || el.disabled) { return; }
        if (ev.preventDefault) { ev.preventDefault(); }
        flashPressed(el);
        setTimeout(fn, PRESS_FLASH_MS);
    }

    function bindPress(id, fn) {
        var el = byId(id);
        el.addEventListener("mousedown", function (ev) { press(el, ev, fn); });
        el.addEventListener("keydown", function (ev) {
            if (ev.keyCode === 13 || ev.keyCode === 32) { press(el, ev, fn); }
        });
    }

    function bindCatalogPress(fn) {
        var list = byId("catalog-list");
        function handle(ev) {
            if (ev.type === "keydown" && ev.keyCode !== 13 && ev.keyCode !== 32) { return; }
            var target = ev.target || ev.srcElement;
            while (target && target !== list) {
                var op = target.getAttribute && target.getAttribute("data-op");
                var id = target.getAttribute && target.getAttribute("data-id");
                if (op && id) {
                    press(target, ev, function () { fn(op, id, target); });
                    return;
                }
                target = target.parentNode;
            }
        }
        list.addEventListener("mousedown", handle);
        list.addEventListener("keydown", handle);
    }

    function setProgress(text) {
        var el = byId("progress");
        if (el) {
            el.innerHTML = esc(text || "\u00a0");
        }
    }

    function setCurrent(text) {
        var el = byId("current");
        if (el) {
            el.innerHTML = esc(text || "\u00a0");
        }
    }

    function taskRunning(data) {
        return data.state === "running" || data.state === "stopping";
    }

    function render(data) {
        var errors = [];
        if (data.lastError) {
            errors.push(data.lastError);
        }
        if (data.collectionsError) {
            errors.push("collections: " + data.collectionsError);
        }
        setError(esc(errors.join(" \u00b7 ")));

        var label = data.state;
        var cssClass = "state-" + data.state;
        setState(esc(label.charAt(0).toUpperCase() + label.slice(1)), cssClass);
        byId("btn-sync-all").disabled = taskRunning(data) || !(data.catalogs || []).some(function (c) { return c.enabled; });
        byId("btn-stop").style.display = taskRunning(data) ? "" : "none";
        byId("btn-stop").disabled = data.state !== "running";
        byId("btn-collections").disabled = taskRunning(data);

        var parts = [];
        parts.push("Downloaded " + data.downloaded);
        parts.push("skipped " + data.skipped);
        parts.push("failed " + data.failed);
        if (data.collectionsAdded > 0 || data.collectionsPending > 0) {
            parts.push("collections +" + data.collectionsAdded);
            if (data.collectionsPending > 0) {
                parts.push(data.collectionsPending + " pending");
            }
        }
        setProgress(parts.join(" \u00b7 "));
        setCurrent(data.current || (taskRunning(data) ? data.catalogName : "Ready when you are."));

        renderCatalogs(data);
    }

    function renderCatalogs(data) {
        var list = byId("catalog-list");
        if (!list) {
            return;
        }
        var running = taskRunning(data);
        var html = "";
        var count = (data.catalogs || []).length;
        var pages = Math.max(1, Math.ceil(count / CATALOGS_PER_PAGE));
        catalogPage = Math.max(0, Math.min(catalogPage, pages - 1));
        byId("catalog-count").innerHTML = "(" + count + ")";
        byId("pagination").style.display = pages > 1 ? "" : "none";
        byId("page-number").innerHTML = (catalogPage + 1) + " / " + pages;
        byId("btn-prev").disabled = catalogPage === 0;
        byId("btn-next").disabled = catalogPage === pages - 1;
        if (!data.catalogs || data.catalogs.length === 0) {
            html = '<div class="empty"><strong>Your library starts here.</strong>Choose Add catalog to connect an OPDS source.</div>';
        } else {
            for (var i = catalogPage * CATALOGS_PER_PAGE; i < Math.min(count, (catalogPage + 1) * CATALOGS_PER_PAGE); i++) {
                var c = data.catalogs[i];
                html += '<div class="catalog" id="catalog-' + esc(c.id) + '">';
                html += '<div class="catalog-meta">' +
                    (c.enabled ? "In sync all" : "Manual sync") +
                    (c.insecure ? " \u00b7 TLS unchecked" : "") + '</div>';
                html += '<div class="catalog-title">' + esc(c.name) + '</div>';
                html += '<div class="catalog-url" title="' + esc(c.url) + '">' + esc(c.url) + '</div>';
                html += '<div class="buttons">';
                html += '<button type="button" class="primary" data-op="sync" data-id="' + esc(c.id) + '"' +
                    (running ? " disabled" : "") + '>Sync</button>';
                html += '<button type="button" data-op="edit" data-id="' + esc(c.id) + '">Edit</button>';
                html += '</div></div>';
            }
        }
        if (list.innerHTML !== html) { list.innerHTML = html; }
    }

    function showView(name, opener) {
        activeView = name;
        var open = name !== "catalogs";
        byId("view-add").style.display = name === "add" ? "" : "none";
        byId("view-settings").style.display = name === "settings" ? "" : "none";
        byId("dialog-overlay").style.display = open ? "" : "none";
        byId("view-catalogs").style.display = "";
        byId("view-catalogs").parentNode.setAttribute("aria-hidden", open ? "true" : "false");
        if (open) {
            dialogOpener = opener || document.activeElement;
            byId("dialog-errors").appendChild(byId("error"));
            byId("dialog").setAttribute("aria-labelledby", name === "add" ? "form-title" : "settings-title");
            byId("dialog").focus();
        } else {
            byId("view-catalogs").parentNode.insertBefore(byId("error"), byId("view-catalogs"));
            if (dialogOpener && document.documentElement.contains(dialogOpener)) { dialogOpener.focus(); }
            else if (dialogOpener) { byId("btn-add").focus(); }
            dialogOpener = null;
        }
    }

    function dialogKey(ev) {
        if (activeView === "catalogs") { return; }
        if (ev.keyCode === 27) {
            ev.preventDefault();
            KSyncForm.clear();
            showView("catalogs");
        } else if (ev.keyCode === 9) {
            var controls = byId("view-" + activeView).querySelectorAll("button, input");
            var available = [];
            for (var i = 0; i < controls.length; i++) {
                if (!controls[i].disabled && controls[i].offsetHeight > 0) { available.push(controls[i]); }
            }
            var first = available[0];
            var last = available[available.length - 1];
            if (first && (document.activeElement === byId("dialog") ||
                    (ev.shiftKey && document.activeElement === first) ||
                    (!ev.shiftKey && document.activeElement === last))) {
                ev.preventDefault();
                (ev.shiftKey ? last : first).focus();
            }
        }
    }

    function changePage(delta, data) {
        catalogPage += delta;
        renderCatalogs(data);
    }

    return {
        render: render,
        dialogKey: dialogKey,
        changePage: changePage,
        showView: showView,
        bindPress: bindPress,
        bindCatalogPress: bindCatalogPress
    };
}());
