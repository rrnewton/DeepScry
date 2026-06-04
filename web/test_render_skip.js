#!/usr/bin/env node
/**
 * Regression test for mtg-6vzht v1 — the updateUI CHANGE-DETECTION whole-render
 * skip. Two properties, the first is the safety-critical one:
 *
 *  (B) A REAL CHANGE IS NEVER SKIPPED (no under-render → no stale UI). After
 *      every game advance, the DOM-rendered status bar must reflect the CURRENT
 *      view-model turn/phase. If a state-changing tick were wrongly skipped, the
 *      rendered turn would lag the view model — the dangerous bug.
 *
 *  (A) AN UNCHANGED TICK DOES NO DOM WORK (the skip actually fires). With a
 *      MutationObserver on the battlefield, benign no-op ticks (arrow nav with no
 *      pending choice) produce far FEWER battlefield mutations than real game
 *      advances — i.e. the skip preserves the DOM (and element identity) when
 *      nothing changed.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const projectRoot = path.join(__dirname, '..');
function log(m) { const ts = new Date().toISOString().substring(11, 23); console.log(`[${ts}] ${m}`); }

(async () => {
    let httpServer, browser;
    const { httpPort: HTTP_PORT } = await getRandomPorts();
    let failures = [];
    function check(name, ok, detail) {
        if (ok) log(`PASS: ${name} — ${detail}`);
        else { log(`FAIL: ${name} — ${detail}`); failures.push(`${name}: ${detail}`); }
    }
    try {
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], { cwd: path.join(projectRoot, 'web'), stdio: ['ignore', 'pipe', 'pipe'] });
        await new Promise(r => setTimeout(r, 1500));
        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });
        const browserErrors = [];
        page.on('pageerror', e => browserErrors.push(e.message));
        page.on('console', m => { if (m.type() === 'error') browserErrors.push(`console.error: ${m.text()}`); });

        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', { deck, p1: 'heuristic', p2: 'heuristic', seed: 42 }),
            { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(1200);

        // ── (B) a real change is NEVER skipped: rendered status must track the
        // view model's turn across many advances. ──────────────────────────
        let mismatches = 0, advances = 0;
        for (let i = 0; i < 20; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(110);
            const m = await page.evaluate(() => {
                const vm = window.__mtg.getViewModel();
                const statusEl = document.getElementById('status-bar') || document.querySelector('.status-bar') || document.getElementById('status-text');
                const statusTxt = statusEl ? statusEl.textContent : '';
                return { vmTurn: vm.turn_number, vmStep: vm.current_step, gameOver: vm.game_over, statusTxt };
            });
            advances++;
            // The rendered status bar must mention the CURRENT view-model turn
            // (it formats "Turn N | Phase: …"). A skipped real change would leave
            // the OLD turn number rendered.
            if (m.statusTxt && !m.statusTxt.includes(`Turn ${m.vmTurn}`)) mismatches++;
            if (m.gameOver) break;
        }
        check('rendered status bar always reflects the current view-model turn (no under-render on real changes)',
              mismatches === 0,
              `${advances} advances, ${mismatches} stale-render mismatches`);

        // ── (A) an unchanged tick does no battlefield DOM work. Use REAL key
        // presses (through the input pipeline, so they trigger the real
        // updateUI) and a MutationObserver on the battlefield: benign no-op nav
        // ticks must produce far FEWER battlefield mutations than real advances.
        await page.evaluate(() => {
            window.__navMut = 0;
            window.__mutObs = new MutationObserver(muts => { window.__navMut += muts.length; });
            window.__mutObs.observe(document.getElementById('player-field-cards'),
                { childList: true, subtree: true, attributes: true, characterData: true });
        });
        for (let i = 0; i < 10; i++) {
            await page.keyboard.press('ArrowDown'); await page.waitForTimeout(45);
            await page.keyboard.press('ArrowUp'); await page.waitForTimeout(45);
        }
        const navMut = await page.evaluate(() => { const n = window.__navMut; window.__navMut = 0; return n; });
        // Now real advances (state changes) for contrast — expect mutations.
        let advMut = 0;
        for (let i = 0; i < 6; i++) {
            await page.keyboard.press('Space'); await page.waitForTimeout(110);
            advMut += await page.evaluate(() => { const n = window.__navMut; window.__navMut = 0; return n; });
        }
        await page.evaluate(() => window.__mutObs.disconnect());
        check('benign no-op nav ticks produce ~no battlefield DOM mutations (skip fires)',
              navMut <= 2,
              `battlefield mutations during 20 no-op nav ticks = ${navMut} (≈0 expected when the render-skip fires; real advances produced ${advMut})`);

        const nonImage404 = browserErrors.filter(e => !(e.includes('Failed to load resource') && e.includes('404')));
        check('no non-image browser errors / WASM panics', nonImage404.length === 0,
              nonImage404.length === 0 ? 'clean' : nonImage404.slice(0, 3).join(' | '));
    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill();
    }
    if (failures.length === 0) { log('=== ALL TESTS PASSED ==='); process.exit(0); }
    else { log(`=== FAILURES (${failures.length}) ===`); failures.forEach(f => log(`  - ${f}`)); process.exit(1); }
})();
