/* Run with Node: exercises the shipped ES5 WAF without a Kindle. */
var assert = require("assert");
var fs = require("fs");
var vm = require("vm");
var markup = fs.readFileSync("kpm/waf/index.html", "utf8");
var elements = {};
var timers = [];
var sent = [];
var domReady;
var receiveStatus;
var page = { setAttribute: function () {}, insertBefore: function () {} };

function makeElement(id) {
    var el = { id: id, className: "", value: "", checked: false, innerHTML: "",
        parentNode: page, focus: function () { document.activeElement = this; }, appendChild: function () {},
        style: {}, listeners: {}, attrs: {}, disabled: false,
        addEventListener: function (kind, fn) { this.listeners[kind] = fn; },
        getAttribute: function (name) { return this.attrs[name] || null; },
        setAttribute: function (name, value) { this.attrs[name] = value; } };
    elements[id] = el;
    return el;
}
markup.replace(/\bid="([^"]+)"/g, function (_, id) { makeElement(id); });
var document = { documentElement: { contains: function () { return true; } }, addEventListener: function (kind, fn) { if (kind === "DOMContentLoaded") { domReady = fn; } } };
var context = {
    document: document,
    window: { confirm: function () { return true; } },
    byId: function (id) { return elements[id]; },
    setTimeout: function (fn) { timers.push(fn); return timers.length; },
    sendJsonCmd: function (_, op) { sent.push(op); return true; },
    setError: function (text) { elements.error.innerHTML = text; },
    setState: function (text, cls) { elements.state.innerHTML = text; elements.state.className = cls; },
    pollStatusJson: function (render) { receiveStatus = render; }, startAutoRefresh: function () {}, hookChromeOnGo: function () {},
    esc: function (text) { return String(text).replace(/&/g, "&amp;").replace(/</g, "&lt;"); }
};
vm.createContext(context);
markup.replace(/<script[^>]+src="([^"?]+)(?:\?[^" ]*)?"/g, function (_, source) {
    if (source !== "waf-base.js") {
        vm.runInContext(fs.readFileSync("kpm/waf/" + source, "utf8"), context, { filename: source });
    }
});
assert(domReady, "WAF must register its startup handler");
domReady();
function flush() { while (timers.length) { timers.shift()(); } }
function press(id) {
    var ev = { preventDefault: function () { this.prevented = true; } };
    elements[id].listeners.mousedown(ev);
    assert(ev.prevented, id + " consumes the unreliable click path");
    flush();
}

receiveStatus({ state: "idle", downloaded: 3, skipped: 1, failed: 0,
    collectionsAdded: 2, collectionsPending: 0, current: "", collectionPrefix: "Books",
    catalogs: [{ id: "one", name: "Main", url: "https://example.test/opds", enabled: true, insecure: false }] });
assert.equal(elements.state.innerHTML, "Idle");
assert.equal(elements["f-global-prefix"].value, "Books");
assert(/Downloaded 3/.test(elements.progress.innerHTML));
assert(/catalog-one/.test(elements["catalog-list"].innerHTML));
press("btn-sync-all");
assert.equal(sent.pop().op, "sync_all");
press("btn-add");
assert.equal(elements["view-add"].style.display, "");
assert.equal(elements["dialog-overlay"].style.display, "");
assert.equal(elements.dialog.attrs["aria-labelledby"], "form-title");
console.log("KSync WAF E2E: render, press feedback, commands, and view switch OK");

var data = { state: "idle", downloaded: 0, skipped: 0, failed: 0,
    collectionPrefix: "Books", catalogs: [] };
for (var i = 0; i < 7; i++) {
    data.catalogs.push({ id: "source-" + i, name: "Source " + i,
        url: "https://example.test/opds", enabled: true });
}
receiveStatus(data);
assert(/catalog-source-0/.test(elements["catalog-list"].innerHTML));
assert(!/catalog-source-3/.test(elements["catalog-list"].innerHTML));
press("btn-next");
assert(/catalog-source-3/.test(elements["catalog-list"].innerHTML));
press("btn-next");
assert.equal(elements["page-number"].innerHTML, "3 / 3");
assert(elements["btn-next"].disabled);
data.catalogs = data.catalogs.slice(0, 1);
receiveStatus(data);
assert(/catalog-source-0/.test(elements["catalog-list"].innerHTML));
assert.equal(elements.pagination.style.display, "none");
press("btn-settings");
assert.equal(elements["view-settings"].style.display, "");
assert.equal(elements["view-catalogs"].style.display, "");
assert.equal(elements["view-add"].style.display, "none");
assert.equal(elements.dialog.attrs["aria-labelledby"], "settings-title");
elements["f-global-prefix"].value = "Draft";
elements["f-global-prefix"].listeners.input();
receiveStatus(data);
assert.equal(elements["f-global-prefix"].value, "Draft");
press("btn-prefix-apply");
assert.equal(sent.pop().prefix, "Draft");
data.state = "running";
data.current = "<Book & title>";
receiveStatus(data);
assert.equal(elements.current.innerHTML, "&lt;Book &amp; title>");
assert(elements["btn-sync-all"].disabled);
assert(!elements["btn-stop"].disabled);
var before = sent.length;
elements["btn-sync-all"].listeners.mousedown({ preventDefault: function () {} });
flush();
assert.equal(sent.length, before);
press("btn-add");
elements["f-url"].value = "https://example.test/opds";
context.sendJsonCmd = function () { return false; };
press("btn-form-submit");
assert.equal(elements["f-url"].value, "https://example.test/opds");
assert.equal(elements["view-add"].style.display, "");
console.log("KSync WAF: pagination, settings drafts, running controls, and failed submissions OK");

context.sendJsonCmd = function (_, op) { sent.push(op); return true; };
function catalogPress(op, id, nested, disabled) {
    var button = makeElement("test-catalog-action");
    button.attrs = { "data-op": op, "data-id": id };
    button.disabled = !!disabled;
    var target = nested ? { parentNode: button } : button;
    var ev = { target: target, preventDefault: function () { this.prevented = true; } };
    var count = sent.length;
    elements["catalog-list"].listeners.mousedown(ev);
    assert.equal(sent.length, count, "actions wait for the e-ink press flash");
    if (!disabled) { assert(ev.prevented); }
    flush();
}
data.state = "idle";
receiveStatus(data);
catalogPress("edit", "source-0", true);
assert.equal(elements["form-title"].innerHTML, "Edit catalog");
assert.equal(elements["f-url"].value, "https://example.test/opds");
assert.equal(elements["f-password"].value, "");
elements["f-url"].value = " https://renamed.test/opds ";
press("btn-form-submit");
var update = sent.pop();
assert.equal(update.op, "catalog_update");
assert.equal(update.id, "source-0");
assert.equal(update.url, "https://renamed.test/opds");
assert.equal(update.password, "");
assert.equal(elements["view-catalogs"].style.display, "");
press("btn-add");
elements["f-url"].value = " https://new.test/opds ";
press("btn-form-submit");
var added = sent.pop();
assert.equal(added.op, "catalog_add");
assert(!("id" in added), "successful edits clear the form's catalog identity");
assert.equal(added.url, "https://new.test/opds");
assert.equal(added.name, "", "OPDS feed titles supply catalog names");
assert.equal(added.enabled, true);
press("btn-add");
before = sent.length;
press("btn-form-submit");
assert.equal(sent.length, before);
assert.equal(elements["form-error"].innerHTML, "OPDS URL is required");
receiveStatus(data);
assert.equal(elements["form-error"].innerHTML, "OPDS URL is required");
press("btn-form-cancel");
assert.equal(elements["dialog-overlay"].style.display, "none");
catalogPress("sync", "source-0", false, true);
assert.equal(sent.length, before);
catalogPress("sync", "source-0");
assert.equal(sent.pop().op, "sync_start");
catalogPress("edit", "source-0");
press("btn-form-delete");
assert.equal(elements["delete-confirm"].style.display, "");
assert.equal(sent.length, before);
press("btn-delete-cancel");
assert.equal(elements["delete-confirm"].style.display, "none");
press("btn-form-delete");
press("btn-delete-confirm");
assert.equal(sent.pop().op, "catalog_remove");
assert.equal(elements["dialog-overlay"].style.display, "none");
press("btn-add");
assert.equal(elements["btn-form-delete"].style.display, "none");
assert.equal(elements["delete-confirm"].style.display, "none");
press("btn-form-cancel");
catalogPress("edit", "missing");
assert.equal(elements.error.innerHTML, "Could not read catalog details");
catalogPress("edit", "source-0");
press("btn-form-cancel");
assert.equal(elements["form-title"].innerHTML, "Add catalog");
assert.equal(elements["view-catalogs"].style.display, "");
console.log("KSync WAF: delegated presses, edit/add payloads, validation, cancel, and delete confirmation OK");

receiveStatus({ state: "idle", catalogs: [] });
assert(elements["btn-sync-all"].disabled);
assert.equal(elements["btn-stop"].style.display, "none");
assert(/Your library starts here/.test(elements["catalog-list"].innerHTML));
assert(!/data-op="delete"|data-op="toggle"/.test(elements["catalog-list"].innerHTML));
press("btn-settings");
press("btn-settings-close");
assert.equal(elements["dialog-overlay"].style.display, "none");
console.log("KSync WAF: dialogs, inline delete confirmation, persistent validation, and empty state OK");
