#!/usr/bin/env node
/**
 * E2E test for the graveyard listing in `web/native_game.html`.
 *
 * Layout (post gamehtml-graveyard-in-hand): the HAND is the critical
 * display and is always shown in full. The LOCAL player's graveyard is a
 * best-effort listing pinned to the BOTTOM of the Hand pane
 * (`#hand-graveyard`, inside `#pane-hand`). The OPPONENT — who has no hand
 * pane — keeps a battlefield graveyard overlay (`#opp-graveyard-overlay`).
 *
 * When the Hand pane lacks room for the whole graveyard, the listing shows
 * the most-recent K cards plus an `… N more cards` ellision line. The
 * top-K + ellision arithmetic is shared with the native ratatui TUI via the
 * Rust helper `graveyard_display::plan_graveyard_display`
 * (mirrored in JS as `graveyardDisplayPlan`).
 *
 * Screenshots (normal + ellision) are written to the gitignored `debug/`
 * directory for visual inspection; their paths are logged.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { getRandomPorts } = require('./test_network_utils');

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

        // Structural DOM: the local player's graveyard lives at the bottom
        // of the Hand pane; the opponent keeps a battlefield overlay.
        const initialDom = await page.evaluate(() => ({
            handGyPresent: !!document.getElementById('hand-graveyard'),
            handGyInHandPane: !!document.querySelector('#pane-hand > #hand-graveyard'),
            oppOverlayPresent: !!document.getElementById('opp-graveyard-overlay'),
            oppInsidePane: !!document.querySelector('#pane-opp-field > .graveyard-overlay'),
            // The OLD player battlefield overlay must be gone.
            playerBattlefieldOverlayGone: !document.getElementById('player-graveyard-overlay'),
        }));
        check('hand-graveyard element exists', initialDom.handGyPresent, 'in DOM');
        check('hand-graveyard is a child of #pane-hand (bottom of Hand pane)',
              initialDom.handGyInHandPane, `inHandPane=${initialDom.handGyInHandPane}`);
        check('opp-graveyard-overlay still exists on opponent battlefield',
              initialDom.oppOverlayPresent && initialDom.oppInsidePane,
              `present=${initialDom.oppOverlayPresent}, insidePane=${initialDom.oppInsidePane}`);
        check('old player-graveyard-overlay removed from battlefield',
              initialDom.playerBattlefieldOverlayGone,
              `gone=${initialDom.playerBattlefieldOverlayGone}`);

        // Drive enough turns that creatures die in combat → graveyard fills.
        for (let i = 0; i < 30; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(80);
        }
        await page.waitForTimeout(500);

        const afterPlay = await page.evaluate(() => {
            const handGy = document.getElementById('hand-graveyard');
            const oppOv = document.getElementById('opp-graveyard-overlay');

            const sample = (ov) => {
                if (!ov) return null;
                const cs = getComputedStyle(ov);
                const cards = Array.from(ov.querySelectorAll('.graveyard-overlay-card'));
                const headerEl = ov.querySelector('.graveyard-overlay-header');
                const ellisionEl = ov.querySelector('.hand-graveyard-ellision');
                return {
                    display: cs.display,
                    hasCardsClass: ov.classList.contains('has-cards'),
                    headerText: headerEl?.textContent || '',
                    cardCount: cards.length,
                    cardNames: cards.slice(0, 5).map(c => c.textContent.trim()),
                    ellisionText: ellisionEl?.textContent || '',
                };
            };

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
                hand: sample(handGy),
                opp: sample(oppOv),
                vmGyCounts,
            };
        });

        log(`After 30x Space:`);
        log(`  hand graveyard: display=${afterPlay.hand.display}, ${afterPlay.hand.cardCount} cards, header="${afterPlay.hand.headerText}", ellision="${afterPlay.hand.ellisionText}"`);
        log(`  opp overlay: display=${afterPlay.opp.display}, ${afterPlay.opp.cardCount} cards, header="${afterPlay.opp.headerText}"`);
        log(`  view model graveyard sizes: ${JSON.stringify(afterPlay.vmGyCounts)}`);

        const totalGyCards = afterPlay.hand.cardCount + afterPlay.opp.cardCount;
        check('at least one graveyard listing has cards after play',
              totalGyCards > 0,
              `hand=${afterPlay.hand.cardCount}, opp=${afterPlay.opp.cardCount}`);

        if (afterPlay.hand.cardCount > 0) {
            check('hand graveyard header includes count',
                  afterPlay.hand.headerText.includes('Graveyard') && /\d/.test(afterPlay.hand.headerText),
                  `header="${afterPlay.hand.headerText}"`);
        }

        // Screenshot the NORMAL case (full-window, with the hand-pane
        // graveyard visible at a comfortable height).
        const normalShot = path.join(debugDir, 'graveyard_hand_pane_normal.png');
        await page.screenshot({ path: normalShot, fullPage: false });
        log(`SCREENSHOT (normal): ${normalShot}`);

        // View-model parity: rendered player graveyard count should equal
        // shown + elided (i.e. the full PlayerView.graveyard.length).
        if (afterPlay.vmGyCounts && afterPlay.vmGyCounts.length === 2) {
            const expectedPlayer = afterPlay.vmGyCounts[0].gySize;
            // Parse "Graveyard (N):" → N (the true total) and elided count.
            const headerTotal = parseInt((afterPlay.hand.headerText.match(/\((\d+)\)/) || [])[1] || '0', 10);
            check('hand graveyard header total matches VM (PlayerView.graveyard)',
                  headerTotal === expectedPlayer,
                  `header total=${headerTotal}, VM=${expectedPlayer}`);
        }

        // Force the ELLISION case: shrink the viewport so the Hand pane is
        // short and the graveyard cannot show every card → "… N more cards".
        // We also need enough graveyard cards; if seed-42 hasn't filled
        // enough, keep playing a bit.
        await page.setViewportSize({ width: 1280, height: 380 });
        for (let i = 0; i < 20; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(60);
        }
        await page.waitForTimeout(400);

        const ellisionState = await page.evaluate(() => {
            const handGy = document.getElementById('hand-graveyard');
            const cards = Array.from(handGy.querySelectorAll('.graveyard-overlay-card'));
            const headerEl = handGy.querySelector('.graveyard-overlay-header');
            const ellisionEl = handGy.querySelector('.hand-graveyard-ellision');
            const headerTotal = parseInt((headerEl?.textContent.match(/\((\d+)\)/) || [])[1] || '0', 10);
            return {
                shown: cards.length,
                total: headerTotal,
                ellisionText: ellisionEl?.textContent || '',
                hasEllision: !!ellisionEl,
            };
        });
        log(`Ellision case (short window): total=${ellisionState.total}, shown=${ellisionState.shown}, ellision="${ellisionState.ellisionText}"`);

        const ellisionShot = path.join(debugDir, 'graveyard_hand_pane_ellision.png');
        await page.screenshot({ path: ellisionShot, fullPage: false });
        log(`SCREENSHOT (ellision): ${ellisionShot}`);

        // The ellision line only appears when total > shown. If the
        // graveyard is large enough (it should be after ~50 plies) the short
        // window MUST trigger ellision; assert the invariant when it does.
        if (ellisionState.total > ellisionState.shown) {
            check('ellision line shown when graveyard exceeds available rows',
                  ellisionState.hasEllision && /…\s*\d+\s*more card/.test(ellisionState.ellisionText),
                  `text="${ellisionState.ellisionText}"`);
            check('ellision count = total - shown',
                  (() => {
                      const n = parseInt((ellisionState.ellisionText.match(/(\d+)\s*more/) || [])[1] || '-1', 10);
                      return n === ellisionState.total - ellisionState.shown;
                  })(),
                  `text="${ellisionState.ellisionText}", total=${ellisionState.total}, shown=${ellisionState.shown}`);
        } else {
            log(`  (note: graveyard total ${ellisionState.total} <= shown ${ellisionState.shown}; ellision not triggered this run)`);
        }

        // Restore viewport and verify clicking a graveyard card updates
        // the Card Details pane (selection pipeline still wired).
        await page.setViewportSize({ width: 1280, height: 720 });
        await page.waitForTimeout(300);
        const clickTarget = (await page.locator('#hand-graveyard .graveyard-overlay-card').count()) > 0
            ? '#hand-graveyard .graveyard-overlay-card'
            : '#opp-graveyard-overlay .graveyard-overlay-card';
        if (await page.locator(clickTarget).count() > 0) {
            const beforeClick = await page.textContent('#card-details-body');
            await page.locator(clickTarget).first().click();
            await page.waitForTimeout(300);
            const afterClick = await page.textContent('#card-details-body');
            check('clicking a graveyard card updates the Card Details pane',
                  afterClick !== beforeClick,
                  `details changed (length ${beforeClick.length} → ${afterClick.length})`);
        }

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
