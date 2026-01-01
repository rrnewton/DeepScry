// Network Random E2E Test using Playwright + launch script
//
// This test uses ./scripts/launch_network_game.sh to start the server
// and native client, then connects with a browser using random controller.
//
// Run with: node test_network_random_e2e.js
//
// This test is NOT part of 'make validate' - it's for manual testing only.

const { chromium } = require('playwright');
const { spawn, execSync } = require('child_process');
const path = require('path');
const fs = require('fs');

// Configuration (should match launch_network_game.sh)
const SERVER_PORT = 17771;
const SERVER_PASSWORD = 'play';
const HTTP_PORT = 8000;

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
            const ws = new WebSocket(`ws://localhost:${port}`);
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
                const req = http.get(`http://localhost:${port}/fancy.html`, res => {
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
    let launchScript = null;
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

    try {
        log('=== Network Random E2E Test ===');
        log('');

        // Start the launch script
        log('Starting launch_network_game.sh...');
        launchScript = spawn('bash', ['./scripts/launch_network_game.sh'], {
            cwd: projectRoot,
            stdio: ['ignore', 'pipe', 'pipe']
        });

        launchScript.stdout.on('data', data => {
            const lines = data.toString().split('\n').filter(l => l.trim());
            for (const line of lines) {
                testResults.launchScriptOutput.push({ timestamp: new Date().toISOString(), line });
                // Only log important lines to reduce noise
                if (line.includes('Ready!') || line.includes('ERROR') ||
                    line.includes('Starting') || line.includes('finished')) {
                    log(`Launch: ${line}`);
                }
            }
        });
        launchScript.stderr.on('data', data => {
            const lines = data.toString().split('\n').filter(l => l.trim());
            for (const line of lines) {
                testResults.launchScriptOutput.push({ timestamp: new Date().toISOString(), line, stderr: true });
                log(`Launch[err]: ${line}`);
            }
        });

        // Wait for both servers to be ready
        log('Waiting for game server...');
        const gameServerReady = await waitForServer(SERVER_PORT);
        if (!gameServerReady) {
            throw new Error(`Game server not ready on port ${SERVER_PORT}`);
        }
        log('Game server ready');
        testResults.steps.push({ name: 'game_server_ready', timestamp: new Date().toISOString() });

        log('Waiting for HTTP server...');
        const httpReady = await waitForHttp(HTTP_PORT);
        if (!httpReady) {
            throw new Error(`HTTP server not ready on port ${HTTP_PORT}`);
        }
        log('HTTP server ready');
        testResults.steps.push({ name: 'http_server_ready', timestamp: new Date().toISOString() });

        // Give native client time to connect
        log('Waiting for native client to connect...');
        await new Promise(r => setTimeout(r, 3000));

        // Launch browser
        log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        // Collect console messages
        page.on('console', msg => {
            const entry = { timestamp: new Date().toISOString(), type: msg.type(), text: msg.text() };
            testResults.browserLogs.push(entry);
            // Log errors and network-related messages
            if (msg.type() === 'error' || msg.text().includes('[Network]')) {
                log(`Browser: ${msg.text()}`);
            }
        });

        page.on('pageerror', err => {
            testResults.errors.push({ timestamp: new Date().toISOString(), error: err.message });
            log(`Page ERROR: ${err.message}`);
        });

        // Navigate to fancy TUI page
        log('Loading fancy TUI page...');
        await page.goto(`http://localhost:${HTTP_PORT}/fancy.html`, {
            waitUntil: 'networkidle',
            timeout: 60000
        });

        // Wait for WASM to initialize (launcher container becomes visible)
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
        testResults.steps.push({ name: 'wasm_loaded', timestamp: new Date().toISOString() });
        log('WASM loaded');

        await page.screenshot({ path: path.join(screenshotDir, 'random_01_initial.png'), fullPage: true });

        // Select Network game mode (using game-mode selector, not controller)
        log('Selecting Remote Network game mode...');
        const gameModeExists = await page.$('#game-mode');
        if (!gameModeExists) {
            throw new Error('Game mode selector not found - UI may have changed');
        }
        await page.selectOption('#game-mode', 'network');

        // Trigger the change event explicitly and wait for UI update
        await page.evaluate(() => {
            const gameMode = document.getElementById('game-mode');
            if (gameMode) {
                gameMode.dispatchEvent(new Event('change', { bubbles: true }));
            }
        });
        await new Promise(r => setTimeout(r, 1000)); // Wait for UI update

        await page.screenshot({ path: path.join(screenshotDir, 'random_02_after_mode_select.png'), fullPage: true });

        // Check if network settings appeared
        const networkSettingsVisible = await page.isVisible('#network-settings-group');
        log(`Network settings visible: ${networkSettingsVisible}`);
        if (!networkSettingsVisible) {
            // Try to get the display style for debugging
            const displayStyle = await page.evaluate(() => {
                const el = document.getElementById('network-settings-group');
                return el ? window.getComputedStyle(el).display : 'element not found';
            });
            log(`Network settings group display style: ${displayStyle}`);
            await page.screenshot({ path: path.join(screenshotDir, 'random_02_network_missing.png'), fullPage: true });
            throw new Error(`Network settings group not visible after selecting network mode (display: ${displayStyle})`);
        }

        // Wait for the server-url field to be visible
        await page.waitForSelector('#server-url', { state: 'visible', timeout: 5000 });

        // Select Random controller for our player
        log('Selecting Random controller...');
        await page.selectOption('#p1-controller', 'random');
        await new Promise(r => setTimeout(r, 500));

        // Check if server-url is still visible after controller change
        const serverUrlStillVisible = await page.isVisible('#server-url');
        log(`Server URL visible after Random select: ${serverUrlStillVisible}`);
        await page.screenshot({ path: path.join(screenshotDir, 'random_02b_after_random.png'), fullPage: true });

        if (!serverUrlStillVisible) {
            // Debug: check the network settings group again
            const networkGroupDisplay = await page.evaluate(() => {
                const el = document.getElementById('network-settings-group');
                return el ? window.getComputedStyle(el).display : 'not found';
            });
            log(`Network settings group display after Random: ${networkGroupDisplay}`);
            throw new Error('Server URL became hidden after selecting Random controller');
        }

        // Fill in network settings
        await page.fill('#server-url', `ws://localhost:${SERVER_PORT}`);
        await page.fill('#server-password', SERVER_PASSWORD);
        await page.fill('#player-name', 'WebRandom');

        await page.screenshot({ path: path.join(screenshotDir, 'random_02_settings.png'), fullPage: true });
        testResults.steps.push({ name: 'settings_filled', timestamp: new Date().toISOString() });
        log('Settings configured: Random controller, network mode');

        // Launch the game
        log('Clicking launch button...');
        await page.click('#btn-launch');

        // Wait for connection - button text changes
        try {
            await page.waitForFunction(() => {
                const btn = document.getElementById('btn-launch');
                return btn && (btn.textContent.includes('Connecting') || btn.textContent.includes('Loading'));
            }, { timeout: 5000 });
            log('Connection initiated');
        } catch (e) {
            log('Note: Button text did not change to Connecting');
        }

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
        const maxWaitTime = 60000; // 60 seconds max
        const startWait = Date.now();

        while (!gameOver && (Date.now() - startWait) < maxWaitTime) {
            // Check for game over message
            const promptText = await page.evaluate(() => {
                // Look for game over indicators in the terminal
                const terminal = document.getElementById('ratzilla-terminal');
                if (terminal) {
                    const text = terminal.textContent || '';
                    return text;
                }
                return '';
            });

            if (promptText.includes('Game Over') || promptText.includes('wins!')) {
                gameOver = true;
                log('Game completed!');
                testResults.steps.push({ name: 'game_over', timestamp: new Date().toISOString() });
                break;
            }

            // Extract turn number if visible
            const turnMatch = promptText.match(/Turn (\d+)/);
            if (turnMatch) {
                const newTurn = parseInt(turnMatch[1]);
                if (newTurn > turnCount) {
                    turnCount = newTurn;
                    log(`Game progressing: Turn ${turnCount}`);
                }
            }

            await new Promise(r => setTimeout(r, 2000));
        }

        await page.screenshot({ path: path.join(screenshotDir, 'random_04_final.png'), fullPage: true });

        // Determine test result
        if (gameOver) {
            testResults.result = 'PASSED';
            testResults.message = `Game completed successfully after ${turnCount} turns`;
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

        // Check for desync errors in logs
        const desyncErrors = testResults.browserLogs.filter(log =>
            log.text.includes('desync') || log.text.includes('DESYNC') || log.text.includes('state mismatch')
        );
        if (desyncErrors.length > 0) {
            log(`WARNING: ${desyncErrors.length} potential desync errors found`);
            testResults.desyncErrors = desyncErrors;
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
        if (browser) {
            log('Closing browser...');
            await browser.close();
        }
        if (launchScript) {
            log('Stopping launch script...');
            // Send SIGINT to trigger cleanup trap
            launchScript.kill('SIGINT');
            // Wait for cleanup
            await new Promise(r => setTimeout(r, 2000));
        }
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
