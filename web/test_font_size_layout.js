// Diagnostic test for font-size regression on tui_game.html
// Measures: terminal-rendered-bounds vs viewport, and grid utilization
// Run with: node test_font_size_layout.js

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');
const { getRandomPorts } = require('./test_network_utils');

function log(m) {
    console.log(`[${new Date().toISOString()}] ${m}`);
}

async function measure(page) {
    return await page.evaluate(() => {
        const term = document.getElementById('ratzilla-terminal');
        const grid = document.getElementById('ratzilla-terminal_ratzilla_grid');
        const termRect = term ? term.getBoundingClientRect() : null;
        const gridRect = grid ? grid.getBoundingClientRect() : null;
        const cs = term ? getComputedStyle(term) : null;
        const sample = grid ? Array.from(grid.querySelectorAll('span')).find(s => s.textContent.trim()) : null;
        const sampleRect = sample ? sample.getBoundingClientRect() : null;
        const preEls = grid ? grid.querySelectorAll('pre') : [];
        const firstPreRect = preEls[0] ? preEls[0].getBoundingClientRect() : null;
        const lastPreRect = preEls[preEls.length - 1] ? preEls[preEls.length - 1].getBoundingClientRect() : null;
        return {
            innerWidth: window.innerWidth,
            innerHeight: window.innerHeight,
            termRect: termRect ? { w: termRect.width, h: termRect.height } : null,
            gridRect: gridRect ? { w: gridRect.width, h: gridRect.height } : null,
            cssFontSize: cs ? cs.fontSize : null,
            cssLineHeight: cs ? cs.lineHeight : null,
            sampleCellW: sampleRect ? sampleRect.width : null,
            sampleCellH: sampleRect ? sampleRect.height : null,
            preCount: preEls.length,
            firstPreW: firstPreRect ? firstPreRect.width : null,
            firstPreH: firstPreRect ? firstPreRect.height : null,
            gridFromBottom: gridRect ? (window.innerHeight - (gridRect.top + gridRect.height)) : null,
            gridFromRight: gridRect ? (window.innerWidth - (gridRect.left + gridRect.width)) : null,
            tuiCellW: window.TUICoordinateSystem ? window.TUICoordinateSystem.cellWidthPx : null,
            tuiCellH: window.TUICoordinateSystem ? window.TUICoordinateSystem.cellHeightPx : null,
        };
    });
}

async function checkViewport(browser, base, w, h) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    const page = await ctx.newPage();
    page.on('pageerror', e => log(`Page ERROR @${w}x${h}: ${e.message}`));
    try {
        // mtg-35z3s page 3: tui_game.html is a PURE renderer — boot a local
        // heuristic-vs-heuristic game from URL params (no launcher form).
        const firstDeck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'tui_game.html', {
            deck: firstDeck, p1: 'heuristic', p2: 'heuristic',
        }), { waitUntil: 'networkidle', timeout: 60000 });
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 30000 });
        await page.waitForSelector('#game-controls', { state: 'visible', timeout: 10000 });
        await page.waitForTimeout(800);
        const m = await measure(page);
        const widthFill = m.gridRect.w / m.innerWidth;
        const heightFill = m.gridRect.h / m.innerHeight;
        const ok = widthFill > 0.95 && heightFill > 0.95;
        log(`@${w}x${h}: grid=${m.gridRect.w}x${m.gridRect.h} fill=${widthFill.toFixed(3)}x${heightFill.toFixed(3)} cssFont=${m.cssFontSize} cssLH=${m.cssLineHeight} ${ok ? 'PASS' : 'FAIL'}`);
        if (!ok) {
            await page.screenshot({ path: path.join(__dirname, 'screenshots', `font_size_FAIL_${w}x${h}.png`), fullPage: true });
        }
        return ok;
    } finally {
        await ctx.close();
    }
}

async function runTest() {
    let server, browser;
    try {
        // EPHEMERAL port (not hardcoded): a fixed port collides with a concurrent
        // cross-worktree validate running the same browser test (ECONNREFUSED/
        // EADDRINUSE flakes). getRandomPorts() picks an available port below the
        // Linux ephemeral range (see test_network_utils).
        const { httpPort: HTTP_PORT } = await getRandomPorts();
        const base = `http://localhost:${HTTP_PORT}`;
        server = spawn('python3', ['-m', 'http.server', String(HTTP_PORT)], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(r => setTimeout(r, 1000));
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const sizes = [
            { w: 1280, h: 720 },
            { w: 1600, h: 900 },
            { w: 1920, h: 1080 },
            { w: 1024, h: 768 },
        ];
        let allPass = true;
        for (const s of sizes) {
            const ok = await checkViewport(browser, base, s.w, s.h);
            if (!ok) allPass = false;
        }
        log(allPass ? 'OVERALL PASS' : 'OVERALL FAIL');
        return allPass;
    } finally {
        if (browser) await browser.close();
        if (server) server.kill();
    }
}

runTest().then(ok => process.exit(ok ? 0 : 1)).catch(e => { console.error(e); process.exit(2); });
