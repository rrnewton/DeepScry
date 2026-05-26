#!/usr/bin/env node
/**
 * Verifies the bug-gamehtml-card-size-v2 fixes:
 *
 * 1. Card sizes do NOT change incrementally on every interaction
 *    (the contracting feedback loop where measuring `.card-grid`
 *    instead of `.pane-body` was making cards shrink on every click).
 * 2. Cards FILL the available battlefield height (not stuck at
 *    min_card = 50x80).
 * 3. Cards SPREAD horizontally instead of clumping at the left edge.
 * 4. Battlefield <img>s request a height matching the rendered card
 *    height, not the hard-coded 106 — so they pull 'normal' images
 *    from `tui_get_image_urls` once cards are >204px tall.
 *
 * Approach: launch a heuristic-vs-heuristic seed-42 game, run a few
 * turns, then sample the computed `--card-w` / `--card-h` and the
 * <img src> on the battlefield, click around / press keys to trigger
 * many updateUI() invocations, and assert that the sizes are stable
 * across those interactions.
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
        page.on('console', msg => {
            if (msg.type() === 'error') browserErrors.push(`console.error: ${msg.text()}`);
        });

        await page.goto(`http://localhost:${HTTP_PORT}/native_game.html`, {
            waitUntil: 'networkidle',
            timeout: 30000,
        });
        await page.waitForFunction(() => {
            const s = document.getElementById('p1-deck');
            return s && s.options.length > 0;
        }, { timeout: 30000 });

        // Heuristic vs heuristic so the test never blocks waiting for human input.
        const deck = await page.evaluate(() => document.getElementById('p1-deck').options[0].value);
        await page.selectOption('#p1-deck', deck);
        await page.selectOption('#p2-deck', deck);
        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');
        await page.fill('#game-seed', '42');
        await page.click('#btn-launch');
        await page.waitForTimeout(2000);

        // Drive a few turns so each battlefield has a handful of permanents.
        for (let i = 0; i < 8; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(120);
        }
        await page.waitForTimeout(500);

        // Helper: sample current battlefield card sizes + an <img src>.
        const sampleSizes = async () => page.evaluate(() => {
            const yourGrid = document.getElementById('player-field-cards');
            const oppGrid = document.getElementById('opp-field-cards');
            const yourPane = document.querySelector('#pane-player-field .pane-body');
            const oppPane = document.querySelector('#pane-opp-field .pane-body');

            const yourCSS = yourGrid ? getComputedStyle(yourGrid) : null;
            const oppCSS = oppGrid ? getComputedStyle(oppGrid) : null;

            const yourImg = yourGrid?.querySelector('img.card-image');
            const oppImg = oppGrid?.querySelector('img.card-image');

            const yourCardCount = yourGrid?.querySelectorAll('.card').length || 0;
            const oppCardCount = oppGrid?.querySelectorAll('.card').length || 0;

            return {
                yourPaneH: yourPane?.clientHeight || 0,
                oppPaneH: oppPane?.clientHeight || 0,
                yourCardW: yourCSS?.getPropertyValue('--card-w').trim() || '',
                yourCardH: yourCSS?.getPropertyValue('--card-h').trim() || '',
                oppCardW: oppCSS?.getPropertyValue('--card-w').trim() || '',
                oppCardH: oppCSS?.getPropertyValue('--card-h').trim() || '',
                yourCardCount,
                oppCardCount,
                yourImgSrc: yourImg?.getAttribute('src') || '',
                oppImgSrc: oppImg?.getAttribute('src') || '',
                yourSectionsJustify: (() => {
                    const sec = yourGrid?.querySelector('.bf-section-cards');
                    return sec ? getComputedStyle(sec).justifyContent : '';
                })(),
            };
        });

        const initial = await sampleSizes();
        log(`After warm-up: yourPane=${initial.yourPaneH}px, your card=${initial.yourCardW}/${initial.yourCardH}, ${initial.yourCardCount} cards`);
        log(`              oppPane=${initial.oppPaneH}px, opp card=${initial.oppCardW}/${initial.oppCardH}, ${initial.oppCardCount} cards`);
        log(`              yourSectionsJustify=${initial.yourSectionsJustify}`);
        log(`              yourImgSrc=${initial.yourImgSrc.slice(-80)}`);

        check('battlefield card sizes were computed (--card-w set)',
              initial.yourCardW.endsWith('px') || initial.oppCardW.endsWith('px'),
              `yourCardW="${initial.yourCardW}", oppCardW="${initial.oppCardW}"`);

        check('cards are bigger than the min_card width (50px)',
              parseFloat(initial.yourCardW) > 60 || parseFloat(initial.oppCardW) > 60,
              `your=${initial.yourCardW}, opp=${initial.oppCardW}`);

        check('bf-section-cards uses centered/spread justify-content',
              ['center', 'space-around', 'space-between', 'space-evenly'].includes(initial.yourSectionsJustify),
              `justifyContent="${initial.yourSectionsJustify}" (was empty/flex-start before fix)`);

        // Card uses some non-trivial fraction of the pane height.
        const yourCardH = parseFloat(initial.yourCardH);
        const yourPaneH = initial.yourPaneH;
        if (yourCardH > 0 && yourPaneH > 0) {
            check('your card height fills at least 30% of pane height',
                  yourCardH >= yourPaneH * 0.30,
                  `cardH=${yourCardH}px / paneH=${yourPaneH}px = ${(100 * yourCardH / yourPaneH).toFixed(1)}%`);
        }

        // Image resolution: if cards are >204 tall, expect 'normal' folder.
        if (yourCardH > 204 && initial.yourImgSrc) {
            check('battlefield <img> uses /normal/ folder when card height > 204px',
                  initial.yourImgSrc.includes('/normal/') || !initial.yourImgSrc.startsWith('/images/'),
                  `imgSrc=${initial.yourImgSrc}`);
        } else if (yourCardH <= 204 && initial.yourImgSrc) {
            log(`(card height ${yourCardH}px ≤ 204 → 'small' folder is correct)`);
        }

        // ===== Stability test: trigger many updateUI calls and confirm sizes don't drift =====
        const samples = [initial];
        for (let i = 0; i < 6; i++) {
            await page.keyboard.press('ArrowDown');
            await page.waitForTimeout(80);
            await page.keyboard.press('ArrowUp');
            await page.waitForTimeout(80);
            samples.push(await sampleSizes());
        }

        const yourWidths = samples.map(s => s.yourCardW);
        const yourHeights = samples.map(s => s.yourCardH);
        const oppWidths = samples.map(s => s.oppCardW);
        const oppHeights = samples.map(s => s.oppCardH);

        const allSame = arr => arr.every(v => v === arr[0]);

        check('your --card-w stable across 6 interactions',
              allSame(yourWidths),
              `samples=${JSON.stringify(yourWidths)}`);
        check('your --card-h stable across 6 interactions',
              allSame(yourHeights),
              `samples=${JSON.stringify(yourHeights)}`);
        check('opp --card-w stable across 6 interactions',
              allSame(oppWidths),
              `samples=${JSON.stringify(oppWidths)}`);
        check('opp --card-h stable across 6 interactions',
              allSame(oppHeights),
              `samples=${JSON.stringify(oppHeights)}`);

        // Filter out the 404s for missing local card images — those are
        // EXPECTED when the local /images/ cache isn't fully populated;
        // the <img onerror> chain falls back to Scryfall (which works
        // and is also asserted via the imgSrc check above). We only
        // care about non-404 errors / WASM panics here.
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
