#!/usr/bin/env node
/**
 * E2E test for bug-gamehtml-tapped-rotation: tapped cards in
 * `web/game.html` rotate a full 90°, matching `web/tui_game.html`'s
 * `TUICoordinateSystem.getCardBox(isTapped=true)` and the native
 * ratatui TUI's `battlefield_layout::entity_size` (1.5× wider, 0.6×
 * shorter — landscape orientation).
 *
 * Pre-fix: `.card.tapped { transform: rotate(8deg); }` — a faint
 * "tilt" that did NOT match the other two renderers. The shared
 * battlefield layout engine already swaps width/height for tapped
 * cards (entity_size at battlefield_layout.rs:672-683), but the CSS
 * was visually pretending tapped cards stayed portrait.
 *
 * Test setup: heuristic-vs-heuristic, seed 42, drive into a state
 * where at least one creature has attacked (and is therefore tapped).
 * Then read the `.card.tapped` element's computed `transform` and
 * `width`/`height` and assert the new rule is in effect.
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

        await page.goto(`http://localhost:${HTTP_PORT}/game.html`, {
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

        // Drive enough turns that at least one creature attacks (and
        // therefore taps). Heuristic AI almost always attacks in the
        // early/mid game.
        for (let i = 0; i < 30; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(80);
        }
        await page.waitForTimeout(500);

        // Wait for at least one .card.tapped to appear in the DOM.
        try {
            await page.waitForSelector('.card.tapped', { timeout: 10000 });
        } catch (_) {
            check('found at least one tapped card after 30 turns',
                  false,
                  'No .card.tapped element appeared (heuristic AI may have stalled)');
            throw new Error('no tapped card found');
        }

        const tappedInfo = await page.evaluate(() => {
            const els = document.querySelectorAll('.card.tapped');
            const sample = els[0];
            const cs = sample ? getComputedStyle(sample) : null;
            const grid = sample?.closest('.card-grid');
            const gridCs = grid ? getComputedStyle(grid) : null;
            const gridCardW = gridCs?.getPropertyValue('--card-w').trim() || '';
            const gridCardH = gridCs?.getPropertyValue('--card-h').trim() || '';

            // matrix(a, b, c, d, e, f) — for rotate(angle), a=cos, b=sin.
            // angle = atan2(b, a) in radians; convert to degrees.
            let rotationDeg = null;
            if (cs?.transform && cs.transform.startsWith('matrix(')) {
                const parts = cs.transform.slice(7, -1).split(',').map(s => parseFloat(s.trim()));
                if (parts.length >= 4) {
                    const angle = Math.atan2(parts[1], parts[0]) * 180 / Math.PI;
                    rotationDeg = Math.round(angle);
                }
            }

            return {
                tappedCount: els.length,
                tappedTransform: cs?.transform || '',
                tappedWidth: cs?.width || '',
                tappedHeight: cs?.height || '',
                rotationDeg,
                gridCardW,
                gridCardH,
                tappedCardName: sample?.getAttribute('data-card-name') || '',
            };
        });

        log(`Found ${tappedInfo.tappedCount} tapped card(s). First: ${tappedInfo.tappedCardName}`);
        log(`  transform=${tappedInfo.tappedTransform}`);
        log(`  width=${tappedInfo.tappedWidth}, height=${tappedInfo.tappedHeight}`);
        log(`  rotation parsed=${tappedInfo.rotationDeg}°`);
        log(`  grid --card-w=${tappedInfo.gridCardW}, --card-h=${tappedInfo.gridCardH}`);

        check('at least one tapped card visible',
              tappedInfo.tappedCount > 0,
              `${tappedInfo.tappedCount} tapped`);

        check('tapped card is rotated 90° (NOT 8°)',
              tappedInfo.rotationDeg === 90 || tappedInfo.rotationDeg === -90,
              `rotation=${tappedInfo.rotationDeg}° (was 8° pre-fix)`);

        // The CSS sets `.card.tapped { width: var(--card-h); }` so the
        // outer flex box reserves landscape space. Confirm the computed
        // tapped-card width matches the grid's --card-h, not --card-w.
        const tappedW = parseFloat(tappedInfo.tappedWidth);
        const gridH = parseFloat(tappedInfo.gridCardH);
        const gridW = parseFloat(tappedInfo.gridCardW);
        check('tapped card outer width = grid --card-h (landscape footprint)',
              Math.abs(tappedW - gridH) < 1,
              `tappedWidth=${tappedW}px, gridCardH=${gridH}px (gridCardW=${gridW}px for comparison)`);

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
