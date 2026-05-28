#!/usr/bin/env node
/**
 * Regression test for the native_game.html battlefield layout (mtg-433 /
 * mtg-572).
 *
 * BUG (before fix): the battlefield container `.card-grid` used
 * `flex-direction: column`, so every view-model section
 * (Creatures | Artifacts | Lands) was forced onto its OWN full-width row.
 * A board with a couple of small sections (1-3 cards each) wasted most of
 * the vertical space — one section per row — while the Web TUI
 * (tui_game.html) rendered the SAME view-model sections densely.
 *
 * FIX: `.card-grid` is now a wrapping flex ROW (`flex-wrap: wrap`) and each
 * `.bf-section` is a content-sized flex item, so multiple small sections
 * pack side by side and only wrap to a new row when they genuinely don't
 * fit. Card sizes are still chosen by the shared Rust layout engine
 * (battlefield_layout::pick_card_size_for_battlefield) measured against the
 * stable `.pane-body` rect, and the section grouping/order still comes from
 * the shared Rust view model (gui_view_model.rs) — so there is ONE source
 * of truth for both surfaces; this is a pure CSS flow change.
 *
 * Approach: launch a heuristic-vs-heuristic Jeskai-aggro game at a seed that
 * reliably produces a board with >=2 small battlefield sections, then assert:
 *   1. `.card-grid` flows as a wrapping row (not a column).
 *   2. When a battlefield has >=2 sections AND those sections are small
 *      enough to fit, at least two of them SHARE a row (i.e. distinct
 *      section `offsetTop`s < section count). This is exactly what the old
 *      column layout could never do.
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

        await page.goto(`http://localhost:${HTTP_PORT}/native_game.html?allow_local_img_load=false`, {
            waitUntil: 'networkidle',
            timeout: 30000,
        });
        await page.waitForFunction(() => {
            const s = document.getElementById('p1-deck');
            return s && s.options.length > 0;
        }, { timeout: 30000 });

        // Jeskai aggro deck reliably puts both a creature and a few lands into
        // play within ~40 ticks; falls back to the first deck if unavailable.
        const deck = await page.evaluate(() => {
            const opts = [...document.getElementById('p1-deck').options].map(o => o.value);
            return opts.find(o => /jeskai_aggro/.test(o)) || opts[0];
        });
        await page.selectOption('#p1-deck', deck);
        await page.selectOption('#p2-deck', deck);
        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');
        await page.fill('#game-seed', '7');
        await page.click('#btn-launch');
        await page.waitForTimeout(1500);

        // Drive the game so both battlefields have a handful of permanents
        // across multiple categories.
        for (let i = 0; i < 40; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(60);
        }
        await page.waitForTimeout(500);

        const sample = async () => page.evaluate(() => {
            const out = {};
            for (const id of ['player-field-cards', 'opp-field-cards']) {
                const grid = document.getElementById(id);
                if (!grid) { out[id] = null; continue; }
                const secs = [...grid.querySelectorAll('.bf-section')];
                const tops = secs.map(s => s.offsetTop);
                out[id] = {
                    flexDirection: getComputedStyle(grid).flexDirection,
                    flexWrap: getComputedStyle(grid).flexWrap,
                    sectionCount: secs.length,
                    distinctRows: new Set(tops).size,
                    labels: secs.map(s => s.querySelector('.bf-section-label')?.textContent || ''),
                };
            }
            return out;
        });

        const s = await sample();
        for (const id of ['player-field-cards', 'opp-field-cards']) {
            const m = s[id];
            if (!m) { check(`${id} present`, false, 'grid element missing'); continue; }
            log(`${id}: dir=${m.flexDirection} wrap=${m.flexWrap} ` +
                `sections=${m.sectionCount} rows=${m.distinctRows} labels=${JSON.stringify(m.labels)}`);

            // (1) The container must flow as a wrapping row — the structural
            // change that lets sections pack horizontally. The old column
            // layout (the bug) would report "column".
            check(`${id} card-grid is a wrapping row`,
                  m.flexDirection === 'row' && m.flexWrap === 'wrap',
                  `flexDirection=${m.flexDirection}, flexWrap=${m.flexWrap}`);
        }

        // (2) On at least one battlefield, two or more small sections must
        // SHARE a row. With the old `flex-direction: column` this was
        // impossible (distinctRows always == sectionCount). We require the
        // condition on whichever battlefield actually has multiple sections
        // this game.
        const multiSec = ['player-field-cards', 'opp-field-cards']
            .map(id => s[id])
            .filter(m => m && m.sectionCount >= 2);
        check('at least one battlefield has >=2 sections to exercise packing',
              multiSec.length >= 1,
              `multi-section battlefields=${multiSec.length}`);
        if (multiSec.length >= 1) {
            const packed = multiSec.some(m => m.distinctRows < m.sectionCount);
            check('multiple sections share a row (dense horizontal packing)',
                  packed,
                  multiSec.map(m => `sections=${m.sectionCount}/rows=${m.distinctRows}`).join(', '));
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
