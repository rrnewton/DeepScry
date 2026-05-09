// Diagnostic test for font-size regression on fancy.html
// Measures: terminal-rendered-bounds vs viewport, and grid utilization
// Run with: node test_font_size_layout.js

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');

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

async function checkViewport(browser, w, h) {
    const ctx = await browser.newContext({ viewport: { width: w, height: h } });
    const page = await ctx.newPage();
    page.on('pageerror', e => log(`Page ERROR @${w}x${h}: ${e.message}`));
    try {
        await page.goto('http://localhost:8767/fancy.html', { waitUntil: 'networkidle', timeout: 60000 });
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
        const firstDeck = await page.evaluate(() => document.getElementById('p1-deck')?.options[0]?.value || '');
        if (firstDeck) {
            await page.evaluate(d => {
                document.getElementById('p1-deck').value = d;
                document.getElementById('p2-deck').value = d;
            }, firstDeck);
        }
        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');
        await page.click('#btn-launch');
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 10000 });
        await page.waitForSelector('#game-controls', { state: 'visible', timeout: 10000 });
        await page.waitForTimeout(800);
        const m = await measure(page);
        // NOTE: with the font-scale shim active, window.innerWidth returns (real / scale),
        // so we compare grid bounds to the REAL viewport (the one we passed to Playwright).
        const widthFill = m.gridRect.w / w;
        const heightFill = m.gridRect.h / h;
        const ok = widthFill > 0.95 && heightFill > 0.95;
        log(`@${w}x${h}: grid=${m.gridRect.w}x${m.gridRect.h} fill=${widthFill.toFixed(3)}x${heightFill.toFixed(3)} shimInner=${m.innerWidth}x${m.innerHeight} cssFont=${m.cssFontSize} cssLH=${m.cssLineHeight} ${ok ? 'PASS' : 'FAIL'}`);
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
        server = spawn('python3', ['-m', 'http.server', '8767'], {
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
            const ok = await checkViewport(browser, s.w, s.h);
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
