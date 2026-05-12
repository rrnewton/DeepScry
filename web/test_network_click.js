#!/usr/bin/env node
/**
 * E2E test: Verify click events and log panel work in NETWORK mode.
 *
 * Starts a game server + native P2 client, connects browser as P1 (human).
 * Advances through several turns, then verifies:
 * 1. Before-click screenshot shows empty Card Details
 * 2. After-click screenshot shows populated Card Details
 * 3. Log panel has game action content
 *
 * Usage: node test_network_click.js
 */

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { getRandomPorts } = require('./test_network_utils');

// Configuration - ports allocated dynamically below
const SERVER_PASSWORD = 'clicktest';
const GAME_SEED = 42;
const DECK_NAME = '01_rogue_rogerbrand';

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

(async () => {
    let httpServer, server, nativeClient, browser;
    const projectRoot = path.join(__dirname, '..');
    const screenshotDir = path.join(projectRoot, 'debug');
    let errors = [];

    // Allocate random ports to avoid conflicts with other tests
    const { serverPort: SERVER_PORT, httpPort: HTTP_PORT } = await getRandomPorts();

    try {
        fs.mkdirSync(screenshotDir, { recursive: true });
        log(`Using ports: server=${SERVER_PORT}, http=${HTTP_PORT}`);

        // Check binary exists
        const mtgBinary = path.join(projectRoot, 'target', 'release', 'mtg');
        if (!fs.existsSync(mtgBinary)) {
            throw new Error('mtg binary not found. Run: cargo build --release --features network');
        }

        // Start HTTP server
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: __dirname, stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(r => setTimeout(r, 1000));
        log('HTTP server started');

        // Start MTG game server
        const deckPath = path.join(projectRoot, 'decks', 'old_school', DECK_NAME + '.dck');
        server = spawn(mtgBinary, [
            'server', '--port', SERVER_PORT.toString(),
            '--password', SERVER_PASSWORD,
            '--seed', GAME_SEED.toString(),
            '--network-debug',
        ], { cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe'] });

        let serverLogs = [];
        server.stdout.on('data', d => serverLogs.push(d.toString()));
        server.stderr.on('data', d => serverLogs.push(d.toString()));

        // Wait for server to be ready. Connect via 127.0.0.1 (not localhost)
        // to avoid Node 17+ Happy-Eyeballs picking IPv6 ::1, which the
        // server doesn't bind to. See test_network_utils.js for context.
        const WebSocket = require('ws');
        let serverReady = false;
        for (let i = 0; i < 20; i++) {
            try {
                const ws = new WebSocket(`ws://127.0.0.1:${SERVER_PORT}`);
                await new Promise((resolve, reject) => {
                    ws.on('open', () => { ws.close(); resolve(); });
                    ws.on('error', reject);
                    setTimeout(() => reject(new Error('timeout')), 1000);
                });
                serverReady = true;
                break;
            } catch { await new Promise(r => setTimeout(r, 500)); }
        }
        if (!serverReady) {
            // Surface server output so we can debug why it never came up.
            console.error('=== Server logs ===');
            console.error(serverLogs.join(''));
            throw new Error('Server failed to start');
        }
        log('Game server ready');

        // Start native client as P2 (heuristic AI) - pass many times to keep game going
        nativeClient = spawn(mtgBinary, [
            'connect', '--server', `localhost:${SERVER_PORT}`,
            '--password', SERVER_PASSWORD,
            '--name', 'TestAI',
            '--controller', 'heuristic',
            deckPath
        ], { cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe'] });

        let clientLogs = [];
        nativeClient.stdout.on('data', d => clientLogs.push(d.toString()));
        nativeClient.stderr.on('data', d => clientLogs.push(d.toString()));
        await new Promise(r => setTimeout(r, 2000));
        log('Native P2 client connected');

        // Launch browser
        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });

        const consoleLogs = [];
        page.on('console', msg => consoleLogs.push(msg.text()));
        page.on('pageerror', err => log(`PAGE ERROR: ${err.message}`));
        page.on('dialog', async dialog => {
            log(`DIALOG: ${dialog.message()}`);
            await dialog.accept();
        });

        // Navigate to fancy.html
        await page.goto(`http://localhost:${HTTP_PORT}/fancy.html`, {
            waitUntil: 'networkidle', timeout: 30000
        });
        await page.waitForSelector('#launcher.show', { state: 'attached', timeout: 30000 });
        log('Page loaded, WASM ready');

        // Wait for deck dropdowns
        await page.waitForFunction(() => {
            const select = document.getElementById('p1-deck');
            return select && select.options.length > 0;
        }, { timeout: 10000 });

        // Configure NETWORK mode
        await page.selectOption('#game-mode', 'network');
        // Set server connection details (field IDs: server-url, server-password, player-name)
        await page.evaluate(({port, password}) => {
            document.getElementById('server-url').value = `ws://localhost:${port}`;
            document.getElementById('server-password').value = password;
            document.getElementById('player-name').value = 'TestP1';
        }, {port: SERVER_PORT, password: SERVER_PASSWORD});

        // Select deck and controller
        const deckOptions = await page.evaluate(() => {
            const select = document.getElementById('p1-deck');
            return Array.from(select.options).map(o => o.value);
        });
        const targetDeck = deckOptions.find(d => d.includes('rogue')) || deckOptions[0];
        await page.selectOption('#p1-deck', targetDeck);
        await page.selectOption('#p1-controller', 'human');
        log(`Selected deck: ${targetDeck}`);

        // Launch game
        log('Clicking Launch...');
        await page.click('#btn-launch');

        // Wait for terminal to appear (game connected and started)
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 60000 });
        log('Terminal visible - game started');

        // Wait for game to settle and AI to take actions
        await page.waitForTimeout(5000);

        // Advance game by making choices (pass priority several times)
        for (let i = 0; i < 20; i++) {
            await page.keyboard.press('0');
            await page.waitForTimeout(300);
        }
        await page.waitForTimeout(3000);
        log('Made 20 choices to advance game');

        // === SCREENSHOT 1: BEFORE CLICK ===
        // Get Card Details text BEFORE any click
        const detailsBefore = await page.evaluate(() => {
            const t = document.getElementById('ratzilla-terminal');
            if (!t) return '';
            const text = t.textContent;
            const idx = text.indexOf('Card Details');
            return idx >= 0 ? text.substring(idx, idx + 300) : '';
        });
        await page.screenshot({
            path: path.join(screenshotDir, 'network_click_BEFORE.png'), fullPage: true
        });
        log(`BEFORE click - Card Details: "${detailsBefore.substring(0, 80)}..."`);

        // === CLICK ON HAND CARDS ===
        const viewport = page.viewportSize();
        let clickWorked = false;
        let detailsAfter = '';

        for (let xFrac of [0.75, 0.8, 0.85, 0.9]) {
            const x = Math.floor(viewport.width * xFrac);
            const y = Math.floor(viewport.height * 0.6);
            await page.mouse.click(x, y);
            await page.waitForTimeout(500);

            detailsAfter = await page.evaluate(() => {
                const t = document.getElementById('ratzilla-terminal');
                if (!t) return '';
                const text = t.textContent;
                const idx = text.indexOf('Card Details');
                return idx >= 0 ? text.substring(idx, idx + 300) : '';
            });

            if (detailsAfter !== detailsBefore) {
                clickWorked = true;
                log(`Click at x=${xFrac} changed Card Details`);
                break;
            }
        }

        // === SCREENSHOT 2: AFTER CLICK ===
        await page.screenshot({
            path: path.join(screenshotDir, 'network_click_AFTER.png'), fullPage: true
        });
        log(`AFTER click - Card Details: "${detailsAfter.substring(0, 80)}..."`);

        // === TEST 1: Click changed Card Details ===
        if (clickWorked) {
            log('  PASS: Click events work - Card Details populated');
        } else {
            errors.push('Click events did NOT change Card Details in network mode');
            log('  FAIL: Click events did not change Card Details');
        }

        // === TEST 2: Log panel has content ===
        const logStatus = await page.evaluate(() => {
            const terminal = document.getElementById('ratzilla-terminal');
            if (!terminal) return null;
            const text = terminal.textContent;
            const match = text.match(/Log.*?(\d+)-(\d+)\/(\d+)/);
            return match ? { start: parseInt(match[1]), end: parseInt(match[2]), total: parseInt(match[3]) } : null;
        });

        if (logStatus && logStatus.total > 0) {
            log(`  PASS: Log panel has ${logStatus.total} entries`);
        } else {
            errors.push(`Log panel empty in network mode (${JSON.stringify(logStatus)})`);
            log(`  FAIL: Log panel has no entries`);
        }

        // === SUMMARY ===
        if (errors.length > 0) {
            fs.writeFileSync(path.join(screenshotDir, 'network_click_console.txt'), consoleLogs.join('\n'));
            fs.writeFileSync(path.join(screenshotDir, 'network_click_server.txt'), serverLogs.join(''));
            fs.writeFileSync(path.join(screenshotDir, 'network_click_client.txt'), clientLogs.join(''));
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
        if (nativeClient) nativeClient.kill();
        if (server) server.kill();
        if (httpServer) httpServer.kill();
        await new Promise(r => setTimeout(r, 500));
    }
})();
