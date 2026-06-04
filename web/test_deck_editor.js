#!/usr/bin/env node
/**
 * E2E test for web/deck_editor.html (the WASM-free deck builder).
 *
 * Verifies:
 *   1. Page loads without JS errors.
 *   2. Card catalog is fetched (index.json → catalog.<hash>.json) and
 *      cards appear in the list.
 *   3. Search/filter reduces the card list.
 *   4. Adding a card appears in the deck list.
 *   5. Save → localStorage (SHARED key 'mtg-forge-custom-decks', canonical
 *      {main_deck, sideboard} object form), Load-chip renders.
 *   6. Saved deck contents are correct (4-of limit reflected in counts).
 *   7. Import round-trips a .dck back to the same deck state.
 *   8. "Use in Lobby" sets mtg_lobby_deck_preselect in localStorage.
 *   9. Editing a PREMADE (?edit=<name>&source=premade) loads its cards and
 *      Save writes a COPY under a new name — the premade is never mutated
 *      (mtg-682 item 4).
 *
 * Prerequisites (wired into validate-wasm-e2e-step via make):
 *   - web/data/sets/index.json  (make wasm-export)
 *   - web/data/catalog.*.json   (produced by the same export)
 *
 * Usage: node web/test_deck_editor.js
 * (Or: cd web && node test_deck_editor.js)
 */

'use strict';

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

const PROJECT_ROOT = path.resolve(__dirname, '..');
const WEB_SRC = __dirname;

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

/** Check that the wasm-export artifacts exist before we spin up a browser. */
function checkPrerequisites() {
    const idxPath = path.join(WEB_SRC, 'data', 'sets', 'index.json');
    if (!fs.existsSync(idxPath)) {
        throw new Error(
            'Missing ' + idxPath + '. Run: make wasm-export\n' +
            'The deck editor test requires the card catalog produced by the export step.'
        );
    }
    // Verify index.json contains a card_catalog field.
    const idx = JSON.parse(fs.readFileSync(idxPath, 'utf8'));
    if (!idx.card_catalog) {
        throw new Error(
            'data/sets/index.json has no card_catalog field.\n' +
            'Re-run make wasm-export with the updated Rust build.'
        );
    }
    const catalogPath = path.join(WEB_SRC, 'data', idx.card_catalog);
    if (!fs.existsSync(catalogPath)) {
        throw new Error('Missing catalog file: ' + catalogPath);
    }
    const catalog = JSON.parse(fs.readFileSync(catalogPath, 'utf8'));
    if (!Array.isArray(catalog) || catalog.length === 0) {
        throw new Error('Card catalog is empty: ' + catalogPath);
    }
    log('Prerequisites OK: ' + catalog.length + ' cards in catalog');
    return catalog;
}

const failures = [];
function check(cond, msg) {
    if (cond) {
        log('  ✓ ' + msg);
    } else {
        log('  ✗ FAIL: ' + msg);
        failures.push(msg);
    }
}

