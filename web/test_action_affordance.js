#!/usr/bin/env node
/**
 * Regression test for the native_game.html ACTION-PANE AFFORDANCE FOOTER
 * (slot05 / claude/native-action-pane-hints).
 *
 * USER-REPORTED PROBLEM: when watching a random-vs-random (both-AI) game, the
 * Actions pane went silent — the engine's semantic prompt is things like
 * "Game ready. Press Space to advance turn.", "Network AI game running...", or
 * "Waiting for server..." with an EMPTY choice list, so the pane showed only
 * "No actions available" and nothing telling the user the next move. The fix
 * adds a persistent affordance footer (#actions-affordance) that surfaces the
 * page's meta-controls (Space to continue / A to toggle auto-run) whenever the
 * game is in a watch / auto-advance / waiting state.
 *
 * Properties asserted (the affordance is the deliverable, so these are the gate):
 *  (1) NEVER SILENT during AI turns: in a paused local random-vs-random game the
 *      affordance footer is visible + non-empty and names the Space / auto-run
 *      controls — and STAYS visible as we advance turn-by-turn (the core
 *      complaint: the pane must always tell the user what to do).
 *  (2) ACCURATE ON TOGGLE: pressing 'A' (toggle auto-run) flips the affordance to
 *      its auto-running treatment, and the #btn-auto label flips in lockstep
 *      (single source of truth — applyAutoRunState). Pressing 'A' again reverts.
 *  (3) HIDDEN WHEN A REAL CHOICE IS PENDING / AT GAME OVER (no double-guidance):
 *      verified opportunistically — when the run reaches game over the footer is
 *      hidden (the game is finished; nothing left to do).
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

// Snapshot of the affordance footer + the auto-run button label, read straight
// from the DOM (no production test-only hook needed). `visible` is the rendered
// display state; `text` is normalized (collapsed whitespace) for substring
// checks; `autoOn` reflects the auto-running CSS treatment.
function readAffordanceInPage() {
    const el = document.getElementById('actions-affordance');
    const btn = document.getElementById('btn-auto');
    const vm = window.__mtg.getViewModel();
    return {
        visible: !!el && el.style.display !== 'none',
        text: el ? el.textContent.replace(/\s+/g, ' ').trim() : '',
        autoOn: !!el && el.classList.contains('auto-on'),
        btnText: btn ? btn.textContent.trim() : '',
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

        // Boot a PAUSED local random-vs-random game (the exact user scenario:
        // both seats AI, no auto_run → engine prompt has empty choices, our seat
        // is AI so the game waits for Space / Auto-Run). seed fixed for repro.
        await page.goto(localGameUrl(base, 'native_game.html', { deck, p1: 'random', p2: 'random', seed: 42 }),
            { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(1200);

        // ── (1) NEVER SILENT: the affordance is visible + names the controls in
        // the initial paused AI-watch state. ───────────────────────────────────
        const initial = await page.evaluate(readAffordanceInPage);
        check('action pane is NOT silent in paused AI-watch state (affordance footer visible + non-empty)',
              initial.visible && initial.text.length > 0 && !initial.gameOver,
              `visible=${initial.visible} gameOver=${initial.gameOver} text="${initial.text}"`);
        // It must name BOTH meta-controls the user needs to know about.
        check('paused affordance names the Space + auto-run controls',
              /Space/i.test(initial.text) && /auto-run/i.test(initial.text),
              `text="${initial.text}"`);
        // Paused (not auto-running) at boot: button reads "Auto Run".
        check('paused affordance is in non-auto treatment + button reads "Auto Run"',
              !initial.autoOn && /Auto Run/i.test(initial.btnText),
              `autoOn=${initial.autoOn} btn="${initial.btnText}"`);

        // ── (1 cont.) STAYS visible as we advance turn-by-turn (Space). The pane
        // must never go silent mid-AI-game while the game is still running. ─────
        let advances = 0, silentWhileRunning = 0;
        for (let i = 0; i < 12; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(110);
            const s = await page.evaluate(readAffordanceInPage);
            advances++;
            // While the game is running AND no human choice is pending, the
            // footer must be visible + non-empty. (random seats never produce a
            // human choice, so choices is expected to stay 0 here.)
            if (!s.gameOver && s.choices === 0 && !(s.visible && s.text.length > 0)) silentWhileRunning++;
            if (s.gameOver) {
                // (3) at game over the footer hides (nothing left to do).
                check('affordance hides at game over (no stale guidance)', !s.visible,
                      `visible=${s.visible} after ${advances} advances`);
                break;
            }
        }
        check('action pane NEVER goes silent during AI turns (footer stays visible while running)',
              silentWhileRunning === 0,
              `${advances} advances, ${silentWhileRunning} silent-while-running frames`);

        // ── (2) ACCURATE ON TOGGLE: only meaningful if the game is still
        // running. If the short Space loop above already finished the game,
        // re-boot a fresh paused game so we can exercise the toggle. ────────────
        let cur = await page.evaluate(readAffordanceInPage);
        if (cur.gameOver) {
            await page.goto(localGameUrl(base, 'native_game.html', { deck, p1: 'random', p2: 'random', seed: 7 }),
                { waitUntil: 'networkidle', timeout: 30000 });
            await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
            await page.waitForTimeout(1200);
            cur = await page.evaluate(readAffordanceInPage);
        }

        if (!cur.gameOver) {
            // Press 'A' → auto-run ON. Read immediately (short wait) so we catch
            // the auto-running treatment before auto-run can race to game over.
            await page.keyboard.press('a');
            await page.waitForTimeout(60);
            const on = await page.evaluate(readAffordanceInPage);
            // Either we caught it auto-running (footer auto-on + button "Stop
            // Auto"), or auto-run already drove to game over (footer hidden).
            const sawAutoOn = on.autoOn && /Stop Auto/i.test(on.btnText);
            check('toggling auto-run (A) flips the affordance + button to the auto-running treatment',
                  sawAutoOn || on.gameOver,
                  `autoOn=${on.autoOn} btn="${on.btnText}" gameOver=${on.gameOver} text="${on.text}"`);
            check('auto-running affordance mentions pausing auto-run',
                  on.gameOver || /pause/i.test(on.text),
                  `gameOver=${on.gameOver} text="${on.text}"`);

            // Press 'A' again → auto-run OFF (revert). Skip if the game ended.
            const mid = await page.evaluate(readAffordanceInPage);
            if (!mid.gameOver) {
                await page.keyboard.press('a');
                await page.waitForTimeout(80);
                const off = await page.evaluate(readAffordanceInPage);
                check('toggling auto-run OFF reverts the affordance + button',
                      off.gameOver || (!off.autoOn && /Auto Run/i.test(off.btnText) && off.visible),
                      `autoOn=${off.autoOn} btn="${off.btnText}" visible=${off.visible} gameOver=${off.gameOver}`);
            } else {
                log('INFO: game reached over after first toggle; skipping revert assertion');
            }
        } else {
            log('INFO: both seeds finished before the toggle could be exercised; toggle assertion skipped (rare)');
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
