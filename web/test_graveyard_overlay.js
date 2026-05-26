#!/usr/bin/env node
/**
 * E2E test for gamehtml-graveyard-display: each player's battlefield
 * pane in `web/native_game.html` shows a clickable graveyard overlay in the
 * bottom-right corner — mirroring the native ratatui TUI's
 * `render_graveyard_overlay`
 * (mtg-engine/src/game/fancy_tui_renderer.rs:3304).
 *
 * Pre-fix: no graveyard widget existed in `web/native_game.html`. The data
 * was always available in the GuiViewModel (`PlayerView.graveyard`)
 * but never rendered.
 *
 * Post-fix: each `.pane` (`#pane-opp-field`, `#pane-player-field`)
 * has a sibling `.graveyard-overlay` next to its `.pane-body`,
 * positioned absolutely (`position: absolute; right: 6px; bottom:
 * 6px`) and shown only when that player's graveyard is non-empty.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');

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

        await page.goto(`http://localhost:${HTTP_PORT}/native_game.html`, {
            waitUntil: 'networkidle',
            timeout: 30000,
        });
        await page.waitForFunction(() => {
            const s = document.getElementById('p1-deck');
            return s && s.options.length > 0;
        }, { timeout: 30000 });

        const deck = await page.evaluate(() => document.getElementById('p1-deck').options[0].value);
        await page.selectOption('#p1-deck', deck);
        await page.selectOption('#p2-deck', deck);
        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');
        await page.fill('#game-seed', '42');
        await page.click('#btn-launch');
        await page.waitForTimeout(2000);

        // Both overlay containers should EXIST in the DOM from the
        // moment the launcher renders, even if hidden.
        const initialDom = await page.evaluate(() => ({
            playerOverlayPresent: !!document.getElementById('player-graveyard-overlay'),
            oppOverlayPresent: !!document.getElementById('opp-graveyard-overlay'),
            playerInsidePane: !!document.querySelector('#pane-player-field > .graveyard-overlay'),
            oppInsidePane: !!document.querySelector('#pane-opp-field > .graveyard-overlay'),
        }));
        check('player-graveyard-overlay element exists',
              initialDom.playerOverlayPresent, 'in DOM');
        check('opp-graveyard-overlay element exists',
              initialDom.oppOverlayPresent, 'in DOM');
        check('overlays are direct children of their .pane (sibling of .pane-body)',
              initialDom.playerInsidePane && initialDom.oppInsidePane,
              `player=${initialDom.playerInsidePane}, opp=${initialDom.oppInsidePane}`);

        // Drive enough turns that creatures die in combat → graveyard
        // accumulates cards. Heuristic vs heuristic on seed 42 fills
        // both graveyards within ~10-15 turns.
        for (let i = 0; i < 30; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(80);
        }
        await page.waitForTimeout(500);

        const afterPlay = await page.evaluate(() => {
            const playerOv = document.getElementById('player-graveyard-overlay');
            const oppOv = document.getElementById('opp-graveyard-overlay');

            const sample = (ov) => {
                const cs = getComputedStyle(ov);
                const cards = Array.from(ov.querySelectorAll('.graveyard-overlay-card'));
                const headerEl = ov.querySelector('.graveyard-overlay-header');
                return {
                    display: cs.display,
                    position: cs.position,
                    right: cs.right,
                    bottom: cs.bottom,
                    zIndex: cs.zIndex,
                    hasCardsClass: ov.classList.contains('has-cards'),
                    headerText: headerEl?.textContent || '',
                    cardCount: cards.length,
                    cardNames: cards.slice(0, 5).map(c => c.textContent.trim()),
                };
            };

            // Also pull the GuiViewModel directly to cross-check that
            // what we render matches what the engine reports.
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
                    }));
                } catch (_) { /* ignore */ }
            }

            return {
                player: sample(playerOv),
                opp: sample(oppOv),
                vmGyCounts,
            };
        });

        log(`After 30x Space:`);
        log(`  player overlay: display=${afterPlay.player.display}, ${afterPlay.player.cardCount} cards, header="${afterPlay.player.headerText}"`);
        log(`  opp overlay: display=${afterPlay.opp.display}, ${afterPlay.opp.cardCount} cards, header="${afterPlay.opp.headerText}"`);
        log(`  view model graveyard sizes: ${JSON.stringify(afterPlay.vmGyCounts)}`);

        // At least one player's graveyard should have cards by turn ~10.
        const totalGyCards = afterPlay.player.cardCount + afterPlay.opp.cardCount;
        check('at least one graveyard overlay has cards after play',
              totalGyCards > 0,
              `player=${afterPlay.player.cardCount}, opp=${afterPlay.opp.cardCount}`);

        // The overlay should be positioned absolutely, anchored
        // bottom-right.
        const checkOverlayPosition = (name, info) => {
            if (info.cardCount === 0) {
                log(`  (skip position check for ${name} — empty graveyard, overlay hidden)`);
                return;
            }
            check(`${name} overlay is position:absolute`,
                  info.position === 'absolute',
                  `position=${info.position}`);
            check(`${name} overlay anchored bottom-right (right=6px, bottom=6px)`,
                  info.right === '6px' && info.bottom === '6px',
                  `right=${info.right}, bottom=${info.bottom}`);
            check(`${name} overlay header includes count`,
                  info.headerText.includes('Graveyard') && /\d/.test(info.headerText),
                  `header="${info.headerText}"`);
        };
        checkOverlayPosition('player', afterPlay.player);
        checkOverlayPosition('opp', afterPlay.opp);

        // View-model parity: rendered card count should match
        // PlayerView.graveyard.length for both players.
        if (afterPlay.vmGyCounts && afterPlay.vmGyCounts.length === 2) {
            // Players[0] is conventionally P1 (the local "us"), [1] is P2.
            // native_game.html flips ours/opp via our_player_idx, but for this
            // seed-42 setup our_player_idx=0 so player overlay = P1, opp
            // overlay = P2.
            const expectedPlayer = afterPlay.vmGyCounts[0].gySize;
            const expectedOpp = afterPlay.vmGyCounts[1].gySize;
            check('player overlay count matches VM (PlayerView.graveyard)',
                  afterPlay.player.cardCount === expectedPlayer,
                  `rendered=${afterPlay.player.cardCount}, expected=${expectedPlayer}`);
            check('opp overlay count matches VM (PlayerView.graveyard)',
                  afterPlay.opp.cardCount === expectedOpp,
                  `rendered=${afterPlay.opp.cardCount}, expected=${expectedOpp}`);
        }

        // Click a graveyard card → details pane should populate. Pick
        // whichever overlay has cards. NOTE: don't use `:first-child`
        // — the .graveyard-overlay-header is the first child, so the
        // first .graveyard-overlay-card is not also a :first-child of
        // its parent. `.graveyard-overlay-card` (any) + `nth(0)` works.
        const overlayWithCards = afterPlay.player.cardCount > 0 ? 'player-graveyard-overlay' : 'opp-graveyard-overlay';
        const beforeClick = await page.textContent('#card-details-body');
        await page.locator(`#${overlayWithCards} .graveyard-overlay-card`).first().click();
        await page.waitForTimeout(300);
        const afterClick = await page.textContent('#card-details-body');
        check('clicking a graveyard card updates the Card Details pane',
              afterClick !== beforeClick,
              `details changed (length ${beforeClick.length} → ${afterClick.length})`);

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
