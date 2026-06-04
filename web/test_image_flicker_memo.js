#!/usr/bin/env node
/**
 * Regression test for mtg-j1ka3 — the FLICKERING opponent card.
 *
 * BUG: `renderBattlefield` rebuilds `container.innerHTML` on every processed
 * network message (updateUI is scheduled per-message), destroying + recreating
 * each card <img> with src = urls[0] (the LOCAL path). For a card with no local
 * image — most OPPONENT cards — that first URL 404s, so the inline onerror
 * cascade walks local→CDN→gatherer, but the NEXT re-render wipes it and restarts
 * from the 404ing local URL before it settles → the card visibly FLICKERS.
 *
 * FIX (resolved-URL memo, native_game.html): once a card image successfully
 * loads, the URL that worked is memoized (keyed by its stable first-choice URL)
 * and served FIRST on later renders, so the recreated <img> paints from cache
 * instantly — the image settles to ONE stable state, no re-cascade, no flicker.
 *
 * Two layers of assertion:
 *   (A) DETERMINISTIC pure-logic: drive `applyImageMemo` / `recordImgResolved`
 *       directly (exposed on window.__mtg) and assert the reorder — empty memo →
 *       identity; after recording the resolved URL → it is served FIRST with the
 *       other sources kept as ordered fallbacks, no duplicates. This is the exact
 *       logic that kills the flicker and is independent of network/timing.
 *   (B) WIRING + STABILITY in a live game: battlefield <img>s carry the
 *       data-memokey plumbing, the live memo records resolved URLs as images
 *       load, and a card's <img src> is STABLE across many re-renders — it never
 *       reverts to the 404ing local memokey URL (the flicker).
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

        // Boot heuristic-vs-heuristic (seed 42 → never blocks on human input)
        // with local images ENABLED so the battlefield cascade is
        // [local, (CDN), gatherer] and local 404s in this hermetic env exactly
        // like an OPPONENT card the client has no local art for.
        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', {
            deck, p1: 'heuristic', p2: 'heuristic', seed: 42,
            extra: { allow_local_img_load: 'true' },
        }), { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout: 30000 });
        await page.waitForTimeout(2000);

        // ===== (A) DETERMINISTIC pure-logic assertions on applyImageMemo =====
        check('window.__mtg exposes the image-memo hooks',
              await page.evaluate(() => !!(window.__mtg && window.__mtg.applyImageMemo
                    && window.__mtg.recordImgResolved && window.__mtg.clearImageMemo)),
              'applyImageMemo / recordImgResolved / clearImageMemo present');

        const logic = await page.evaluate(() => {
            const m = window.__mtg;
            m.clearImageMemo();
            const L = '/images/small/x/Test Card.jpg';
            const C = 'https://cards.scryfall.io/small/front/0/0/00000000-0000-0000-0000-000000000000.jpg?1';
            const G = 'https://gatherer.wizards.com/Handlers/Image.ashx?name=Test%20Card&type=card';
            // Empty memo → identity order, key = first url.
            const r1 = m.applyImageMemo([L, C, G]);
            // Record the URL that actually loaded (gatherer here).
            m.recordImgResolved(L, G);
            const r2 = m.applyImageMemo([L, C, G]);
            // A different card with nothing memoized stays in original order.
            const r3 = m.applyImageMemo(['/images/small/y/Other.jpg', C]);
            m.clearImageMemo();
            return { r1, r2, r3, L, C, G };
        });

        check('empty memo → original order (key = first url)',
              JSON.stringify(logic.r1.urls) === JSON.stringify([logic.L, logic.C, logic.G])
                  && logic.r1.key === logic.L,
              `urls=${JSON.stringify(logic.r1.urls)} key=${logic.r1.key.slice(-30)}`);

        check('after recording resolved URL → it is served FIRST, rest kept as ordered fallbacks',
              logic.r2.urls[0] === logic.G
                  && JSON.stringify(logic.r2.urls) === JSON.stringify([logic.G, logic.L, logic.C]),
              `urls=${JSON.stringify(logic.r2.urls.map(u => u.slice(-24)))}`);

        check('reordered list has no duplicates and preserves length',
              logic.r2.urls.length === 3 && new Set(logic.r2.urls).size === 3,
              `len=${logic.r2.urls.length} unique=${new Set(logic.r2.urls).size}`);

        check('a different (unmemoized) card stays in original order',
              logic.r3.urls[0].startsWith('/images/'),
              `urls[0]=${logic.r3.urls[0].slice(-30)}`);

        // ===== (B) WIRING + STABILITY in a live game =====
        // Drive a few turns so each battlefield has permanents to render.
        for (let i = 0; i < 8; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(120);
        }
        await page.waitForTimeout(800);

        const wiring = await page.evaluate(() => {
            const imgs = [...document.querySelectorAll('#player-field-cards img.card-image, #opp-field-cards img.card-image')];
            const withMemoKey = imgs.filter(i => (i.getAttribute('data-memokey') || '').length > 0).length;
            // Sample (cardId -> src) for every battlefield card with an <img>.
            const sample = {};
            for (const grid of ['player-field-cards', 'opp-field-cards']) {
                const g = document.getElementById(grid);
                if (!g) continue;
                for (const c of g.querySelectorAll('.card')) {
                    const img = c.querySelector('img.card-image');
                    const id = c.getAttribute('data-card-id');
                    if (img && id) sample[id] = { src: img.getAttribute('src') || '', memokey: img.getAttribute('data-memokey') || '' };
                }
            }
            return { imgCount: imgs.length, withMemoKey, memoSize: window.__mtg.imageMemoSize(), sample };
        });

        check('battlefield <img>s render with the memo plumbing (data-memokey)',
              wiring.imgCount > 0 && wiring.withMemoKey === wiring.imgCount,
              `${wiring.withMemoKey}/${wiring.imgCount} imgs carry data-memokey`);

        check('the live image memo recorded resolved URLs as images loaded',
              wiring.memoSize > 0,
              `memo has ${wiring.memoSize} resolved entr${wiring.memoSize === 1 ? 'y' : 'ies'}`);

        // Force many re-renders (each ArrowDown/Up drives updateUI →
        // renderBattlefield, the exact path that re-creates the <img>).
        for (let i = 0; i < 8; i++) {
            await page.keyboard.press('ArrowDown');
            await page.waitForTimeout(60);
            await page.keyboard.press('ArrowUp');
            await page.waitForTimeout(60);
        }
        await page.waitForTimeout(400);

        const after = await page.evaluate(() => {
            const out = {};
            for (const grid of ['player-field-cards', 'opp-field-cards']) {
                const g = document.getElementById(grid);
                if (!g) continue;
                for (const c of g.querySelectorAll('.card')) {
                    const img = c.querySelector('img.card-image');
                    const id = c.getAttribute('data-card-id');
                    if (img && id) out[id] = img.getAttribute('src') || '';
                }
            }
            return out;
        });

        // For every card present both before and after, the src must be STABLE
        // and must NOT have reverted to the 404ing local memokey URL.
        let stableCount = 0, revertedToLocal = [];
        for (const [id, info] of Object.entries(wiring.sample)) {
            if (!(id in after)) continue;
            stableCount++;
            if (after[id] !== info.src) {
                revertedToLocal.push(`card ${id}: ${info.src.slice(-30)} → ${after[id].slice(-30)}`);
            } else if (info.memokey && after[id] === info.memokey && info.memokey.startsWith('/images/')) {
                // src equals the local memokey AND that's a /images/ path that
                // 404s — the flicker state. (Only a problem if it 404'd; if the
                // local image genuinely loaded, memokey==src is fine.)
                revertedToLocal.push(`card ${id}: stuck on local ${after[id].slice(-30)}`);
            }
        }
        check('battlefield <img src> is STABLE across 8 re-renders (no flicker)',
              stableCount > 0 && revertedToLocal.length === 0,
              stableCount > 0
                  ? (revertedToLocal.length === 0
                      ? `${stableCount} cards held a single settled src across re-renders`
                      : `unstable: ${revertedToLocal.slice(0, 3).join(' | ')}`)
                  : 'no comparable cards (battlefield empty?)');

        // No WASM panics / non-image errors (image 404s are expected + handled).
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
