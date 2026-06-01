#!/usr/bin/env node
/**
 * E2E test for decouple-step6: WASM `WasmHumanController` populates
 * `GameUiSessionState::valid_choices` so `is_valid_choice` highlighting
 * works in both `web/native_game.html` (HTML GUI) and `web/tui_game.html` (ratzilla
 * canvas).
 *
 * Pre-step-6 the field was only ever written by the *native*
 * `FancyTuiController` / `FancyFixedController`. The WASM controllers
 * went through `ChoiceResult::NeedInput(ChoiceContext::*)` and never
 * touched the renderer state, so every `card.is_valid_choice` flag in
 * the GUI view model JSON was silently `false` for the entire game —
 * none of the cards the human could pick lit up as such.
 *
 * Test setup: human-vs-heuristic on native_game.html with seed 42, drive into
 * a state where the human has a real spell-ability prompt with multiple
 * options, then read the GUI view model JSON and assert that:
 *   1. `state.choices` is non-empty (controller is asking for input).
 *   2. At least one CardView in the hand or battlefield has
 *      `is_valid_choice === true` (highlight is now wired up).
 *
 * If `valid_choices` is still empty (the bug), assertion (2) fails
 * and we surface that as a regression.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const projectRoot = path.join(__dirname, '..');

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
        page.on('console', msg => {
            if (msg.type() === 'error') browserErrors.push(`console.error: ${msg.text()}`);
        });

        // mtg-35z3s page 3: native_game.html is a PURE renderer — boot via URL
        // params. Human (P1) vs heuristic (P2) gives P1 real prompts to handle.
        // Same deck both sides, seed 42 for reproducibility (matches the setup
        // test_game_gui_bugfixes.js uses, where the human gets a priority prompt
        // with several land/pass choices on turn 1).
        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', {
            deck, p1: 'human', p2: 'heuristic', seed: 42,
        }), { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(2000);

        // Helper: read the GuiViewModel JSON directly out of WASM and
        // collect every is_valid_choice flag we can see on cards across
        // the hand and both battlefields.
        const sampleViewModel = async () => page.evaluate(async () => {
            const mod = await import('/pkg/mtg_engine.js');
            const json = mod.tui_get_gui_view_model_json();
            const vm = JSON.parse(json);

            const cards = [];
            for (const player of vm.players || []) {
                for (const c of player.hand || []) {
                    cards.push({ where: 'hand', name: c.name, id: c.card_id, valid: !!c.is_valid_choice });
                }
                for (const sec of player.battlefield_sections || []) {
                    for (const c of sec.cards || []) {
                        cards.push({ where: `bf:${sec.label}`, name: c.name, id: c.card_id, valid: !!c.is_valid_choice });
                    }
                }
            }

            return {
                turnNumber: vm.turn_number,
                choiceCount: (vm.choices || []).length,
                statusText: vm.status_text,
                gameOver: vm.game_over,
                cards,
                validCards: cards.filter(c => c.valid),
            };
        });

        // Initial sample — turn 1, human should be at a priority prompt.
        const initial = await sampleViewModel();
        log(`Initial state: turn=${initial.turnNumber}, ` +
            `choices=${initial.choiceCount}, ` +
            `cards visible=${initial.cards.length}, ` +
            `is_valid_choice=true count=${initial.validCards.length}`);

        check('controller is asking for human input (choices > 0)',
              initial.choiceCount > 0,
              `${initial.choiceCount} choices in vm.choices`);

        // The headline assertion: AT LEAST ONE card has is_valid_choice=true.
        // Pre-step-6, this was ALWAYS false because WasmHumanController
        // never wrote valid_choices. Post-step-6, the Spell/Ability prompt
        // populates valid_choices with the controllers / lands the human
        // can play.
        check('at least one card has is_valid_choice=true (decouple-step6)',
              initial.validCards.length > 0,
              initial.validCards.length > 0
                  ? `${initial.validCards.length} valid: ${initial.validCards.slice(0, 5).map(c => `${c.name}@${c.where}`).join(', ')}`
                  : `0 valid out of ${initial.cards.length} cards visible — pre-step-6 bug still present`);

        // Walk the game forward: pass priority a few times, then sample
        // again. valid_choices should reset (and be non-empty when there
        // are spells to play).
        for (let i = 0; i < 4; i++) {
            await page.keyboard.press(' '); // Space = commit highlighted (decouple-step4) + advance
            await page.waitForTimeout(150);
        }
        const after = await sampleViewModel();
        log(`After 4x Space: turn=${after.turnNumber}, ` +
            `choices=${after.choiceCount}, ` +
            `is_valid_choice=true count=${after.validCards.length}`);

        if (after.choiceCount > 0) {
            check('valid_choices recomputed after each prompt (still highlighted post-Space)',
                  after.validCards.length > 0,
                  after.validCards.length > 0
                      ? `${after.validCards.length} valid: ${after.validCards.slice(0, 3).map(c => `${c.name}@${c.where}`).join(', ')}`
                      : `0 valid — clear-then-repopulate path may be broken`);
        } else {
            log(`(no pending choice after passing — skip the post-pass valid check)`);
        }

        // Also confirm: when there is NO pending prompt (game advanced
        // past the human's turn), valid_choices should be empty (we
        // shouldn't be highlighting cards that aren't choosable).
        // The clear_pending_choice_highlights() helper handles this.
        if (after.choiceCount === 0) {
            check('valid_choices cleared when no choice is pending',
                  after.validCards.length === 0,
                  `${after.validCards.length} valid (expected 0 with no prompt)`);
        }

        const nonImage404Errors = browserErrors.filter(e =>
            !(e.includes('Failed to load resource') && e.includes('404'))
        );
        check('no non-image browser errors / WASM panics during the run',
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
