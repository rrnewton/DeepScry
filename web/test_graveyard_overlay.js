#!/usr/bin/env node
/**
 * E2E test for the Graveyard | Exile side pane in `web/native_game.html`.
 *
 * Layout (post mtg-815 + exile-tab): both players' graveyard/exile contents
 * live in a dedicated `#pane-graveyard` on the right, laid out as two columns
 * ("Ours" / "Theirs"). The pane header is a two-tab control — "Graveyard"
 * (upper-left, default) and "Exile" (upper-right) — real `<button role=tab>`
 * controls. Clicking a tab switches which zone the SHARED renderer
 * (`renderZonePane`) shows; the active tab is highlighted and preserved across
 * game-state re-renders. The older hand-pane graveyard (`#hand-graveyard`) and
 * battlefield overlays (`#opp-graveyard-overlay`) no longer exist.
 *
 * This test verifies: the pane + tabs exist with correct default state; the
 * graveyard fills after combat and renders in the pane; the Exile tab switches
 * (highlight + aria), shows its own empty-state, and switching back restores
 * the graveyard; clicking a graveyard card drives the Card Details pane; and
 * the G-key full-graveyard popup (mtg-444) still works.
 *
 * Screenshots are written to the gitignored `debug/` directory.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { getRandomPorts } = require('./test_network_utils');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const projectRoot = path.join(__dirname, '..');
const debugDir = path.join(projectRoot, 'debug');

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

(async () => {
    let httpServer, browser;
    const { httpPort: HTTP_PORT } = await getRandomPorts();
    let failures = [];

    function check(name, ok, detail) {
        if (ok) {
            log(`PASS: ${name} — ${detail}`);
        } else {
            log(`FAIL: ${name} — ${detail}`);
            failures.push(`${name}: ${detail}`);
        }
    }

    try {
        fs.mkdirSync(debugDir, { recursive: true });

        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: path.join(projectRoot, 'web'),
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        await new Promise(r => setTimeout(r, 1500));

        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });

        const browserErrors = [];
        page.on('pageerror', err => browserErrors.push(err.message));

        // mtg-35z3s page 3: native_game.html is a PURE renderer — boot via URL params.
        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', {
            deck, p1: 'heuristic', p2: 'heuristic', seed: 42,
        }), { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(2000);

        // ── Structural DOM: the dedicated graveyard/exile pane + its tabs ─────
        const initialDom = await page.evaluate(() => ({
            panePresent: !!document.getElementById('pane-graveyard'),
            gyTabPresent: !!document.getElementById('zone-tab-graveyard'),
            exileTabPresent: !!document.getElementById('zone-tab-exile'),
            ourCardsPresent: !!document.getElementById('graveyard-our-cards'),
            oppCardsPresent: !!document.getElementById('graveyard-opp-cards'),
            // Graveyard is the default-active tab.
            gyActive: !!document.querySelector('#zone-tab-graveyard.active'),
            gyAria: document.getElementById('zone-tab-graveyard')?.getAttribute('aria-selected'),
            exileActive: !!document.querySelector('#zone-tab-exile.active'),
            exileAria: document.getElementById('zone-tab-exile')?.getAttribute('aria-selected'),
            // The OLD hand-pane graveyard + battlefield overlays must be gone.
            oldHandGyGone: !document.getElementById('hand-graveyard'),
            oldOppOverlayGone: !document.getElementById('opp-graveyard-overlay'),
            oldPlayerOverlayGone: !document.getElementById('player-graveyard-overlay'),
        }));
        check('dedicated #pane-graveyard exists', initialDom.panePresent, 'in DOM');
        check('Graveyard + Exile tabs exist',
              initialDom.gyTabPresent && initialDom.exileTabPresent,
              `gy=${initialDom.gyTabPresent}, exile=${initialDom.exileTabPresent}`);
        check('Ours / Theirs card columns exist',
              initialDom.ourCardsPresent && initialDom.oppCardsPresent,
              `ours=${initialDom.ourCardsPresent}, theirs=${initialDom.oppCardsPresent}`);
        check('Graveyard tab is active by default (highlight + aria-selected)',
              initialDom.gyActive && initialDom.gyAria === 'true'
                && !initialDom.exileActive && initialDom.exileAria === 'false',
              `gyActive=${initialDom.gyActive}/${initialDom.gyAria}, exileActive=${initialDom.exileActive}/${initialDom.exileAria}`);
        check('old hand-pane graveyard + battlefield overlays removed',
              initialDom.oldHandGyGone && initialDom.oldOppOverlayGone && initialDom.oldPlayerOverlayGone,
              `handGyGone=${initialDom.oldHandGyGone}, oppOverlayGone=${initialDom.oldOppOverlayGone}, playerOverlayGone=${initialDom.oldPlayerOverlayGone}`);

        // ── Drive enough turns that creatures die in combat → graveyard fills ─
        for (let i = 0; i < 30; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(80);
        }
        await page.waitForTimeout(500);

        const afterPlay = await page.evaluate(() => {
            const ourCardsEl = document.getElementById('graveyard-our-cards');
            const oppCardsEl = document.getElementById('graveyard-opp-cards');
            const ourHeader = document.getElementById('graveyard-our-header');
            const oppHeader = document.getElementById('graveyard-opp-header');
            const count = el => el ? el.querySelectorAll('.graveyard-overlay-card').length : 0;

            const stateJson = window.tui_get_gui_view_model_json
                ? window.tui_get_gui_view_model_json()
                : null;
            let vmGyCounts = null;
            if (stateJson) {
                try {
                    const vm = JSON.parse(stateJson);
                    vmGyCounts = (vm.players || []).map(p => ({
                        name: p.name,
                        gySize: p.graveyard?.length || 0,
                        // The exile field MUST be present on the serialized
                        // PlayerView now (engine view-model addition).
                        hasExileField: Object.prototype.hasOwnProperty.call(p, 'exile'),
                        exileSize: p.exile?.length || 0,
                    }));
                } catch (_) { /* ignore */ }
            }

            return {
                ourCount: count(ourCardsEl),
                oppCount: count(oppCardsEl),
                ourHeader: ourHeader?.textContent || '',
                oppHeader: oppHeader?.textContent || '',
                vmGyCounts,
            };
        });

        log(`After 30x Space:`);
        log(`  graveyard pane: ours=${afterPlay.ourCount} ("${afterPlay.ourHeader}"), theirs=${afterPlay.oppCount} ("${afterPlay.oppHeader}")`);
        log(`  view model: ${JSON.stringify(afterPlay.vmGyCounts)}`);

        const totalGyCards = afterPlay.ourCount + afterPlay.oppCount;
        check('graveyard pane shows cards after play',
              totalGyCards > 0,
              `ours=${afterPlay.ourCount}, theirs=${afterPlay.oppCount}`);

        // Engine view-model addition: every PlayerView carries an `exile` field.
        if (afterPlay.vmGyCounts && afterPlay.vmGyCounts.length >= 1) {
            check('serialized PlayerView carries the exile field (engine plumb)',
                  afterPlay.vmGyCounts.every(p => p.hasExileField),
                  JSON.stringify(afterPlay.vmGyCounts.map(p => p.hasExileField)));
        }

        // Header total parity with the view model (rendered count == VM length,
        // there is no ellision in the dedicated pane).
        if (afterPlay.vmGyCounts && afterPlay.vmGyCounts.length === 2) {
            const headerTotal = parseInt((afterPlay.ourHeader.match(/\((\d+)\)/) || [])[1] || '0', 10);
            check('Ours graveyard header count matches rendered cards',
                  headerTotal === afterPlay.ourCount,
                  `header=${headerTotal}, rendered=${afterPlay.ourCount}`);
        }

        const normalShot = path.join(debugDir, 'graveyard_pane_normal.png');
        await page.locator('#pane-graveyard').screenshot({ path: normalShot });
        log(`SCREENSHOT (graveyard tab): ${normalShot}`);

        // ── Exile tab: switch, verify highlight/aria + empty-state, restore ──
        // Inject a synthetic exiled card so the Exile tab has visible content
        // to render through the SAME renderer (exile is otherwise infrequent).
        await page.evaluate(() => {
            const st = window._lastRenderedState;
            if (st && Array.isArray(st.players)) {
                const them = st.players.find(p => !p.is_us) || st.players[1];
                if (them) them.exile = [{ card_id: 990002, name: 'Banishing Light (test)' }];
            }
        });
        await page.locator('#zone-tab-exile').click();
        await page.waitForTimeout(250);
        const exileState = await page.evaluate(() => ({
            exileActive: !!document.querySelector('#zone-tab-exile.active'),
            exileAria: document.getElementById('zone-tab-exile').getAttribute('aria-selected'),
            gyActive: !!document.querySelector('#zone-tab-graveyard.active'),
            gyAria: document.getElementById('zone-tab-graveyard').getAttribute('aria-selected'),
            // Our exile is empty → shared empty-state text; theirs has the card.
            ourBody: document.getElementById('graveyard-our-cards').textContent,
            oppBody: document.getElementById('graveyard-opp-cards').textContent,
        }));
        log(`Exile tab: active=${exileState.exileActive}/${exileState.exileAria}, ours="${exileState.ourBody.slice(0, 40)}", theirs="${exileState.oppBody.slice(0, 60)}"`);
        check('Exile tab activates (highlight + aria) and Graveyard deactivates',
              exileState.exileActive && exileState.exileAria === 'true'
                && !exileState.gyActive && exileState.gyAria === 'false',
              `exile=${exileState.exileActive}/${exileState.exileAria}, gy=${exileState.gyActive}/${exileState.gyAria}`);
        check('Exile tab shows per-zone empty-state for an empty column',
              /No cards in exile/.test(exileState.ourBody),
              `ourBody="${exileState.ourBody.slice(0, 60)}"`);
        check('Exile tab renders exile cards through the shared renderer',
              /Banishing Light \(test\)/.test(exileState.oppBody),
              `oppBody="${exileState.oppBody.slice(0, 60)}"`);

        const exileShot = path.join(debugDir, 'graveyard_pane_exile.png');
        await page.locator('#pane-graveyard').screenshot({ path: exileShot });
        log(`SCREENSHOT (exile tab): ${exileShot}`);

        // Switch back to Graveyard → contents restored.
        await page.locator('#zone-tab-graveyard').click();
        await page.waitForTimeout(250);
        const restored = await page.evaluate(() => ({
            gyActive: !!document.querySelector('#zone-tab-graveyard.active'),
            ourCount: document.getElementById('graveyard-our-cards').querySelectorAll('.graveyard-overlay-card').length
                    + document.getElementById('graveyard-opp-cards').querySelectorAll('.graveyard-overlay-card').length,
        }));
        check('switching back to Graveyard restores its contents',
              restored.gyActive && restored.ourCount === totalGyCards,
              `gyActive=${restored.gyActive}, cards=${restored.ourCount} (was ${totalGyCards})`);

        // ── Clicking a graveyard card drives the Card Details pane ────────────
        if (await page.locator('#pane-graveyard .graveyard-overlay-card').count() > 0) {
            const beforeClick = await page.textContent('#card-details-body');
            await page.locator('#pane-graveyard .graveyard-overlay-card').first().click();
            await page.waitForTimeout(300);
            const afterClick = await page.textContent('#card-details-body');
            check('clicking a graveyard card updates the Card Details pane',
                  afterClick !== beforeClick,
                  `details changed (length ${beforeClick.length} → ${afterClick.length})`);
        }

        // ── mtg-444: the Shift+G full-graveyard popup ────────────────────────
        // Keymap rework (task #6): the graveyard POPUP (a big overlay) now needs
        // the Shift modifier — unmodified `g` FOCUSES the graveyard pane instead
        // (modifier-multiplexing: popups require Shift/Ctrl). Verify plain `g`
        // does NOT open the popup, then Shift+G does.
        await page.keyboard.press('g');
        await page.waitForTimeout(150);
        const gyPopupAfterPlainG = await page.evaluate(() =>
            document.getElementById('graveyard-dialog')?.classList.contains('show'));
        check('plain g does NOT open the graveyard popup (it focuses the pane)',
              !gyPopupAfterPlainG, `popupShown=${gyPopupAfterPlainG}`);

        await page.keyboard.press('Shift+G');
        await page.waitForTimeout(250);
        const gyPopup = await page.evaluate(() => {
            const dialog = document.getElementById('graveyard-dialog');
            const overlay = document.getElementById('graveyard-dialog-overlay');
            const content = document.getElementById('graveyard-dialog-content');
            return {
                dialogShown: !!dialog && dialog.classList.contains('show'),
                overlayShown: !!overlay && overlay.classList.contains('show'),
                sectionCount: content ? content.querySelectorAll('.gy-section').length : 0,
                cardCount: content ? content.querySelectorAll('.gy-card').length : 0,
                titleEntries: content
                    ? Array.from(content.querySelectorAll('.gy-section-title')).map(e => e.textContent.trim())
                    : [],
            };
        });
        log(`Shift+G popup: shown=${gyPopup.dialogShown}, sections=${gyPopup.sectionCount}, cards=${gyPopup.cardCount}, titles=${JSON.stringify(gyPopup.titleEntries)}`);
        check('Shift+G opens the graveyard popup (dialog + overlay shown)',
              gyPopup.dialogShown && gyPopup.overlayShown,
              `dialog=${gyPopup.dialogShown}, overlay=${gyPopup.overlayShown}`);
        check('graveyard popup has one section per player',
              gyPopup.sectionCount >= 2,
              `sections=${gyPopup.sectionCount}`);
        check('graveyard popup lists real cards (non-empty graveyards)',
              gyPopup.cardCount > 0,
              `cards=${gyPopup.cardCount}`);

        const gyPopupShot = path.join(debugDir, 'graveyard_g_popup.png');
        await page.screenshot({ path: gyPopupShot, fullPage: false });
        log(`SCREENSHOT (G popup): ${gyPopupShot}`);

        // Any key dismisses it (capture-phase handler, same contract as ? help).
        await page.keyboard.press('Escape');
        await page.waitForTimeout(200);
        const gyClosed = await page.evaluate(() => {
            const dialog = document.getElementById('graveyard-dialog');
            return !dialog.classList.contains('show');
        });
        check('any key dismisses the graveyard popup', gyClosed, `closed=${gyClosed}`);

        const nonImage404Errors = browserErrors.filter(e =>
            !(e.includes('Failed to load resource') && e.includes('404'))
        );
        check('no non-image browser errors / WASM panics',
              nonImage404Errors.length === 0,
              nonImage404Errors.length === 0
                  ? `clean (${browserErrors.length} expected image-404 fallbacks ignored)`
                  : nonImage404Errors.slice(0, 3).join(' | '));

    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill();
    }

    if (failures.length === 0) {
        log('=== ALL TESTS PASSED ===');
        process.exit(0);
    } else {
        log(`=== FAILURES (${failures.length}) ===`);
        failures.forEach(f => log(`  - ${f}`));
        process.exit(1);
    }
})();