(async () => {
    // Check prerequisites before touching the browser.
    let catalogCards;
    try {
        catalogCards = checkPrerequisites();
    } catch (err) {
        console.error('PREREQUISITE FAIL: ' + err.message);
        process.exit(1);
    }

    // Pick a random port in the 19000-19999 range to avoid conflicts with
    // other parallel validate steps.
    const HTTP_PORT = 19000 + Math.floor(Math.random() * 1000);
    log('Using HTTP port: ' + HTTP_PORT);

    let httpServer = null;
    let browser = null;

    try {
        // Start a minimal HTTP server from the web/ directory.
        httpServer = spawn('python3', ['-m', 'http.server', String(HTTP_PORT)], {
            cwd: WEB_SRC,
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        httpServer.stderr.on('data', () => {});
        // Give the server a moment to start.
        await new Promise((r) => setTimeout(r, 1200));
        log('HTTP server started');

        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
        const page = await ctx.newPage();

        const pageErrors = [];
        page.on('pageerror', (e) => {
            log('PAGE ERROR: ' + e.message);
            pageErrors.push(e.message);
        });
        page.on('console', (m) => {
            if (m.type() === 'error') log('console.error: ' + m.text());
        });

        // ── 1. Page loads ──────────────────────────────────────────────
        log('\n=== 1. Page load ===');
        const BASE = 'http://localhost:' + HTTP_PORT;
        await page.goto(BASE + '/deck_editor.html', { timeout: 20000 });
        await page.waitForLoadState('domcontentloaded');
        check(await page.title() === 'Deck Editor — DeepScry', 'page title is correct');
        check(pageErrors.length === 0, 'no JS errors on load (errors: ' + pageErrors.join('; ') + ')');

        // ── 2. Catalog loads ───────────────────────────────────────────
        log('\n=== 2. Catalog load ===');
        // Wait for the catalog to appear (card-list becomes visible).
        await page.waitForFunction(
            () => {
                const el = document.getElementById('card-list');
                return el && el.style.display !== 'none' && el.children.length > 0;
            },
            { timeout: 15000 }
        );
        const cardCount = await page.evaluate(() => document.getElementById('card-list').children.length);
        check(cardCount > 0, 'card list rendered ' + cardCount + ' items');
        check(cardCount >= Math.min(200, catalogCards.length - 1),
            'card list shows up to 200 items (got ' + cardCount + ')');

        const countText = await page.textContent('#catalog-count');
        check(/\d+ cards/.test(countText), 'catalog count shows number: "' + countText + '"');

        // ── 3. Search filter ───────────────────────────────────────────
        log('\n=== 3. Search filter ===');
        await page.fill('#search-input', 'Lightning Bolt');
        // Wait for debounce + re-render.
        await page.waitForTimeout(400);
        const afterSearch = await page.evaluate(() => document.getElementById('card-list').children.length);
        check(afterSearch < cardCount, 'search reduces card list (' + afterSearch + ' < ' + cardCount + ')');
        // Verify Lightning Bolt appears if it exists in catalog.
        const hasLB = catalogCards.some((c) => c.name === 'Lightning Bolt');
        if (hasLB) {
            const names = await page.evaluate(() =>
                [...document.querySelectorAll('#card-list li .card-name')].map((el) => el.textContent)
            );
            check(names.includes('Lightning Bolt'), 'Lightning Bolt is in filtered results');
        } else {
            log('  (Lightning Bolt not in catalog — skipping name assertion)');
        }
        // Clear search.
        await page.fill('#search-input', '');
        await page.waitForTimeout(300);

        // ── 4. Add a card ──────────────────────────────────────────────
        log('\n=== 4. Add card to deck ===');
        // Pick the first card shown in the list.
        const firstName = await page.evaluate(() => {
            const li = document.querySelector('#card-list li');
            return li ? li.querySelector('.card-name').textContent : null;
        });
        check(!!firstName, 'first card has a name: ' + firstName);
        if (firstName) {
            // Click the "Add" button of the first card.
            await page.evaluate(() => {
                const btn = document.querySelector('#card-list li .add-btn');
                if (btn && !btn.disabled) btn.click();
            });
            await page.waitForTimeout(100);
            // Deck list should now have an entry.
            const deckItems = await page.evaluate(() =>
                [...document.querySelectorAll('#deck-list li .dl-name')].map((el) => el.textContent)
            );
            check(deckItems.includes(firstName), 'added card appears in deck list');
            const mainCount = await page.textContent('#stat-main');
            check(parseInt(mainCount) >= 1, 'main deck count >= 1 after add');
        }

        // Add 4 copies (max limit for non-basic).
        for (let i = 0; i < 3; i++) {
            await page.evaluate(() => {
                const btn = document.querySelector('#card-list li .add-btn');
                if (btn && !btn.disabled) btn.click();
            });
            await page.waitForTimeout(50);
        }
        // 5th attempt should be blocked (button disabled).
        const addBtnDisabled = await page.evaluate(() => {
            const btn = document.querySelector('#card-list li .add-btn');
            return btn ? btn.disabled : false;
        });
        check(addBtnDisabled, '5th copy: add button is disabled (4-of limit enforced)');

        // ── 4b. Card Details panel (TUI parity) ────────────────────────
        log('\n=== 4b. Card Details panel ===');
        // Pick a creature with a cost + oracle so we can assert ALL detail rows
        // (name, cost, type, P/T, oracle). Fall back to the first card otherwise.
        const detailCard = catalogCards.find((c) =>
            c.power !== null && c.power !== undefined &&
            c.toughness !== null && c.toughness !== undefined &&
            c.mana_cost && c.oracle) || catalogCards[0];
        if (detailCard) {
            await page.fill('#search-input', detailCard.name);
            await page.waitForTimeout(300);
            // Click the matching catalog row (not the Add button) to show details.
            await page.evaluate((nm) => {
                const rows = [...document.querySelectorAll('#card-list li')];
                const li = rows.find((r) => r.querySelector('.card-name') &&
                    r.querySelector('.card-name').textContent === nm);
                if (li) li.click();
            }, detailCard.name);
            await page.waitForTimeout(100);
            const details = await page.evaluate(() => {
                const el = document.getElementById('card-details');
                return {
                    name: el.querySelector('.cd-name') ? el.querySelector('.cd-name').textContent : null,
                    rows: [...el.querySelectorAll('.cd-row')].map((r) => r.textContent),
                    hasCost: !!(el.querySelector('.cd-row') && el.innerHTML.includes('Cost:')),
                    hasMana: !!el.querySelector('.cd-row .ms'),
                    hasType: el.innerHTML.includes('Type:'),
                    pt: el.querySelector('.cd-pt') ? el.querySelector('.cd-pt').textContent : null,
                    oracle: el.querySelector('.cd-oracle') ? el.querySelector('.cd-oracle').textContent : null,
                    empty: !!el.querySelector('.cd-empty'),
                };
            });
            check(!details.empty, 'card-details no longer shows the empty hint after a click');
            check(details.name === detailCard.name,
                'card-details shows the clicked card name: "' + details.name + '"');
            check(details.hasType, 'card-details shows a Type line');
            if (detailCard.mana_cost) {
                check(details.hasCost && details.hasMana,
                    'card-details shows the mana Cost (rendered symbols)');
            }
            if (detailCard.power !== null && detailCard.power !== undefined) {
                check(details.pt && details.pt.includes(String(detailCard.power) + '/' + String(detailCard.toughness)),
                    'card-details shows P/T for the creature: "' + details.pt + '"');
            }
            if (detailCard.oracle) {
                check(!!details.oracle && details.oracle.length > 0,
                    'card-details shows the oracle text');
            }
            // Clear the search so later sections see the full list again.
            await page.fill('#search-input', '');
            await page.waitForTimeout(250);
        } else {
            check(false, 'catalog had no card to exercise the details panel');
        }

        // ── 5. Save deck / saved chips ──────────────────────────────────
        log('\n=== 5. Save deck ===');
        // Set deck name.
        await page.fill('#deck-name', 'TestDeck');
        await page.click('#btn-save');
        await page.waitForTimeout(200);

        const statusText = await page.textContent('#action-status');
        check(statusText.includes('Saved') || statusText.includes('TestDeck'),
            'save status message shown: "' + statusText + '"');

        // Saved chips should appear.
        const chipText = await page.evaluate(() => {
            const chips = [...document.querySelectorAll('#saved-list .saved-chip span:first-child')];
            return chips.map((el) => el.textContent);
        });
        check(chipText.includes('TestDeck'), 'saved chip "TestDeck" appears');

        // Verify localStorage was written under the SHARED key in the canonical
        // {main_deck, sideboard} object form (mtg-682) — the same shape the
        // launcher + game pages consume. (The old editor wrote a .dck STRING
        // under 'mtg_custom_decks', which nothing else could read.)
        const lsDecks = await page.evaluate((key) => {
            try { return JSON.parse(localStorage.getItem(key) || '{}'); } catch (_) { return {}; }
        }, 'mtg-forge-custom-decks');
        const savedDeck = lsDecks['TestDeck'];
        check(savedDeck && typeof savedDeck === 'object' && !Array.isArray(savedDeck),
            'localStorage contains TestDeck as an object (shared custom-deck shape)');
        check(Array.isArray(savedDeck && savedDeck.main_deck),
            'saved deck has a main_deck array');
        check(Array.isArray(savedDeck && savedDeck.sideboard),
            'saved deck has a sideboard array');

        // ── 6. Saved-deck contents ─────────────────────────────────────
        log('\n=== 6. Saved-deck contents ===');
        const mainPairs = (savedDeck && savedDeck.main_deck) || [];
        const totalCards = mainPairs.reduce((s, p) => s + (p[1] || 0), 0);
        check(totalCards > 0, 'saved deck main_deck is non-empty (cards=' + totalCards + ')');
        if (firstName) {
            const found = mainPairs.find((p) => p[0] === firstName);
            check(!!found, 'saved deck contains added card "' + firstName + '"');
            // 4-of limit: non-basic firstName should be at exactly 4.
            if (found && !firstName.toLowerCase().includes('plains') &&
                !firstName.toLowerCase().includes('island') &&
                !firstName.toLowerCase().includes('swamp') &&
                !firstName.toLowerCase().includes('mountain') &&
                !firstName.toLowerCase().includes('forest')) {
                check(found[1] === 4, '4-of limit serialized correctly (count=' + found[1] + ')');
            }
        }

        // ── 7. Import round-trip ───────────────────────────────────────
        log('\n=== 7. Import round-trip ===');
        // Build a .dck text from the saved object and import it back; the deck
        // state should repopulate. (Import parses .dck text; the saved form is
        // an object, so we serialize it to .dck here for the round-trip.)
        const dckText = '[metadata]\nName=TestDeck\n\n[Main]\n' +
            mainPairs.map((p) => p[1] + ' ' + p[0]).join('\n') + '\n';
        await page.click('#btn-import');
        await page.waitForSelector('#import-panel.open', { timeout: 3000 });
        await page.fill('#import-text', dckText);
        await page.click('#btn-import-confirm');
        await page.waitForTimeout(200);

        const afterImportMain = await page.textContent('#stat-main');
        check(parseInt(afterImportMain) >= 1, 'deck has cards after import round-trip (main=' + afterImportMain + ')');

        // ── 8. "Use in Lobby" removed + context-sensitive SAVE ─────────
        log('\n=== 8. Use-in-Lobby removed + context-sensitive SAVE ===');
        // The no-op "Use in Lobby" button is gone.
        const useBtnGone = await page.evaluate(() => !document.getElementById('btn-use-in-lobby'));
        check(useBtnGone, '#btn-use-in-lobby is removed from the deck editor');
        // Opened from the LOBBY (no ?from=launcher): SAVE just saves + stays.
        const saveLabelLobby = await page.evaluate(() => {
            const b = document.getElementById('btn-save');
            return b ? b.textContent.trim() : null;
        });
        check(saveLabelLobby === 'Save',
            'SAVE button reads "Save" when not from launcher (got "' + saveLabelLobby + '")');

        // Opened FROM the launcher (?from=launcher&...): SAVE becomes "Save and use"
        // and navigates back to the launcher (index.html?goto=launcher&...&deck=).
        const launcherCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
        const lp = await launcherCtx.newPage();
        lp.on('pageerror', (e) => check(false, 'from-launcher page JS error: ' + e.message));
        const fromLauncherUrl = BASE + '/deck_editor.html?from=launcher&game=ed-game&name=ed-user&ws=ws://x';
        await lp.goto(fromLauncherUrl, { timeout: 20000 });
        await lp.waitForLoadState('domcontentloaded');
        await lp.waitForFunction(
            () => { const el = document.getElementById('card-list'); return el && el.children.length > 0; },
            { timeout: 15000 },
        ).catch(() => {});
        const saveLabelLauncher = await lp.evaluate(() => {
            const b = document.getElementById('btn-save');
            return b ? b.textContent.trim() : null;
        });
        check(saveLabelLauncher === 'Save and use',
            'SAVE button reads "Save and use" when from launcher (got "' + saveLabelLauncher + '")');

        // Add a card + name the deck, then click "Save and use". It first
        // navigates to index.html?goto=launcher&…&deck=LauncherDeck (the
        // launcherReturnUrl), whose dispatcher then forwards to launcher.html.
        // Asserting the SETTLED url is racy (the dispatcher redirect is near-
        // instant), so we record the whole nav chain and assert the
        // index.html?goto=launcher hop was emitted — with a backstop that we end
        // up back at the launcher with the deck preselected.
        const navs = [];
        lp.on('framenavigated', (fr) => { if (fr === lp.mainFrame()) navs.push(fr.url()); });
        await lp.evaluate(() => {
            const btn = document.querySelector('#card-list li .add-btn');
            if (btn && !btn.disabled) btn.click();
        });
        await lp.fill('#deck-name', 'LauncherDeck');
        await lp.click('#btn-save');
        await lp.waitForTimeout(1000); // let the dispatcher redirect settle
        const sawReturnHop = navs.some((u) =>
            /index\.html\?/.test(u) && /goto=launcher/.test(u) && /deck=LauncherDeck/.test(u));
        const settledAtLauncher = /launcher\.html/.test(lp.url()) && /deck=LauncherDeck/.test(lp.url());
        check(sawReturnHop || settledAtLauncher,
            '"Save and use" navigates back to the launcher with the deck preselected ' +
            '(index.html?goto=launcher&…&deck=LauncherDeck → launcher.html); navs=' + JSON.stringify(navs));
        await launcherCtx.close();

        // ── 9. Edit a PREMADE saves a COPY (mtg-682) ───────────────────
        log('\n=== 9. Edit premade → save-as-copy (premade never mutated) ===');
        // Pick the first premade name from the served index.json deck_contents.
        const idxForEdit = await page.evaluate(async (base) => {
            try {
                const r = await fetch(base + '/data/sets/index.json', { cache: 'no-store' });
                const j = await r.json();
                const names = j.deck_contents ? Object.keys(j.deck_contents) : [];
                return { name: names[0] || null, contents: names[0] ? j.deck_contents[names[0]] : null };
            } catch (e) { return { name: null, contents: null, error: String(e) }; }
        }, BASE);

        if (!idxForEdit.name) {
            check(false, 'index.json deck_contents has at least one premade to edit (got none)');
        } else {
            const premade = idxForEdit.name;
            log('  editing premade: ' + premade);
            const editCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
            const editPage = await editCtx.newPage();
            editPage.on('pageerror', (e) => { check(false, 'edit page JS error: ' + e.message); });
            await editPage.goto(
                BASE + '/deck_editor.html?edit=' + encodeURIComponent(premade) + '&source=premade',
                { timeout: 20000 });
            await editPage.waitForLoadState('domcontentloaded');
            // The editor should load the premade's card list and set the name field.
            await editPage.waitForFunction(
                (n) => document.getElementById('deck-name') &&
                       document.getElementById('deck-name').value === n,
                premade, { timeout: 15000 },
            ).catch(() => {});
            const loadedName = await editPage.$eval('#deck-name', (el) => el.value).catch(() => '');
            check(loadedName === premade, 'editor loaded premade name "' + premade + '" (got "' + loadedName + '")');
            const mainStat = parseInt(await editPage.textContent('#stat-main').catch(() => '0'), 10);
            check(mainStat > 0, 'editor populated the premade card list (main=' + mainStat + ')');

            // Save → must write a COPY under a NEW name, leaving the premade absent
            // from the custom-decks store (premades are not custom decks).
            await editPage.click('#btn-save');
            await editPage.waitForTimeout(250);
            const afterSave = await editPage.evaluate((key) => {
                try { return JSON.parse(localStorage.getItem(key) || '{}'); } catch (_) { return {}; }
            }, 'mtg-forge-custom-decks');
            const customNames = Object.keys(afterSave);
            const copyName = customNames.find((n) => n !== 'TestDeck' && n.startsWith(premade));
            check(!!copyName, 'Save wrote a COPY (custom deck named like the premade): ' + JSON.stringify(customNames));
            check(copyName !== premade, 'the saved copy name differs from the premade name (no in-place mutation)');
            check(!afterSave[premade] || copyName !== premade,
                'premade itself is NOT stored as a custom deck under its original name');
            if (copyName) {
                const copy = afterSave[copyName];
                check(copy && Array.isArray(copy.main_deck) && copy.main_deck.length > 0,
                    'the saved copy is in canonical {main_deck,...} form with cards');
            }
            await editCtx.close();
        }

        // ── Summary ────────────────────────────────────────────────────
        await browser.close();
        browser = null;

        log('\n=== Test Summary ===');
        if (failures.length === 0) {
            log('✓ All deck editor checks passed');
        } else {
            log('✗ ' + failures.length + ' failure(s):');
            for (const f of failures) log('    - ' + f);
            process.exit(1);
        }
    } finally {
        if (browser) await browser.close().catch(() => {});
        if (httpServer && !httpServer.killed) {
            try { process.kill(httpServer.pid, 'SIGTERM'); } catch (_) {}
        }
    }
})();
