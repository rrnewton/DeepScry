#!/usr/bin/env node
/**
 * Regression test for the native_game.html AI-WATCH AFFORDANCE, choice-based
 * design (slot05 / claude/web-ui-fixes).
 *
 * DESIGN (user feedback on the earlier footer): when an AI seat is paused and it
 * is our move to advance, the affordance is presented as NORMAL numbered CHOICES
 * in the action list — "1. Play next <Kind> AI move" / "2. Auto-run" — the SAME
 * UI a human-controller game uses ("1. Pass / 2. Cast Lightning Bolt"), NOT a
 * separate footer banner. The bottom footer (#actions-affordance) is now the
 * GREEN "auto-running" banner ONLY.
 *
 * Properties asserted:
 *  (1) IN-LIST CHOICES: a paused local random-vs-random game shows two
 *      action-items ("Play next … move" + "Auto-run") — not "No actions
 *      available" — and the footer banner is HIDDEN while paused. No wrong
 *      "waiting for the other player" text appears on our own turn.
 *  (2) "PLAY NEXT" advances: selecting choice 1 (number key) advances the game.
 *  (3) "AUTO-RUN" choice enables auto-run: selecting choice 2 turns the GREEN
 *      footer banner on and flips #btn-auto to "Stop Auto"; pressing A reverts
 *      to the paused in-list choices with the banner hidden.
 *  (4) No WASM panics / non-image browser errors.
 *
 * Run with: node test_action_affordance.js
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const projectRoot = path.join(__dirname, '..');
function log(m) { const ts = new Date().toISOString().substring(11, 23); console.log(`[${ts}] ${m}`); }

// Snapshot of the action pane: the in-list choices (data-meta entries), the
// footer banner, the auto-run button, and view-model bits — read straight from
// the DOM (no production test-only hook needed).
function readPaneInPage() {
    const body = document.getElementById('actions-body');
    const footer = document.getElementById('actions-affordance');
    const btn = document.getElementById('btn-auto');
    const vm = window.__mtg.getViewModel();
    const items = body ? Array.from(body.querySelectorAll('.action-item')) : [];
    return {
        // All action-items currently in the list (real or synthetic).
        itemTexts: items.map(el => el.textContent.replace(/\s+/g, ' ').trim()),
        // Synthetic meta-choices specifically (carry data-meta).
        metaActions: items.filter(el => el.dataset.meta).map(el => el.dataset.meta),
        bodyText: body ? body.textContent.replace(/\s+/g, ' ').trim() : '',
        footerVisible: !!footer && footer.style.display !== 'none',
        footerText: footer ? footer.textContent.replace(/\s+/g, ' ').trim() : '',
        footerAutoOn: !!footer && footer.classList.contains('auto-on'),
        btnText: btn ? btn.textContent.trim() : '',
        turn: vm.turn_number,
        gameOver: !!vm.game_over,
        choices: (vm.choices || []).length,
    };
}

(async () => {
    let httpServer, browser;
    const { httpPort: HTTP_PORT } = await getRandomPorts();
    const failures = [];
    function check(name, ok, detail) {
        if (ok) log(`PASS: ${name} — ${detail}`);
        else { log(`FAIL: ${name} — ${detail}`); failures.push(`${name}: ${detail}`); }
    }
    try {
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()],
            { cwd: path.join(projectRoot, 'web'), stdio: ['ignore', 'pipe', 'pipe'] });
        await new Promise(r => setTimeout(r, 1500));
        browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--enable-unsafe-swiftshader'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });
        const browserErrors = [];
        page.on('pageerror', e => browserErrors.push(e.message));
        page.on('console', m => { if (m.type() === 'error') browserErrors.push(`console.error: ${m.text()}`); });

        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);

        // Boot a PAUSED local random-vs-random game (both seats AI, no auto_run →
        // empty engine choices, our seat is AI so the game waits for us).
        await page.goto(localGameUrl(base, 'native_game.html', { deck, p1: 'random', p2: 'random', seed: 42 }),
            { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(1200);

        // ── (1) IN-LIST CHOICES in the paused AI-watch state. ───────────────────
        const initial = await page.evaluate(readPaneInPage);
        check('paused AI-watch shows the two synthetic meta-choices in the action list',
              !initial.gameOver && initial.metaActions.length === 2
              && initial.metaActions.includes('continue') && initial.metaActions.includes('autorun'),
              `metaActions=[${initial.metaActions}] items=${JSON.stringify(initial.itemTexts)}`);
        check('action list is NOT "No actions available" while paused',
              !/no actions available/i.test(initial.bodyText),
              `bodyText="${initial.bodyText}"`);
        check('choice 1 reads "Play next … move", choice 2 reads "Auto-run"',
              /play next/i.test(initial.itemTexts[0] || '') && /auto-run/i.test(initial.itemTexts[1] || ''),
              `items=${JSON.stringify(initial.itemTexts)}`);
        // fix #3: no contradictory "waiting for the other player" text on our turn.
        check('no wrong "waiting for the other player" text on our own turn',
              !/waiting for the other player/i.test(initial.bodyText)
              && !/waiting for the other player/i.test(initial.footerText),
              `bodyText="${initial.bodyText}" footerText="${initial.footerText}"`);
        // The green auto-running footer is hidden while paused.
        check('green auto-running footer is hidden while paused',
              !initial.footerVisible,
              `footerVisible=${initial.footerVisible} footerText="${initial.footerText}"`);

        // ── (2) "Play next" (choice 1) advances the game. ───────────────────────
        const beforeTurn = initial.turn;
        let advanced = false;
        for (let i = 0; i < 8 && !advanced; i++) {
            await page.keyboard.press('1');           // select meta-choice 1 = continue
            await page.waitForTimeout(140);
            const s = await page.evaluate(readPaneInPage);
            if (s.gameOver || s.turn !== beforeTurn || s.choices > 0) advanced = true;
        }
        check('selecting "Play next … move" (key 1) advances the game',
              advanced,
              `started at turn ${beforeTurn}; advanced=${advanced}`);

        // ── (3) "Auto-run" (choice 2) turns on the GREEN banner. Reboot a fresh
        // paused game so the meta-choices are present and the game is running. ──
        await page.goto(localGameUrl(base, 'native_game.html', { deck, p1: 'random', p2: 'random', seed: 7 }),
            { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(1200);
        const paused = await page.evaluate(readPaneInPage);
        if (!paused.gameOver && paused.metaActions.includes('autorun')) {
            await page.keyboard.press('2');           // select meta-choice 2 = autorun
            await page.waitForTimeout(80);
            const on = await page.evaluate(readPaneInPage);
            // Either we caught auto-running (green banner + "Stop Auto"), or
            // auto-run already raced to game over.
            const sawBanner = on.footerVisible && on.footerAutoOn && /auto-running/i.test(on.footerText)
                && /Stop Auto/i.test(on.btnText);
            check('selecting "Auto-run" (key 2) turns on the GREEN auto-running banner',
                  sawBanner || on.gameOver,
                  `footerVisible=${on.footerVisible} autoOn=${on.footerAutoOn} btn="${on.btnText}" footer="${on.footerText}" gameOver=${on.gameOver}`);

            // Press A to stop auto-run → banner hidden, paused choices return.
            const mid = await page.evaluate(readPaneInPage);
            if (!mid.gameOver) {
                await page.keyboard.press('a');
                await page.waitForTimeout(120);
                const off = await page.evaluate(readPaneInPage);
                check('stopping auto-run (A) hides the banner and restores the in-list choices',
                      off.gameOver
                      || (!off.footerVisible && /Auto Run/i.test(off.btnText) && off.metaActions.length === 2),
                      `footerVisible=${off.footerVisible} btn="${off.btnText}" metaActions=[${off.metaActions}] gameOver=${off.gameOver}`);
            } else {
                log('INFO: game reached over after enabling auto-run; skipping revert assertion');
            }
        } else {
            log('INFO: fresh game already over or no autorun choice; skipping auto-run assertion (rare)');
        }

        // ── (4) no WASM panics / non-image browser errors. ──────────────────────
        const nonImage = browserErrors.filter(e => !(e.includes('Failed to load resource') && e.includes('404')));
        check('no non-image browser errors / WASM panics', nonImage.length === 0,
              nonImage.length === 0 ? 'clean' : nonImage.slice(0, 3).join(' | '));
    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill();
    }
    if (failures.length === 0) { log('=== ALL TESTS PASSED ==='); process.exit(0); }
    else { log(`=== FAILURES (${failures.length}) ===`); failures.forEach(f => log(`  - ${f}`)); process.exit(1); }
})();
