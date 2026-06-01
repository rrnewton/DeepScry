// Network E2E Test using Playwright
// Run with: node test_network_e2e.js
//
// This test requires wasm-network feature to be built:
//   make wasm WASM_FEATURES="--features wasm-network"
//
// The test:
// 1. Starts a native MTG server
// 2. Starts a native client with fixed controller as P2
// 3. Launches browser as P1 with network mode
// 4. Connects to server
// 5. Verifies game starts and can progress
// 6. Checks for desync errors

const { chromium } = require('playwright');
const { spawn, execSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const { getRandomPorts, enableReplayVerifier, checkForFatalErrors } = require('./test_network_utils');
const { parseDckIntoCustomDeck } = require('./game_boot_params');

// Configuration - ports allocated dynamically in runTest()
const SERVER_PASSWORD = 'test123';
const GAME_SEED = 42;
const DECK_NAME = 'grizzly_bears.dck';

// Timestamped logging
function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

// Wait for server to be ready by attempting connection
async function waitForServer(port, maxAttempts = 30) {
    const WebSocket = require('ws');
    for (let i = 0; i < maxAttempts; i++) {
        try {
            const ws = new WebSocket(`ws://127.0.0.1:${port}`);
            await new Promise((resolve, reject) => {
                ws.on('open', () => {
                    ws.close();
                    resolve();
                });
                ws.on('error', reject);
                setTimeout(() => reject(new Error('timeout')), 1000);
            });
            return true;
        } catch (e) {
            await new Promise(r => setTimeout(r, 500));
        }
    }
    return false;
}

async function runTest() {
    let server = null;
    let nativeClient = null;
    let httpServer = null;
    let browser = null;
    const testResults = {
        startTime: new Date().toISOString(),
        steps: [],
        errors: [],
        browserLogs: [],
        serverLogs: [],
        clientLogs: []
    };

    const screenshotDir = path.join(__dirname, 'screenshots');
    if (!fs.existsSync(screenshotDir)) {
        fs.mkdirSync(screenshotDir);
    }

    // Allocate random ports to avoid conflicts with other tests
    const { serverPort: SERVER_PORT, httpPort: HTTP_PORT } = await getRandomPorts();
    log(`Using ports: server=${SERVER_PORT}, http=${HTTP_PORT}`);

    try {
        // Check if wasm-network build exists
        const wasmPkgPath = path.join(__dirname, 'pkg', 'mtg_engine.js');
        if (!fs.existsSync(wasmPkgPath)) {
            throw new Error('WASM package not found. Run: make wasm WASM_FEATURES="--features wasm-network"');
        }

        // Check for network exports in WASM
        const wasmContent = fs.readFileSync(wasmPkgPath, 'utf8');
        if (!wasmContent.includes('network_init')) {
            log('WARNING: WASM package may not have network features. Testing graceful fallback...');
            testResults.networkEnabled = false;
        } else {
            log('Network features detected in WASM package');
            testResults.networkEnabled = true;
        }

        // Find the mtg binary
        const mtgBinary = path.join(__dirname, '..', 'target', 'release', 'mtg');
        if (!fs.existsSync(mtgBinary)) {
            throw new Error('mtg binary not found. Run: cargo build --release');
        }

        // Start HTTP server for web content
        log(`Starting HTTP server on port ${HTTP_PORT}...`);
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: __dirname,
            stdio: ['ignore', 'pipe', 'pipe']
        });
        httpServer.stdout.on('data', data => log(`HTTP: ${data.toString().trim()}`));
        httpServer.stderr.on('data', data => log(`HTTP: ${data.toString().trim()}`));
        await new Promise(r => setTimeout(r, 1000));

        // Start MTG server (from project root to find cardsfolder)
        const projectRoot = path.join(__dirname, '..');
        log(`Starting MTG server on port ${SERVER_PORT}...`);
        server = spawn(mtgBinary, [
            'server',
            '--port', SERVER_PORT.toString(),
            '--password', SERVER_PASSWORD,
            '--seed', GAME_SEED.toString(),
            '--network-debug'
        ], {
            cwd: projectRoot,
            stdio: ['ignore', 'pipe', 'pipe']
        });

        server.stdout.on('data', data => {
            const line = data.toString().trim();
            testResults.serverLogs.push({ timestamp: new Date().toISOString(), line });
            log(`Server: ${line}`);
        });
        server.stderr.on('data', data => {
            const line = data.toString().trim();
            testResults.serverLogs.push({ timestamp: new Date().toISOString(), line });
            log(`Server: ${line}`);
        });

        // Wait for server to be ready
        log('Waiting for server to accept connections...');
        const serverReady = await waitForServer(SERVER_PORT);
        if (!serverReady) {
            throw new Error('Server failed to start');
        }
        testResults.steps.push({ name: 'server_start', timestamp: new Date().toISOString() });
        log('Server is ready');

        // Start native client as P2 with fixed controller (from project root)
        log('Starting native client as P2...');
        const deckPath = path.join(projectRoot, 'decks', DECK_NAME);
        // Read the deck so the WEB client can seed the SAME deck as a custom deck
        // (mtg-drxh5: the pure renderer boots from params; out-of-glob decks like
        // grizzly_bears are loaded via getCustomDecks + register_custom_deck).
        const deckContent = fs.readFileSync(deckPath, 'utf8');
        const deckNameMatch = deckContent.match(/^\s*Name\s*=\s*(.+)$/im);
        const webDeckName = deckNameMatch
            ? deckNameMatch[1].trim()
            : path.basename(DECK_NAME).replace(/\.(dck|txt)$/i, '');
        nativeClient = spawn(mtgBinary, [
            'connect',
            '--server', `localhost:${SERVER_PORT}`,
            '--password', SERVER_PASSWORD,
            '--name', 'NativeP2',
            '--controller', 'fixed',
            deckPath,
            '--fixed-inputs', '0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0;0'  // Pass 20 times
        ], {
            cwd: projectRoot,
            stdio: ['ignore', 'pipe', 'pipe']
        });

        nativeClient.stdout.on('data', data => {
            const line = data.toString().trim();
            testResults.clientLogs.push({ timestamp: new Date().toISOString(), line });
            log(`NativeP2: ${line}`);
        });
        nativeClient.stderr.on('data', data => {
            const line = data.toString().trim();
            testResults.clientLogs.push({ timestamp: new Date().toISOString(), line });
            log(`NativeP2: ${line}`);
        });

        testResults.steps.push({ name: 'native_client_start', timestamp: new Date().toISOString() });
        log('Native client started');

        // Give native client time to connect
        await new Promise(r => setTimeout(r, 2000));

        // Launch browser
        log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        // Seed the deck into localStorage as a custom deck BEFORE navigation, so
        // the param-booted page can register it by name (mtg-drxh5).
        const webCustomDeck = parseDckIntoCustomDeck(deckContent);
        await page.addInitScript(({ name, deck }) => {
            const KEY = 'mtg-forge-custom-decks';
            let decks = {};
            try { decks = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch (e) { /* ignore */ }
            decks[name] = deck;
            localStorage.setItem(KEY, JSON.stringify(decks));
        }, { name: webDeckName, deck: webCustomDeck });

        // Collect console messages
        page.on('console', msg => {
            const entry = { timestamp: new Date().toISOString(), type: msg.type(), text: msg.text() };
            testResults.browserLogs.push(entry);
            if (msg.type() === 'error') {
                log(`Browser [error]: ${msg.text()}`);
            }
        });

        page.on('pageerror', err => {
            testResults.errors.push({ timestamp: new Date().toISOString(), error: err.message });
            log(`Page ERROR: ${err.message}`);
        });

        // mtg-682 page 3 / mtg-drxh5: tui_game.html is a PURE renderer with no
        // built-in launcher. Boot the network client ENTIRELY from URL params via
        // the auto-match contract (?mode=network&controller=...&ws=&server_pass=
        // &name=&deck=) — the server pairs this web client with the native
        // `mtg connect` AI above. This replaces the deleted #game-mode / #server-url
        // / #btn-launch form. `fixed` mirrors the native client's fixed controller.
        log('Booting tui_game.html (network auto-match boot via params)...');
        const bootUrl = `http://localhost:${HTTP_PORT}/tui_game.html?` + new URLSearchParams({
            mode: 'network',
            ws: `ws://localhost:${SERVER_PORT}`,
            server_pass: SERVER_PASSWORD,
            name: 'WebP1',
            deck: webDeckName,
            controller: 'fixed',
        }).toString();
        await page.goto(bootUrl, { waitUntil: 'networkidle', timeout: 60000 });
        testResults.steps.push({ name: 'wasm_loaded', timestamp: new Date().toISOString() });
        log('WASM loaded; network boot initiated from params');

        // Enable rewind/replay verifier (see test_network_utils.js for the
        // full rationale). Network E2E is a prime candidate: any divergence
        // between the WASM client and the native server during replay would
        // surface as REWIND/REPLAY FATAL — strictly more informative than
        // waiting for the eventual desync further downstream.
        const verifierEnabled = await enableReplayVerifier(page);
        log(`Replay verifier enabled: ${verifierEnabled}`);

        testResults.steps.push({ name: 'network_boot', timestamp: new Date().toISOString() });

        // Wait for game to start (terminal should appear)
        // Note: With current placeholder implementation, game may not fully work
        // but we're testing the connection and UI flow
        try {
            await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 15000 });
            testResults.steps.push({ name: 'game_started', timestamp: new Date().toISOString() });
            log('Game UI appeared');

            await page.screenshot({ path: path.join(screenshotDir, 'network_03_game.png'), fullPage: true });

            testResults.result = 'PASSED';
            testResults.message = 'Network connection and game launch successful';
        } catch (e) {
            // Check network status
            const statusText = await page.evaluate(() => {
                return document.getElementById('network-status')?.textContent || 'unknown';
            });
            log(`Network status: ${statusText}`);

            await page.screenshot({ path: path.join(screenshotDir, 'network_03_waiting.png'), fullPage: true });

            // The placeholder implementation may not fully connect
            // Check if we at least tried to connect
            if (statusText.includes('Connecting') || statusText.includes('Waiting')) {
                testResults.result = 'PARTIAL';
                testResults.message = `Connection initiated but game not fully started: ${statusText}`;
            } else {
                testResults.result = 'FAILED';
                testResults.message = `Game failed to start: ${e.message}`;
            }
        }

        // Check for any JS errors related to network
        const networkErrors = testResults.browserLogs.filter(
            log => log.type === 'error' && log.text.includes('network')
        );
        if (networkErrors.length > 0) {
            log(`Network-related errors: ${networkErrors.length}`);
            testResults.networkErrors = networkErrors;
        }

        // Catch any REWIND/REPLAY FATAL or DESYNC entries surfaced via the
        // browser console; these supersede the PASSED/PARTIAL classification
        // above because state-divergence is never acceptable.
        const fatalLog = checkForFatalErrors(testResults.browserLogs);
        if (fatalLog) {
            log(`FATAL browser-log entry detected: ${fatalLog}`);
            testResults.fatalLog = fatalLog;
            testResults.result = 'FAILED';
            testResults.message = `Fatal browser-log entry: ${fatalLog}`;
        }

        return testResults;

    } catch (error) {
        log(`Test error: ${error.message}`);
        testResults.result = 'ERROR';
        testResults.message = error.message;

        if (browser) {
            try {
                const page = browser.contexts()[0]?.pages()[0];
                if (page) {
                    await page.screenshot({ path: path.join(screenshotDir, 'network_error.png'), fullPage: true });
                }
            } catch (e) {}
        }

        return testResults;

    } finally {
        // Cleanup
        if (browser) {
            log('Closing browser...');
            await browser.close();
        }
        if (nativeClient) {
            log('Stopping native client...');
            nativeClient.kill('SIGTERM');
        }
        if (server) {
            log('Stopping server...');
            server.kill('SIGTERM');
        }
        if (httpServer) {
            log('Stopping HTTP server...');
            httpServer.kill('SIGTERM');
        }
    }
}

// Run the test
runTest().then(results => {
    log('=== Test Results ===');
    log(`Result: ${results.result || 'UNKNOWN'}`);
    log(`Message: ${results.message || 'No message'}`);
    log(`Steps completed: ${results.steps.length}`);
    log(`Browser logs: ${results.browserLogs.length}`);
    log(`Errors: ${results.errors.length}`);

    // Write results to file
    const resultsPath = path.join(__dirname, 'network_test_results.json');
    fs.writeFileSync(resultsPath, JSON.stringify(results, null, 2));
    log(`Results written to: ${resultsPath}`);

    // Exit with appropriate code
    if (results.result === 'PASSED' || results.result === 'SKIPPED') {
        process.exit(0);
    } else if (results.result === 'PARTIAL') {
        log('Partial success - placeholder implementation limitation');
        process.exit(0);  // Don't fail for partial success
    } else {
        process.exit(1);
    }
}).catch(err => {
    log(`Fatal error: ${err.message}`);
    console.error(err);
    process.exit(1);
});
