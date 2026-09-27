/* ES5 only: loaded as a plain script by Mesquite. */
var KSyncForm = (function () {
    "use strict";
    var editingId = null;

    function formValues() {
        return {
            name: byId("f-name").value.replace(/^\s+|\s+$/g, ""),
            url: byId("f-url").value.replace(/^\s+|\s+$/g, ""),
            username: byId("f-username").value,
            password: byId("f-password").value,
            insecure: byId("f-insecure").checked,
            enabled: byId("f-enabled").checked
        };
    }

    function clearForm() {
        editingId = null;
        byId("form-error").innerHTML = "";
        byId("btn-form-delete").style.display = "none";
        byId("delete-confirm").style.display = "none";
        byId("f-name").value = "";
        byId("f-url").value = "";
        byId("f-username").value = "";
        byId("f-password").value = "";
        byId("f-insecure").checked = false;
        byId("f-enabled").checked = true;
        byId("form-title").innerHTML = "Add catalog";
        byId("btn-form-submit").innerHTML = "Add catalog";
        byId("btn-form-cancel").style.display = "";
    }

    function fillForm(c) {
        clearForm();
        editingId = c.id;
        byId("btn-form-delete").style.display = "";
        byId("f-name").value = c.name;
        byId("f-url").value = c.url;
        byId("f-username").value = "";
        byId("f-password").value = "";
        byId("f-insecure").checked = !!c.insecure;
        byId("f-enabled").checked = !!c.enabled;
        byId("form-title").innerHTML = "Edit catalog";
        byId("btn-form-submit").innerHTML = "Save";
        byId("btn-form-cancel").style.display = "";
    }

    function command() {
        var values = formValues();
        if (!values.name || !values.url) {
            byId("form-error").innerHTML = "Name and URL are required";
            return null;
        }
        byId("form-error").innerHTML = "";
        values.op = editingId ? "catalog_update" : "catalog_add";
        if (editingId) { values.id = editingId; }
        return values;
    }

    return { clear: clearForm, edit: fillForm, command: command, editingId: function () { return editingId; } };
}());
