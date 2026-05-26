#!/usr/bin/env node
/**
 * E2E test: Verify click events work (card details) and log panel has content.
 *
 * Uses LOCAL mode (human vs random AI) to avoid network complexity.
 * Advances the game through several turns, then verifies:
 * 1. Clicks on the hand area produce card details
 * 2. The log panel has game action content (draws, plays, etc.)
 *
 * Usage: node test_click_and_log.js
 */

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { getRandomPorts, enableReplayVerifier, checkForFatalErrors } = require('./test_network_utils');

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

(async () => {
    let httpServer, browser;
    const projectRoot = path.join(__dirname, '..');
    const screenshotDir = path.join(projectRoot, 'debug');
    let errors = [];

    // Allocate a random HTTP port to avoid conflicts
    const { httpPort: HTTP_PORT } = await getRandomPorts();

    try {
        fs.mkdirSync(screenshotDir, { recursive: true });
        log(`Using HTTP port: ${HTTP_PORT}`);

        // Start HTTP server
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: path.join(projectRoot, 'web'),
            stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(r => setTimeout(r, 1500));
        log('HTTP server started');

        // Launch browser
        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });

        // Capture console logs for diagnostics on failure
        const consoleLogs = [];
        page.on('console', msg => consoleLogs.push(msg.text()));
        page.on('pageerror', err => log(`PAGE ERROR: ${err.message}`));
        page.on('dialog', async dialog => {
            log(`DIALOG: ${dialog.message()}`);
            await dialog.accept();
        });

        // Navigate and wait for WASM
        await page.goto(`http://localhost:${HTTP_PORT}/tui_game.html`, {
            waitUntil: 'networkidle', timeout: 30000
        });
        await page.waitForSelector('#launcher.show', { state: 'attached', timeout: 30000 });
        log('Page loaded, WASM ready');

        // Enable rewind/replay verifier — this test exercises the human
        // controller path via simulated key presses, so the verifier WILL
        // run on each replayed choice. Catch any divergence as a hard fail
        // (REWIND/REPLAY FATAL pattern, see test_network_utils.js).
        const verifierEnabled = await enableReplayVerifier(page);
        log(`Replay verifier enabled: ${verifierEnabled}`);

        // Wait for deck dropdowns
        await page.waitForFunction(() => {
            const select = document.getElementById('p1-deck');
            return select && select.options.length > 0;
        }, { timeout: 10000 });

        // Select deck for both players
        const firstDeck = await page.evaluate(() => document.getElementById('p1-deck').options[0].value);
        await page.selectOption('#p1-deck', firstDeck);
        await page.selectOption('#p2-deck', firstDeck);

        // Use LOCAL mode: human P1, random P2
        await page.selectOption('#game-mode', 'local');
        await page.selectOption('#p1-controller', 'human');
        await page.selectOption('#p2-controller', 'random');

        log('Launching game...');
        await page.click('#btn-launch');
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 30000 });
        log('Terminal visible');

        // Advance the game through multiple turns by making choices
        // Press '0' (pass priority) and occasionally '1' (play land/cast)
        await page.waitForTimeout(3000);
        for (let i = 0; i < 30; i++) {
            const key = (i % 5 === 0) ? '1' : '0';
            await page.keyboard.press(key);
            await page.waitForTimeout(200);
        }
        await page.waitForTimeout(2000);
        log('Advanced game through multiple turns');

        // === TEST 1: Log panel has content ===
        const logStatus = await page.evaluate(() => {
            const terminal = document.getElementById('ratzilla-terminal');
            if (!terminal) return null;
            const text = terminal.textContent;
            // Extract log status like "1-15/15 [F]" from "Log (L) ──────1-15/15 [F]"
            const match = text.match(/Log.*?(\d+)-(\d+)\/(\d+)/);
            return match ? { start: parseInt(match[1]), end: parseInt(match[2]), total: parseInt(match[3]) } : null;
        });
        log(`Log panel: ${logStatus ? `${logStatus.total} entries` : 'NOT FOUND'}`);

        if (!logStatus || logStatus.total === 0) {
            errors.push(`Log panel has no entries (expected game actions after multiple turns)`);
        } else {
            log(`  PASS: Log has ${logStatus.total} entries`);
        }

        // === TEST 2: Click on hand area produces card details ===
        // Get card details text before click
        const detailsBefore = await page.evaluate(() => {
            const t = document.getElementById('ratzilla-terminal');
            if (!t) return '';
            const text = t.textContent;
            const idx = text.indexOf('Card Details');
            return idx >= 0 ? text.substring(idx, idx + 200) : '';
        });

        // Click across the hand area (bottom-right area of screen)
        const viewport = page.viewportSize();
        let clickWorked = false;
        for (let xFrac of [0.75, 0.8, 0.85, 0.9]) {
            const x = Math.floor(viewport.width * xFrac);
            const y = Math.floor(viewport.height * 0.6); // Hand area
            await page.mouse.click(x, y);
            await page.waitForTimeout(300);

            const detailsAfter = await page.evaluate(() => {
                const t = document.getElementById('ratzilla-terminal');
                if (!t) return '';
                const text = t.textContent;
                const idx = text.indexOf('Card Details');
                return idx >= 0 ? text.substring(idx, idx + 200) : '';
            });

            if (detailsAfter !== detailsBefore) {
                clickWorked = true;
                // Extract card name from details (first line after "Card Details")
                const nameMatch = detailsAfter.match(/Card Details[^│]*│\s*(.+?)(?:\s{2,}|\s*│)/);
                log(`  PASS: Click at x=${xFrac} changed card details${nameMatch ? `: ${nameMatch[1].trim()}` : ''}`);
                break;
            }
        }

        if (!clickWorked) {
            errors.push('No clicks on the hand area changed card details');
        }

        // Save screenshot for debugging
        await page.screenshot({ path: path.join(screenshotDir, 'click_test_final.png'), fullPage: true });

        // === TEST 3: No replay-verifier or desync fatals in console ===
        // checkForFatalErrors expects {text: ...} entries; map flat strings.
        const fatalLog = checkForFatalErrors(consoleLogs.map(text => ({ text })));
        if (fatalLog) {
            errors.push(`Fatal browser-log entry: ${fatalLog}`);
        } else {
            log('  PASS: No REWIND/REPLAY FATAL or DESYNC entries in console');
        }

        // === SUMMARY ===
        if (errors.length > 0) {
            // Write diagnostics on failure
            fs.writeFileSync(path.join(screenshotDir, 'click_test_console.txt'), consoleLogs.join('\n'));
            log(`\n=== FAILURES (${errors.length}) ===`);
            for (const e of errors) log(`  FAIL: ${e}`);
            process.exitCode = 1;
        } else {
            log('\n=== ALL TESTS PASSED ===');
        }

    } catch (e) {
        log(`ERROR: ${e.message}`);
        log(e.stack);
        process.exitCode = 1;
    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill();
        await new Promise(r => setTimeout(r, 500));
    }
})();
