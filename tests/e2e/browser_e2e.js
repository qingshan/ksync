/* Real DOM and shipped polling/command helpers; only the Kindle boundary is stubbed. */
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { chromium } = require('playwright');
const ROOT = path.resolve(__dirname, '../..');
const OUT = path.join(ROOT, 'target/e2e/browser');

async function main() {
    fs.mkdirSync(OUT, { recursive: true });
    const browser = await chromium.launch();
    const page = await browser.newPage({ viewport: { width: 1264, height: 1464 } });
    const errors = [];
    const commands = [];
    let polls = 0;
    let brokenStatus = false;
    const state = { state: 'idle', downloaded: 0, skipped: 0, failed: 0,
        collectionPrefix: 'KSync', catalogs: [] };
    const catalog = i => ({ id: 'source-' + i, name: 'Catalog ' + i,
        url: 'https://example.test/' + i + '/opds', enabled: i !== 1, insecure: false });
    page.on('pageerror', error => errors.push(error.message));
    await page.exposeFunction('recordCommand', (app, property, payload) => {
        assert.equal(app, 'dev.qingshan.ksyncd');
        assert.equal(property, 'cmd');
        commands.push(JSON.parse(payload));
    });
    await page.addInitScript(() => {
        window.kindle = { messaging: {
            sendMessage: function () {},
            sendStringMessage: function (app, property, payload) {
                if (window.failDelivery) { throw new Error('fixture transport failure'); }
                window.recordCommand(app, property, payload);
            }
        } };
    });
    await page.route('http://ksync.test/**', async route => {
        const name = new URL(route.request().url()).pathname.slice(1) || 'index.html';
        if (name === 'status.json') {
            polls++;
            return route.fulfill({ contentType: 'application/json',
                body: brokenStatus ? '{invalid' : JSON.stringify(state) });
        }
        const directory = name.startsWith('waf-base.') ? 'common' : 'kpm/waf';
        await route.fulfill({ path: path.join(ROOT, directory, name) });
    });
    async function expect(fn, message) {
        for (let i = 0; i < 60; i++) {
            if (await fn()) { return; }
            await page.waitForTimeout(100);
        }
        throw new Error(message);
    }
    async function tap(selector) {
        const box = await page.locator(selector).boundingBox();
        assert(box, selector + ' is visible');
        // Deliberately omit mouseup until after the action: Mesquite can lose it.
        await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
        await page.mouse.down();
        await page.waitForTimeout(350);
        await page.mouse.up();
    }
    async function refresh() {
        const previous = polls;
        await tap('#btn-refresh');
        await expect(() => polls > previous, 'status poll completed');
    }
    async function command(expected) {
        await expect(() => commands.length > 0, 'command delivered');
        assert.deepEqual(commands.shift(), expected);
    }
    async function shot(name) {
        await page.screenshot({ path: path.join(OUT, name + '.png') });
    }
    try {
        await page.goto('http://ksync.test');
        // Mesquite uses a large default font at native framebuffer resolution.
        await page.addStyleTag({ content: 'body { font-size: 48px; }' });
        await expect(() => page.locator('.empty').isVisible(), 'empty catalog list');
        assert(await page.locator('#btn-sync-all').isDisabled());
        await shot('01-empty');
        state.catalogs = Array.from({ length: 7 }, (_, i) => catalog(i));
        state.catalogs[0].name = '<Science & stories>';
        await refresh();
        assert.equal(await page.locator('.catalog').count(), 3);
        assert.equal(await page.locator('.catalog-title').first().innerText(), '<Science & stories>');
        const bottom = await page.locator('#pagination').boundingBox();
        assert(bottom.y + bottom.height <= 1464, 'three catalogs and paging fit');
        await shot('02-catalogs');
        await tap('#btn-next');
        assert(await page.locator('#catalog-source-3').isVisible());
        await tap('#btn-next');
        assert(await page.locator('#btn-next').isDisabled());
        state.catalogs = state.catalogs.slice(0, 2);
        await refresh();
        assert(await page.locator('#catalog-source-0').isVisible());
        assert(!(await page.locator('#pagination').isVisible()));

        await tap('#catalog-source-1 [data-op="sync"]');
        await command({ op: 'sync_start', id: 'source-1' });
        await tap('#btn-sync-all');
        await command({ op: 'sync_all' });
        state.state = 'running';
        state.current = '<Book & title>';
        await refresh();
        assert(await page.locator('#btn-sync-all').isDisabled());
        assert(await page.locator('[data-op="sync"]').first().isDisabled());
        assert.equal(await page.locator('#current').innerText(), '<Book & title>');
        await shot('03-running');
        await tap('#btn-stop');
        await command({ op: 'sync_stop' });
        state.state = 'stopping';
        await refresh();
        assert(await page.locator('#btn-stop').isDisabled());
        state.state = 'idle';
        state.current = '';
        await refresh();

        await tap('#btn-add');
        await page.keyboard.press('Tab');
        assert.equal(await page.evaluate(() => document.activeElement.id), 'f-name');
        await page.keyboard.press('Shift+Tab');
        assert.equal(await page.evaluate(() => document.activeElement.id), 'btn-form-cancel');
        await tap('#btn-form-submit');
        assert.equal(await page.locator('#form-error').innerText(), 'Name and URL are required');
        const prior = polls;
        await expect(() => polls > prior, 'automatic polling continues behind dialog');
        assert.equal(await page.locator('#form-error').innerText(), 'Name and URL are required');
        await page.locator('#f-name').fill(' New catalog ');
        await page.locator('#f-url').fill(' https://example.test/new/opds ');
        await page.evaluate(() => { window.failDelivery = true; });
        await tap('#btn-form-submit');
        assert(await page.locator('#view-add').isVisible());
        assert.equal(await page.locator('#f-name').inputValue(), ' New catalog ');
        assert.match(await page.locator('#dialog-errors').innerText(), /fixture transport failure/);
        await page.evaluate(() => { window.failDelivery = false; });
        await shot('04-add');
        await tap('#btn-form-submit');
        await command({ op: 'catalog_add', name: 'New catalog', url: 'https://example.test/new/opds',
            username: '', password: '', insecure: false, enabled: true });
        assert(!(await page.locator('#dialog-overlay').isVisible()));

        await tap('#catalog-source-0 [data-op="edit"]');
        await page.locator('#f-enabled').uncheck();
        await tap('#btn-form-submit');
        await command({ op: 'catalog_update', id: 'source-0', name: '<Science & stories>',
            url: 'https://example.test/0/opds', username: '', password: '', insecure: false, enabled: false });
        await tap('#catalog-source-0 [data-op="edit"]');
        await tap('#btn-form-delete');
        assert.equal(commands.length, 0, 'delete requires confirmation');
        await tap('#btn-delete-cancel');
        assert(!(await page.locator('#delete-confirm').isVisible()));
        await tap('#btn-form-delete');
        await shot('05-edit');
        await tap('#btn-delete-confirm');
        await command({ op: 'catalog_remove', id: 'source-0' });
        await tap('#btn-add');
        assert(!(await page.locator('#btn-form-delete').isVisible()));
        await page.keyboard.press('Escape');
        assert.equal(await page.evaluate(() => document.activeElement.id), 'btn-add');

        await tap('#btn-settings');
        assert.equal(await page.locator('[role="dialog"]:visible').count(), 1);
        await page.locator('#f-global-prefix').fill('Reading');
        const beforeDraft = polls;
        await expect(() => polls > beforeDraft, 'settings draft survives polling');
        assert.equal(await page.locator('#f-global-prefix').inputValue(), 'Reading');
        await shot('06-settings');
        await tap('#btn-prefix-apply');
        await command({ op: 'set_collection_prefix', prefix: 'Reading' });
        await tap('#btn-collections');
        await command({ op: 'collections_rebuild' });
        await tap('#btn-settings-close');
        assert.equal(await page.evaluate(() => document.activeElement.id), 'btn-settings');
        brokenStatus = true;
        await refresh();
        await expect(async () => (await page.locator('#error').innerText()).includes('Could not parse'), 'poll errors shown');
        brokenStatus = false;
        await refresh();
        await expect(async () => (await page.locator('#error').innerText()) === '', 'poll recovers');
        assert.deepEqual(errors, []);
        assert.deepEqual(commands, []);
        console.log('Browser E2E passed: catalogs, sync commands, polling, dialogs, CRUD, keyboard, errors and layout');
    } catch (error) {
        await shot('failure');
        throw error;
    } finally {
        await browser.close();
    }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
