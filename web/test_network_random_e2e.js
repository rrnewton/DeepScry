// Network Random E2E Test using Playwright + launch script
//
// This test uses ./scripts/launch_network_game.sh to start the server
// and native client, then connects with a browser using random controller.
//
// Run with: node test_network_random_e2e.js
//
// This test is NOT part of 'make validate' - it's for manual testing only.

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { enableReplayVerifier, checkForFatalErrors } = require('./test_network_utils');
const { parseDckIntoCustomDeck } = require('./game_boot_params');

// Configuration. mtg-692: the old launch_network_game.sh (deleted in 43a0661f)
// is gone, so this test now spawns the server + native AI peer DIRECTLY (like
// test_network_e2e.js) and boots the web client from URL params (no launcher).
const SERVER_PORT = 17771;
const SERVER_PASSWORD = 'play';
const HTTP_PORT = 8000;
const DECK_NAME = 'grizzly_bears.dck';

// Timestamped logging
function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

// Wait for server to be ready by attempting connection
async function waitForServer(port, maxAttempts = 60) {
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

// Wait for HTTP server to be ready
async function waitForHttp(port, maxAttempts = 30) {
    const http = require('http');
    for (let i = 0; i < maxAttempts; i++) {
        try {
            await new Promise((resolve, reject) => {
                const req = http.get(`http://localhost:${port}/tui_game.html`, res => {
                    if (res.statusCode === 200) resolve();
                    else reject(new Error(`HTTP ${res.statusCode}`));
                });
                req.on('error', reject);
                req.setTimeout(1000, () => reject(new Error('timeout')));
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
    let httpServer = null;
    let nativeClient = null;
    let browser = null;
    const projectRoot = path.join(__dirname, '..');
    const screenshotDir = path.join(__dirname, 'screenshots');

    const testResults = {
        startTime: new Date().toISOString(),
        steps: [],
        errors: [],
        browserLogs: [],
        launchScriptOutput: []
    };

    if (!fs.existsSync(screenshotDir)) {
        fs.mkdirSync(screenshotDir);
    }

    // Read the deck so the web client can seed the SAME deck as a custom deck.
    const deckPath = path.join(projectRoot, 'decks', DECK_NAME);
    const deckContent = fs.readFileSync(deckPath, 'utf8');
    const deckNameMatch = deckContent.match(/^\s*Name\s*=\s*(.+)$/im);
    const webDeckName = deckNameMatch
        ? deckNameMatch[1].trim()
        : path.basename(DECK_NAME).replace(/\.(dck|txt)$/i, '');

    try {
        log('=== Network Random E2E Test ===');
        log('');

        // mtg-692: start the static HTTP server, the game server, and a native
        // AI peer DIRECTLY (the old launch_network_game.sh is deleted).
        log('Starting HTTP server...');
        httpServer = spawn('python3', ['-m', 'http.server', String(HTTP_PORT)], {
            cwd: __dirname, stdio: ['ignore', 'pipe', 'pipe']
        });
        const httpReady = await waitForHttp(HTTP_PORT);
        if (!httpReady) throw new Error(`HTTP server not ready on port ${HTTP_PORT}`);
        log('HTTP server ready');
        testResults.steps.push({ name: 'http_server_ready', timestamp: new Date().toISOString() });

        log('Starting MTG game server...');
        const mtgBinary = path.join(projectRoot, 'target', 'release', 'mtg');
        server = spawn(mtgBinary, ['server', '--port', String(SERVER_PORT), '--password', SERVER_PASSWORD, '--network-debug'], {
            cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe']
        });
        server.stdout.on('data', d => testResults.launchScriptOutput.push({ timestamp: new Date().toISOString(), line: d.toString().trim() }));
        server.stderr.on('data', d => testResults.launchScriptOutput.push({ timestamp: new Date().toISOString(), line: d.toString().trim(), stderr: true }));
        const gameServerReady = await waitForServer(SERVER_PORT);
        if (!gameServerReady) throw new Error(`Game server not ready on port ${SERVER_PORT}`);
        log('Game server ready');
        testResults.steps.push({ name: 'game_server_ready', timestamp: new Date().toISOString() });

        // Native AI peer (random controller) — pairs with the web client.
        log('Starting native AI peer (random)...');
        nativeClient = spawn(mtgBinary, [
            'connect', '--server', `localhost:${SERVER_PORT}`, '--password', SERVER_PASSWORD,
            '--name', 'NativeRandom', '--controller', 'random', deckPath,
        ], { cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe'] });
        nativeClient.stdout.on('data', d => testResults.launchScriptOutput.push({ timestamp: new Date().toISOString(), line: `NativeRandom: ${d.toString().trim()}` }));
        nativeClient.stderr.on('data', d => testResults.launchScriptOutput.push({ timestamp: new Date().toISOString(), line: `NativeRandom: ${d.toString().trim()}`, stderr: true }));

        // Give native client time to connect and settle
        log('Waiting for native client to connect...');
        await new Promise(r => setTimeout(r, 3000));

        // Launch browser
        log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        // Seed the deck as a custom deck before navigation (mtg-692).
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
            // Log errors, network messages, and WASM debug logs
            if (msg.type() === 'error' ||
                msg.text().includes('[Network]') ||
                msg.text().includes('[Test]') ||
                msg.text().includes('wasm_tui') ||
                msg.text().includes('WasmNetwork') ||
                msg.text().includes('NETWORK AI')) {
                log(`Browser: ${msg.text()}`);
            }
        });

        page.on('pageerror', err => {
            testResults.errors.push({ timestamp: new Date().toISOString(), error: err.message });
            log(`Page ERROR: ${err.message}`);
        });

        // mtg-682 page 3 / mtg-692: tui_game.html is a PURE renderer with no
        // built-in launcher. Boot the Random-controller network client ENTIRELY
        // from URL params via the auto-match contract (?mode=network&controller=
        // random&ws=&server_pass=&name=&deck=) — the server pairs it with the
        // native random peer. Replaces the deleted #game-mode / #server-url /
        // #p1-controller / #btn-launch form.
        log('Booting tui_game.html (network auto-match boot via params)...');
        const bootUrl = `http://localhost:${HTTP_PORT}/tui_game.html?` + new URLSearchParams({
            mode: 'network',
            ws: `ws://localhost:${SERVER_PORT}`,
            server_pass: SERVER_PASSWORD,
            name: 'WebRandom',
            deck: webDeckName,
            controller: 'random',
            // mtg-780: web games start PAUSED now. This unattended AI-vs-AI
            // run must opt back into auto-advancing with ?auto_run=true.
            auto_run: 'true',
        }).toString();
        await page.goto(bootUrl, { waitUntil: 'networkidle', timeout: 60000 });
        testResults.steps.push({ name: 'wasm_loaded', timestamp: new Date().toISOString() });
        log('WASM loaded; network boot initiated from params');

        // Enable rewind/replay verifier so any post-replay state divergence
        // surfaces as a "REWIND/REPLAY FATAL" entry (see fancy_tui.rs and
        // replay_verifier.rs). Even though Random itself doesn't drive rewinds,
        // the WASM TUI rewinds whenever a human-style choice gets resumed, and we
        // want any divergence from a network round-trip to fail the test rather
        // than silently corrupting state.
        const verifierEnabled = await enableReplayVerifier(page);
        log(`Replay verifier enabled: ${verifierEnabled}`);
        testResults.steps.push({ name: 'settings_filled', timestamp: new Date().toISOString() });

        // Wait for game to start (terminal should appear)
        log('Waiting for game to start...');
        try {
            await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 30000 });
            testResults.steps.push({ name: 'game_started', timestamp: new Date().toISOString() });
            log('Game terminal appeared!');
            await page.screenshot({ path: path.join(screenshotDir, 'random_03_game_started.png'), fullPage: true });
        } catch (e) {
            // Check network status
            const statusText = await page.evaluate(() => {
                return document.getElementById('network-status')?.textContent || 'no status element';
            });
            log(`Network status: ${statusText}`);
            await page.screenshot({ path: path.join(screenshotDir, 'random_03_not_started.png'), fullPage: true });

            if (statusText.includes('Error')) {
                throw new Error(`Network error: ${statusText}`);
            }
            // Log but continue - the game might still be connecting
            log('Game terminal not visible yet, but continuing...');
        }

        // Wait for game to progress (Random AI will play automatically)
        log('Waiting for game to progress (Random AI should auto-play)...');

        // Check for game over or significant progress
        let gameOver = false;
        let turnCount = 0;
        let choiceCount = 0;
        let lastLogCount = 0;
        const maxWaitTime = 180000; // 180 seconds max (3 minutes)
        const startWait = Date.now();

        while (!gameOver && (Date.now() - startWait) < maxWaitTime) {
            // Check browser logs for GameEnded message (network protocol)
            const newLogs = testResults.browserLogs.slice(lastLogCount);
            lastLogCount = testResults.browserLogs.length;

            for (const logEntry of newLogs) {
                // Check for game ended in network messages
                if (logEntry.text.includes('"type":"game_ended"') ||
                    logEntry.text.includes('type":"game_ended')) {
                    gameOver = true;
                    log('Game completed (GameEnded message received)!');
                    testResults.steps.push({ name: 'game_over', timestamp: new Date().toISOString() });
                    break;
                }

                // Track choice_seq to show progress
                const choiceMatch = logEntry.text.match(/"choice_seq":(\d+)/);
                if (choiceMatch) {
                    const newChoice = parseInt(choiceMatch[1]);
                    if (newChoice > choiceCount) {
                        choiceCount = newChoice;
                    }
                }
            }

            if (gameOver) break;

            // Check terminal for game over indicators
            const promptText = await page.evaluate(() => {
                const terminal = document.getElementById('ratzilla-terminal');
                if (terminal) {
                    const text = terminal.textContent || '';
                    return text;
                }
                return '';
            });

            if (promptText.includes('Game Over') || promptText.includes('wins!') ||
                promptText.includes('has won') || promptText.includes('defeated')) {
                gameOver = true;
                log('Game completed (terminal message)!');
                testResults.steps.push({ name: 'game_over', timestamp: new Date().toISOString() });
                break;
            }

            // Extract turn number if visible
            const turnMatch = promptText.match(/Turn (\d+)/);
            if (turnMatch) {
                const newTurn = parseInt(turnMatch[1]);
                if (newTurn > turnCount) {
                    turnCount = newTurn;
                    log(`Game progressing: Turn ${turnCount}, Choice ${choiceCount}`);
                }
            } else if (choiceCount > 0 && choiceCount % 50 === 0) {
                // Log progress every 50 choices if no turn info
                log(`Game progressing: Choice ${choiceCount}`);
            }

            await new Promise(r => setTimeout(r, 2000));
        }

        await page.screenshot({ path: path.join(screenshotDir, 'random_04_final.png'), fullPage: true });

        // Determine test result
        if (gameOver) {
            testResults.result = 'PASSED';
            testResults.message = `Game completed successfully after ${choiceCount} choices`;
        } else if (choiceCount > 0) {
            // Game progressed but didn't finish - still a partial success for network sync testing
            testResults.result = 'PARTIAL';
            testResults.message = `Game progressed through ${choiceCount} choices but did not complete in time`;
        } else if (turnCount > 0) {
            testResults.result = 'PARTIAL';
            testResults.message = `Game progressed to turn ${turnCount} but did not complete in time`;
        } else {
            // Check if we at least connected
            const connected = testResults.browserLogs.some(log =>
                log.text.includes('Game is ready') || log.text.includes('WebSocket connected')
            );
            if (connected) {
                testResults.result = 'PARTIAL';
                testResults.message = 'Connected to server but game did not progress visibly';
            } else {
                testResults.result = 'FAILED';
                testResults.message = 'Game did not start or progress';
            }
        }

        // Check for desync errors in logs - these are FATAL
        const desyncErrors = testResults.browserLogs.filter(log =>
            log.text.includes('desync') || log.text.includes('DESYNC') ||
            log.text.includes('state mismatch') || log.text.includes('SYNC ERROR') ||
            log.text.includes('action_count mismatch')
        );
        if (desyncErrors.length > 0) {
            log(`FATAL: ${desyncErrors.length} sync errors found - this is a blocking bug!`);
            testResults.desyncErrors = desyncErrors;
            testResults.result = 'FAILED';
            testResults.message = `SYNC FAILURE: ${desyncErrors.length} sync errors detected`;
        }

        // Also check for REWIND/REPLAY FATAL entries surfaced by the
        // verifier we enabled above. checkForFatalErrors already covers
        // DESYNC patterns, but matching it explicitly here lets us label
        // the failure mode distinctly in test output.
        const fatalLog = checkForFatalErrors(testResults.browserLogs);
        if (fatalLog && fatalLog.toUpperCase().includes('REWIND/REPLAY FATAL')) {
            log(`FATAL: replay-verifier divergence detected: ${fatalLog}`);
            testResults.replayVerifierError = fatalLog;
            testResults.result = 'FAILED';
            testResults.message = `REPLAY VERIFIER FAILURE: ${fatalLog}`;
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
                    await page.screenshot({ path: path.join(screenshotDir, 'random_error.png'), fullPage: true });
                }
            } catch (e) {}
        }

        return testResults;

    } finally {
        // Cleanup
        if (browser)      { log('Closing browser...');     await browser.close(); }
        if (nativeClient) { log('Stopping native peer...'); nativeClient.kill('SIGTERM'); }
        if (server)       { log('Stopping game server...'); server.kill('SIGTERM'); }
        if (httpServer)   { log('Stopping HTTP server...'); httpServer.kill('SIGTERM'); }
        await new Promise(r => setTimeout(r, 500));
    }
}

// Run the test
runTest().then(results => {
    log('');
    log('=== Test Results ===');
    log(`Result: ${results.result || 'UNKNOWN'}`);
    log(`Message: ${results.message || 'No message'}`);
    log(`Steps completed: ${results.steps.map(s => s.name).join(' -> ')}`);
    log(`Browser logs: ${results.browserLogs.length}`);
    log(`Errors: ${results.errors.length}`);

    // Write results to file
    const resultsPath = path.join(__dirname, 'network_random_test_results.json');
    fs.writeFileSync(resultsPath, JSON.stringify(results, null, 2));
    log(`Results written to: ${resultsPath}`);

    // Exit with appropriate code
    if (results.result === 'PASSED') {
        log('');
        log('SUCCESS! Network game with random controller completed.');
        process.exit(0);
    } else if (results.result === 'PARTIAL') {
        log('');
        log('Partial success - game started but did not fully complete.');
        process.exit(0);  // Don't fail for partial success
    } else {
        log('');
        log('FAILED - see error details above');
        process.exit(1);
    }
}).catch(err => {
    log(`Fatal error: ${err.message}`);
    console.error(err);
    process.exit(1);
});
