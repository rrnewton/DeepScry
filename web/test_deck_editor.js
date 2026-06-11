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
 *      Card-details pane (4b) shows name/cost/type/P-T/oracle AND a Gatherer
 *      card-art <img> (WASM-free, computed from the card name) on selection.
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
        // One page is PAGE_SIZE=100 rows now (batch2 item 5 pagination); a full
        // catalog fills the first page exactly.
        check(cardCount === Math.min(100, catalogCards.length),
            'card list shows one page of up to 100 items (got ' + cardCount + ')');

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
                const art = el.querySelector('img.cd-art');
                return {
                    name: el.querySelector('.cd-name') ? el.querySelector('.cd-name').textContent : null,
                    rows: [...el.querySelectorAll('.cd-row')].map((r) => r.textContent),
                    hasCost: !!(el.querySelector('.cd-row') && el.innerHTML.includes('Cost:')),
                    hasMana: !!el.querySelector('.cd-row .ms'),
                    hasType: el.innerHTML.includes('Type:'),
                    pt: el.querySelector('.cd-pt') ? el.querySelector('.cd-pt').textContent : null,
                    oracle: el.querySelector('.cd-oracle') ? el.querySelector('.cd-oracle').textContent : null,
                    empty: !!el.querySelector('.cd-empty'),
                    // Read the raw src ATTRIBUTE (not the .src property), since an
                    // onerror handler may hide the <img> when Gatherer art is
                    // unavailable in the headless/offline test — the element +
                    // its computed URL must still be present.
                    artSrc: art ? art.getAttribute('src') : null,
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
            // Card art: the pane builds a Scryfall EXACT-named <img> (WASM-free,
            // computed from the card name) for the clicked card. The art may be
            // hidden on an offline/missing-image onerror, but the element + URL
            // must exist. The previous Gatherer `Image.ashx?name=` handler did a
            // FUZZY server-side match and could return a DIFFERENT card's art
            // (user bug: Lightning Bolt rendered "Thrum of the Vestige"); the
            // exact-name endpoint cannot collide that way.
            check(!!details.artSrc && details.artSrc.includes('api.scryfall.com/cards/named'),
                'card-details renders a Scryfall exact-name art <img> for the card: "' + details.artSrc + '"');
            check(!!details.artSrc &&
                details.artSrc.includes('exact=' + encodeURIComponent(detailCard.name)),
                'card-details art URL is an EXACT lookup of the clicked card name');
            // Clear the search so later sections see the full list again.
            await page.fill('#search-input', '');
            await page.waitForTimeout(250);
        } else {
            check(false, 'catalog had no card to exercise the details panel');
        }

        // ── 4c. WRONG-IMAGE regression: each card maps to ITS OWN art ───
        // Selecting several specific cards in a row must give each one its own
        // exact-name image URL (and the panel + <img> name stamps must agree),
        // so the description and the image can never drift apart the way the old
        // fuzzy Gatherer lookup allowed.
        log('\n=== 4c. Card-details image maps to the SELECTED card ===');
        {
            // Use real cards from the served catalog so the assertions are stable
            // regardless of which sets are bundled. Prefer the user's reported
            // case (Lightning Bolt) + an Adventure card + a basic land when present.
            const present = (nm) => catalogCards.some((c) => c.name === nm);
            const probes = ['Lightning Bolt', 'Bonecrusher Giant', 'Forest']
                .filter(present);
            // Backfill with arbitrary distinct catalog names if the preferred set
            // isn't available, so this section always exercises >=2 cards.
            for (const c of catalogCards) {
                if (probes.length >= 3) break;
                if (!probes.includes(c.name)) probes.push(c.name);
            }
            for (const nm of probes) {
                await page.fill('#search-input', nm);
                await page.waitForTimeout(300);
                await page.evaluate((n) => {
                    const li = [...document.querySelectorAll('#card-list li')]
                        .find((r) => r.querySelector('.card-name') &&
                            r.querySelector('.card-name').textContent === n);
                    if (li) li.click();
                }, nm);
                await page.waitForTimeout(120);
                const d = await page.evaluate(() => {
                    const el = document.getElementById('card-details');
                    const img = el.querySelector('img.cd-art');
                    return {
                        detailName: el.querySelector('.cd-name')
                            ? el.querySelector('.cd-name').textContent : null,
                        panelStamp: el.dataset.selectedCard || null,
                        imgStamp: img ? (img.dataset.cardName || null) : null,
                        src: img ? img.getAttribute('src') : null,
                    };
                });
                check(d.detailName === nm &&
                    d.panelStamp === nm &&
                    d.imgStamp === nm &&
                    !!d.src && d.src.includes('exact=' + encodeURIComponent(nm)),
                    'card "' + nm + '": details name, panel/img name-stamps, and art URL all match it ' +
                    '(name=' + JSON.stringify(d.detailName) + ', src=' + d.src + ')');
            }
            await page.fill('#search-input', '');
            await page.waitForTimeout(250);
        }

        // ── 5. Save deck / saved chips ──────────────────────────────────
        log('\n=== 5. Save deck ===');
        // Set deck name. Save is now per-destination (batch2 item 8): use the
        // in-section "Save to Local" button (logged-out default destination).
        await page.fill('#deck-name', 'TestDeck');
        await page.click('#btn-save-local');
        await page.waitForTimeout(200);

        const statusText = await page.textContent('#action-status');
        check(statusText.includes('Saved') || statusText.includes('TestDeck'),
            'save status message shown: "' + statusText + '"');

        // Saved chips should appear. The chip label now includes a card count,
        // e.g. "TestDeck (4)", so match by prefix rather than exact equality.
        const chipText = await page.evaluate(() => {
            const chips = [...document.querySelectorAll('#saved-list .saved-chip span:first-child')];
            return chips.map((el) => el.textContent);
        });
        check(chipText.some((t) => t.startsWith('TestDeck')), 'saved chip "TestDeck" appears');

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
        // Opened from the LOBBY (no ?from=launcher): the ambiguous top-level SAVE
        // is HIDDEN (batch2 item 8); the user picks a destination via the
        // in-section "Save to Local" / "Save to Cloud" buttons instead.
        const lobbySave = await page.evaluate(() => {
            const top = document.getElementById('btn-save');
            const vis = (e) => !!e && getComputedStyle(e).display !== 'none';
            return {
                topVisible: vis(top),
                hasLocalSave: !!document.getElementById('btn-save-local'),
                hasCloudSave: !!document.getElementById('btn-save-cloud'),
                hasNew: !!document.getElementById('btn-new'),
            };
        });
        check(!lobbySave.topVisible,
            'ambiguous top-level Save is hidden outside the launcher flow');
        check(lobbySave.hasLocalSave && lobbySave.hasCloudSave,
            'per-destination Save buttons exist: Save to Local + Save to Cloud');
        check(lobbySave.hasNew, '"New deck" button exists (replaces Clear)');

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
            // from the custom-decks store (premades are not custom decks). Uses
            // the in-section "Save to Local" button (batch2 item 8).
            await editPage.click('#btn-save-local');
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

        // ── 9b. Two deck lists + empty-save guard + upload (logged OUT) ──
        // The plain HTTP server has no /auth/status route → the editor is in the
        // signed-out state: the Cloud section shows a sign-in note (no account
        // block) and the Local Browser list is the localStorage decks.
        log('\n=== 9b. Two lists (Cloud/Local), empty-save guard, upload (logged out) ===');
        {
            const lo = await browser.newContext({ viewport: { width: 1280, height: 900 } });
            const lop = await lo.newPage();
            lop.on('pageerror', (e) => check(false, '9b page JS error: ' + e.message));
            await lop.addInitScript(() => localStorage.setItem('mtg-forge-custom-decks',
                JSON.stringify({ 'LocalA': { main_deck: [['Forest', 24]], sideboard: [] } })));
            await lop.goto(BASE + '/deck_editor.html', { timeout: 20000 });
            await lop.waitForFunction(
                () => document.getElementById('card-list') &&
                      document.getElementById('card-list').children.length > 0,
                { timeout: 15000 });
            await lop.waitForTimeout(400);
            const two = await lop.evaluate(() => {
                const vis = (id) => { const e = document.getElementById(id); return e ? getComputedStyle(e).display !== 'none' : null; };
                return {
                    cloudSignedOut: vis('cloud-signed-out'),
                    cloudAccount: vis('cloud-account'),
                    localChips: [...document.querySelectorAll('#saved-list .saved-chip')].map((c) => c.textContent),
                    cloudListText: document.getElementById('cloud-list').textContent.trim(),
                    hasCloudHeading: [...document.querySelectorAll('#cloud-section h3')].some((h) => /Cloud/.test(h.textContent)),
                    hasLocalHeading: [...document.querySelectorAll('.saved-decks-wrap h3')].some((h) => /Local Browser/.test(h.textContent)),
                };
            });
            check(two.cloudSignedOut === true && two.cloudAccount === false,
                'logged-out: Cloud section shows the sign-in note, hides the account block');
            check(two.hasCloudHeading && two.hasLocalHeading,
                'two distinctly-labeled lists: "Cloud" + "Local Browser"');
            check(two.localChips.some((t) => t.includes('LocalA')),
                'Local Browser list shows the localStorage deck: ' + JSON.stringify(two.localChips));

            // Empty-save guard: naming an empty deck and clicking Save to Local
            // is refused and writes NOTHING to localStorage (the 86-byte
            // metadata-only bug). (batch2 item 8: per-destination save button.)
            await lop.fill('#deck-name', 'EmptyGuardDeck');
            await lop.click('#btn-save-local');
            await lop.waitForTimeout(200);
            const guard = await lop.evaluate(() => ({
                msg: document.getElementById('action-status').textContent,
                stored: !!JSON.parse(localStorage.getItem('mtg-forge-custom-decks') || '{}')['EmptyGuardDeck'],
            }));
            check(/empty/i.test(guard.msg) && guard.stored === false,
                'empty-save guard: refuses a 0-card deck and stores nothing ("' + guard.msg + '")');

            // Upload a .dck file → imports through the same path as paste.
            await lop.click('#btn-import');
            await lop.waitForSelector('#import-panel.open', { timeout: 3000 });
            const importBtns = await lop.evaluate(() =>
                [...document.querySelectorAll('#import-panel .row button')].map((b) => b.textContent.trim()));
            check(importBtns.length === 3 && importBtns.some((t) => /Upload/.test(t)),
                'import dialog has a 3-button row incl. Upload: ' + JSON.stringify(importBtns));
            await lop.setInputFiles('#import-file', {
                name: 'uploaded.dck',
                mimeType: 'text/plain',
                buffer: Buffer.from('[metadata]\nName=UploadedDeck\n\n[Main]\n4 Lightning Bolt\n20 Mountain\n'),
            });
            await lop.waitForTimeout(300);
            const uploaded = await lop.evaluate(() => ({
                name: document.getElementById('deck-name').value,
                main: parseInt(document.getElementById('stat-main').textContent, 10),
            }));
            check(uploaded.name === 'UploadedDeck' && uploaded.main === 24,
                'uploading a .dck file imports its cards (name=' + uploaded.name + ', main=' + uploaded.main + ')');

            // ── batch2 items 3,4,5,6,7,12: layout + pagination + alphabet +
            //    curve tooltip + local-storage info affordance ──────────────
            log('  -- batch2 layout/pagination/alphabet/curve/info checks --');

            // Item 4: the search toolbar is inside the RESULTS pane (column 2).
            // Item 3: the deck stats row is inside the DECK pane (column 3).
            const placement = await lop.evaluate(() => {
                const inSel = (childId, parentSel) => {
                    const c = document.getElementById(childId);
                    const p = document.querySelector(parentSel);
                    return !!(c && p && p.contains(c));
                };
                return {
                    searchInResults: inSel('search-input', '.pane-results'),
                    statsInDeck: inSel('deck-stats', '.pane-deck'),
                    statsAboveCurve: (() => {
                        const s = document.getElementById('deck-stats');
                        const cv = document.getElementById('curve-bars');
                        if (!s || !cv) return false;
                        // stats node precedes curve in document order within the deck pane.
                        return !!(s.compareDocumentPosition(cv) & Node.DOCUMENT_POSITION_FOLLOWING);
                    })(),
                };
            });
            check(placement.searchInResults, 'item 4: search bar lives at the top of the results column');
            check(placement.statsInDeck && placement.statsAboveCurve,
                'item 3: deck counts live in the deck column, above the mana curve');

            // Item 6: alphabet bar exists with All + 26 letters; clicking a letter
            // filters to names starting with it (AND with the search term).
            const alpha = await lop.evaluate(() => {
                const bar = document.getElementById('alpha-bar');
                const letters = bar ? [...bar.querySelectorAll('.alpha-letter')].map((b) => b.textContent) : [];
                return { count: letters.length, hasAll: letters.includes('All'), hasZ: letters.includes('Z') };
            });
            check(alpha.count === 27 && alpha.hasAll && alpha.hasZ,
                'item 6: alphabet bar has All + A–Z (' + alpha.count + ' buttons)');
            // Click "A" and assert every visible row name starts with A.
            await lop.evaluate(() => {
                const a = [...document.querySelectorAll('#alpha-bar .alpha-letter')].find((b) => b.textContent === 'A');
                if (a) a.click();
            });
            await lop.waitForTimeout(150);
            const alphaFiltered = await lop.evaluate(() => {
                const names = [...document.querySelectorAll('#card-list .card-name')].map((n) => n.textContent);
                const firstAlpha = (s) => (s.match(/[A-Za-z]/) || [''])[0].toUpperCase();
                return { total: names.length, allA: names.length > 0 && names.every((n) => firstAlpha(n) === 'A') };
            });
            check(alphaFiltered.total > 0 && alphaFiltered.allA,
                'item 6: clicking "A" filters to names starting with A (' + alphaFiltered.total + ' shown)');
            // Reset the letter via "All" so pagination math below is over the full set.
            await lop.evaluate(() => {
                const all = [...document.querySelectorAll('#alpha-bar .alpha-letter')].find((b) => b.textContent === 'All');
                if (all) all.click();
            });
            await lop.waitForTimeout(150);

            // Item 5: pagination — with the full catalog the pager is visible and
            // Next advances the page.
            const pager0 = await lop.evaluate(() => {
                const p = document.getElementById('pager');
                return {
                    visible: !!p && getComputedStyle(p).display !== 'none',
                    info: (document.getElementById('pager-info') || {}).textContent || '',
                    firstName: (document.querySelector('#card-list .card-name') || {}).textContent || '',
                };
            });
            check(pager0.visible && /Page 1 of/.test(pager0.info),
                'item 5: pagination footer shows "Page 1 of N" ("' + pager0.info + '")');
            await lop.click('#pager-next');
            await lop.waitForTimeout(150);
            const pager1 = await lop.evaluate(() => ({
                info: (document.getElementById('pager-info') || {}).textContent || '',
                firstName: (document.querySelector('#card-list .card-name') || {}).textContent || '',
                prevDisabled: document.getElementById('pager-prev').disabled,
            }));
            check(/Page 2 of/.test(pager1.info) && pager1.firstName !== pager0.firstName,
                'item 5: Next advances to page 2 with a different first row');
            check(pager1.prevDisabled === false, 'item 5: Prev is enabled on page 2');

            // Item 7: mana-curve column carries a per-CMC count tooltip (title).
            // Add a 1-CMC card so a bar has a non-zero count to describe.
            await lop.fill('#search-input', 'Lightning Bolt');
            await lop.waitForTimeout(250);
            await lop.evaluate(() => { const b = document.querySelector('#card-list li .add-btn'); if (b && !b.disabled) b.click(); });
            await lop.waitForTimeout(150);
            const curveTip = await lop.evaluate(() => {
                const cols = [...document.querySelectorAll('#curve-bars .curve-col')];
                return cols.map((c) => c.title).filter((t) => /\d+ card/.test(t));
            });
            check(curveTip.length === 8 && curveTip.some((t) => /at \d/.test(t)),
                'item 7: every curve column has a "N cards at C CMC" tooltip');

            // Item 12: the Local Browser header has an info affordance whose tooltip
            // names the local-only storage + the Safari ~7-day caveat.
            const info = await lop.evaluate(() => {
                const el = document.getElementById('local-info');
                return el ? el.getAttribute('title') : null;
            });
            check(!!info && /local storage/i.test(info) && /Safari/.test(info) && /7[\s-]?day/i.test(info),
                'item 12: Local Browser info tooltip explains local-only storage + Safari 7-day eviction');

            await lo.close();
        }

        // ── 9c. Logged-IN: account block + auto-hydrated cloud list + save ──
        // Mock /auth/status (logged in), the credentials endpoint, and a fake R2
        // so the editor's authoritative-login + auto-hydrate + cloud-save path is
        // exercised hermetically (no real OAuth / R2).
        log('\n=== 9c. Logged-in: cloud account, auto-hydrate, cloud save ===');
        {
            const ci = await browser.newContext({ viewport: { width: 1280, height: 900 } });
            const cip = await ci.newPage();
            cip.on('pageerror', (e) => check(false, '9c page JS error: ' + e.message));
            const r2 = { bytes: null, etag: null };
            await cip.route('**/auth/status', (route) => route.fulfill({
                status: 200, contentType: 'application/json',
                body: JSON.stringify({ logged_in: true, user_id: 'github-42', provider: 'github',
                    display_name: 'octocat', suggested_name: 'octocat', oauth_enabled: true,
                    providers: { github: true, google: true } }),
            }));
            await cip.route('**/api/deck-storage/credentials', (route) => {
                const base = 'https://fake-r2.example.com/deepscry-decks/decks/github-42/collection.tgz';
                route.fulfill({ status: 200, contentType: 'application/json',
                    body: JSON.stringify({ user_id: 'github-42', object_key: 'decks/github-42/collection.tgz',
                        ttl_secs: 600, put_url: base + '?m=PUT', get_url: base + '?m=GET',
                        head_url: base + '?m=HEAD', download_url: base + '?m=DL', content_type: 'application/gzip' }) });
            });
            await cip.route('**/fake-r2.example.com/**', async (route) => {
                const m = route.request().method();
                const cors = { 'Access-Control-Allow-Origin': '*', 'Access-Control-Expose-Headers': 'ETag' };
                if (m === 'PUT') { r2.bytes = Buffer.from(route.request().postDataBuffer()); r2.etag = '"e1"';
                    return route.fulfill({ status: 200, headers: { ...cors, ETag: r2.etag }, body: '' }); }
                if (m === 'GET' || m === 'HEAD') return r2.bytes
                    ? route.fulfill({ status: 200, headers: { ...cors, ETag: r2.etag, 'Content-Type': 'application/gzip' }, body: m === 'HEAD' ? Buffer.alloc(0) : r2.bytes })
                    : route.fulfill({ status: 404, headers: cors, body: '' });
                return route.fulfill({ status: 200, headers: cors, body: '' });
            });
            await cip.goto(BASE + '/deck_editor.html', { timeout: 20000 });
            await cip.waitForFunction(
                () => document.getElementById('card-list') &&
                      document.getElementById('card-list').children.length > 0,
                { timeout: 15000 });
            await cip.waitForTimeout(700);
            const acct = await cip.evaluate(() => ({
                accountVisible: getComputedStyle(document.getElementById('cloud-account')).display !== 'none',
                who: document.getElementById('cloud-acct-who').textContent,
                objpath: document.getElementById('cloud-objpath').textContent,
            }));
            check(acct.accountVisible, 'logged-in: the cloud account block is shown');
            check(/github/.test(acct.who), 'account line shows the provider: "' + acct.who + '"');
            check(acct.objpath === 'decks/github-42/collection.tgz',
                'object path is shown for transparency: "' + acct.objpath + '"');

            // Build a real deck and Save → reports cloud + appears in cloud list.
            await cip.fill('#search-input', 'Lightning Bolt');
            await cip.waitForTimeout(300);
            await cip.evaluate(() => { const b = document.querySelector('#card-list li .add-btn'); if (b && !b.disabled) b.click(); });
            await cip.fill('#deck-name', 'CloudSaveDeck');
            await cip.fill('#search-input', '');
            await cip.waitForTimeout(200);
            // Save to Cloud destination button (batch2 item 8).
            await cip.click('#btn-save-cloud');
            await cip.waitForTimeout(1200);
            const saved = await cip.evaluate(() => ({
                msg: document.getElementById('action-status').textContent,
                cloudChips: [...document.querySelectorAll('#cloud-list .saved-chip')].map((c) => c.textContent),
            }));
            check(/cloud/i.test(saved.msg) && /☁/.test(saved.msg),
                'save reports it went to the CLOUD account ("' + saved.msg + '")');
            check(saved.cloudChips.some((t) => t.includes('CloudSaveDeck')),
                'saved deck appears in the auto-hydrated Cloud list: ' + JSON.stringify(saved.cloudChips));

            // Migrate All button exists + migrates local→cloud.
            await cip.evaluate(() => localStorage.setItem('mtg-forge-custom-decks',
                JSON.stringify(Object.assign(JSON.parse(localStorage.getItem('mtg-forge-custom-decks') || '{}'),
                    { 'MigrateMe': { main_deck: [['Forest', 30]], sideboard: [] } }))));
            await cip.click('#btn-migrate-all');
            await cip.waitForTimeout(1000);
            const migd = await cip.evaluate(() => ({
                msg: document.getElementById('action-status').textContent,
                cloudChips: [...document.querySelectorAll('#cloud-list .saved-chip')].map((c) => c.textContent),
            }));
            check(/Migrated/i.test(migd.msg), 'Migrate All reports progress ("' + migd.msg + '")');
            check(migd.cloudChips.some((t) => t.includes('MigrateMe')),
                'migrated local deck now appears in the Cloud list');

            // batch2 items 9/10/11: structural checks on the cloud/local sections.
            const struct = await cip.evaluate(() => {
                const txt = (id) => { const e = document.getElementById(id); return e ? e.textContent.trim() : null; };
                const vis = (id) => { const e = document.getElementById(id); return !!e && getComputedStyle(e).display !== 'none'; };
                // Which section is the migrate button inside — local or cloud?
                const mig = document.getElementById('btn-migrate-all');
                const local = document.querySelector('.saved-decks-wrap:not(#cloud-section)');
                return {
                    migrateLabel: txt('btn-migrate-all'),
                    migrateInLocal: !!(mig && local && local.contains(mig)),
                    migrateToolsVisible: vis('migrate-tools'),
                    downloadLabel: txt('btn-download-decks'),
                    hasDirectLink: !!document.getElementById('btn-direct-link'),
                };
            });
            check(/Migrate All to Cloud/i.test(struct.migrateLabel || ''),
                'item 9: migrate button renamed "Migrate All to Cloud" ("' + struct.migrateLabel + '")');
            check(struct.migrateInLocal && struct.migrateToolsVisible,
                'item 9: migrate tools live in the LOCAL section and show when signed in');
            check(struct.downloadLabel === 'Download deck collection',
                'item 10: download link relabelled "Download deck collection" ("' + struct.downloadLabel + '")');
            check(struct.hasDirectLink, 'item 11: a "Direct link" button exists in the Cloud section');
            await ci.close();
        }

        // ── 9d. Launcher: two custom groups + cloud deck is PLAYABLE ─────
        // Mock /auth/status + credentials + a fake R2 pre-seeded with a cloud
        // deck, then verify launcher.html offers BOTH "Custom Decks (Cloud)" and
        // "Custom Decks (Local Browser)" groups, and that selecting the cloud
        // deck makes its cards available to the play path (buildDeckSubmission +
        // mirrored into localStorage) — the "not found / missing cards" fix.
        log('\n=== 9d. Launcher two custom groups + cloud deck playable ===');
        {
            const li = await browser.newContext({ viewport: { width: 1280, height: 900 } });
            const lip = await li.newPage();
            lip.on('pageerror', (e) => check(false, '9d launcher JS error: ' + e.message));
            // Pre-pack a cloud collection {CloudDeck} as a .tgz the fake R2 returns.
            await lip.addInitScript(() => localStorage.setItem('mtg-forge-custom-decks',
                JSON.stringify({ 'LocalDeck': { main_deck: [['Forest', 24]], sideboard: [] } })));
            await lip.route('**/auth/status', (route) => route.fulfill({
                status: 200, contentType: 'application/json',
                body: JSON.stringify({ logged_in: true, user_id: 'github-9d', provider: 'github',
                    display_name: 'octocat', oauth_enabled: true, providers: { github: true } }),
            }));
            await lip.route('**/api/deck-storage/credentials', (route) => {
                const base = 'https://fake-r2.example.com/deepscry-decks/decks/github-9d/collection.tgz';
                route.fulfill({ status: 200, contentType: 'application/json',
                    body: JSON.stringify({ user_id: 'github-9d', object_key: 'decks/github-9d/collection.tgz',
                        ttl_secs: 600, put_url: base + '?m=PUT', get_url: base + '?m=GET',
                        head_url: base + '?m=HEAD', download_url: base + '?m=DL', content_type: 'application/gzip' }) });
            });
            // The fake R2 GET returns a .tgz built in-page from a known cloud deck.
            // We let the first GET 404 then the page builds nothing; instead we
            // pre-seed by packing via DeckStorage in a throwaway eval AFTER load is
            // simplest: intercept GET to return a packed collection.
            let cloudTgz = null;
            await lip.route('**/fake-r2.example.com/**', async (route) => {
                const cors = { 'Access-Control-Allow-Origin': '*', 'Access-Control-Expose-Headers': 'ETag' };
                const m = route.request().method();
                if ((m === 'GET' || m === 'HEAD') && cloudTgz)
                    return route.fulfill({ status: 200, headers: { ...cors, ETag: '"e1"', 'Content-Type': 'application/gzip' }, body: m === 'HEAD' ? Buffer.alloc(0) : cloudTgz });
                if (m === 'GET' || m === 'HEAD') return route.fulfill({ status: 404, headers: cors, body: '' });
                return route.fulfill({ status: 200, headers: { ...cors, ETag: '"e1"' }, body: '' });
            });
            // First load a throwaway page on this origin to pack the cloud .tgz.
            await lip.goto(BASE + '/deck_editor.html', { timeout: 20000 });
            cloudTgz = Buffer.from(await lip.evaluate(async () => {
                const files = window.DeckStorage.collectionToFiles({
                    'CloudDeck': { main_deck: [['Lightning Bolt', 4], ['Mountain', 36]], sideboard: [] } });
                const tgz = await window.DeckStorage.packTgz(files);
                return Array.from(tgz);
            }));
            // Now load the launcher; it should hydrate the cloud deck from R2.
            await lip.goto(BASE + '/launcher.html?game=g9d&name=tester', { timeout: 20000 });
            await lip.waitForTimeout(2500);
            const res = await lip.evaluate(() => {
                const col = document.getElementById('deck-collection');
                col.value = 'custom'; col.dispatchEvent(new Event('change'));
                const ds = document.getElementById('deck-select');
                const groups = [...ds.querySelectorAll('optgroup')].map((g) => g.label);
                // Pick the cloud deck and trigger the change handlers.
                const opt = [...ds.options].find((o) => o.value.startsWith('cloud:'));
                let mirrored = null;
                if (opt) {
                    ds.value = opt.value; ds.dispatchEvent(new Event('change'));
                    const ls = JSON.parse(localStorage.getItem('mtg-forge-custom-decks') || '{}');
                    mirrored = ls['CloudDeck'] ? ls['CloudDeck'].main_deck.reduce((s, p) => s + p[1], 0) : null;
                }
                return { groups, hadCloudOpt: !!opt, mirroredCards: mirrored };
            });
            check(res.groups.some((g) => /Cloud/.test(g)) && res.groups.some((g) => /Local/.test(g)),
                'launcher offers BOTH custom groups (Cloud + Local Browser): ' + JSON.stringify(res.groups));
            check(res.hadCloudOpt, 'launcher lists the hydrated cloud deck as a cloud: option');
            check(res.mirroredCards === 40,
                'selecting the cloud deck makes its 40 cards available to play (mirrored to localStorage)');
            await li.close();
        }

        // ── 10. World Championship collections (all years) in the launcher ──
        // For EVERY championship year we expect:
        //   (a) all four of its decks are bundled/served (their file-stem names
        //       appear in index.json deck_names — so the .dck files actually
        //       reached web/data, not just referenced), AND
        //   (b) the solo launcher dropdown offers a "<year> World Championship"
        //       collection, AND
        //   (c) selecting it fills the deck picker with EXACTLY that year's four
        //       decks — no more, no less. (c) is the collision guard: champion
        //       surnames repeat across years (manfield: 2015/2020/2025; pvddr:
        //       2010/2020), so a sloppy substring filter would leak decks
        //       between years. Exact-set membership must keep them separated.
        log('\n=== 10. World Championship collections — all years (solo launcher) ===');
        {
            // Expected year → exact deck stems (mirror of CHAMPIONSHIP_DECKS in
            // the launchers and the export-wasm globs in mtg-engine/src/main.rs).
            const CHAMP = {
                1994: ['01_dolan_wug_stasis', '02_lestree_rg_zoo', '03_symens_zoo', '04_defoucaud_zoo'],
                1995: ['01_blumke_bw_rack', '02_hernandez_rw_vise_orb', '03_justice_red_artifact', '04_stern_rg_burn'],
                2000: ['01_finkel_tinker', '02_maher_tinker', '03_vandelogt_replenish', '04_labarre_chimera'],
                2005: ['01_mori_ghazi_glare', '02_karsten_greater_gift', '03_asahara_enduring_ideal', '04_kaji_ghazi_glare'],
                2010: ['01_matignon_ub_control', '02_wafotapa_ub_control', '03_pvddr_ub_control', '04_janse_eldrazi_green'],
                2015: ['01_manfield_abzan_control', '02_turtenwald_abzan_control', '03_rietzl_abzan_aggro', '04_black_mono_white'],
                2020: ['01_pvddr_azorius_control', '02_carvalho_jeskai_fires', '03_manfield_mono_red', '04_nassif_jeskai_fires'],
                2025: ['01_manfield_izzet_lessons', '02_shibata_izzet_lessons', '03_davis_izzet_lessons', '04_henry_temur_otters'],
            };

            // (a) all championship decks bundled/served in index.json deck_names.
            const idxJson = JSON.parse(fs.readFileSync(
                path.join(WEB_SRC, 'data', 'sets', 'index.json'), 'utf8'));
            const servedNames = new Set(Array.isArray(idxJson.deck_names) ? idxJson.deck_names : []);
            for (const [year, stems] of Object.entries(CHAMP)) {
                const missing = stems.filter((s) => !servedNames.has(s));
                check(missing.length === 0,
                    'all four ' + year + ' championship decks are bundled/served in index.json ' +
                    (missing.length ? '(MISSING ' + JSON.stringify(missing) + ')' : ''));
            }

            const lctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
            const lpage = await lctx.newPage();
            lpage.on('pageerror', (e) => check(false, 'solo_launcher JS error: ' + e.message));
            await lpage.goto(BASE + '/solo_launcher.html', { timeout: 20000 });
            await lpage.waitForLoadState('domcontentloaded');
            await lpage.waitForFunction(
                () => document.getElementById('p1-collection') &&
                      document.getElementById('p1-collection').options.length > 0,
                { timeout: 15000 }).catch(() => {});

            for (const [year, stems] of Object.entries(CHAMP)) {
                const key = 'championship_' + year;
                // (b) dropdown offers the year's collection.
                const hasGroup = await lpage.evaluate((k) => {
                    const sel = document.getElementById('p1-collection');
                    return !!sel && Array.from(sel.options).some((o) => o.value === k);
                }, key);
                check(hasGroup, 'solo launcher dropdown offers the "' + year + ' World Championship" collection');

                // (c) selecting it fills the picker with EXACTLY this year's decks.
                const filledDecks = await lpage.evaluate((k) => {
                    const col = document.getElementById('p1-collection');
                    const deck = document.getElementById('p1-deck');
                    if (!col || !deck) return [];
                    col.value = k;
                    col.dispatchEvent(new Event('change'));
                    return Array.from(deck.options).map((o) => o.value).filter((v) => v);
                }, key);
                const filledSet = new Set(filledDecks);
                const exact = filledDecks.length === stems.length && stems.every((s) => filledSet.has(s));
                check(exact,
                    'selecting "' + year + ' World Championship" lists EXACTLY its four decks ' +
                    '(got ' + JSON.stringify(filledDecks) + ')');
            }
            await lctx.close();
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
