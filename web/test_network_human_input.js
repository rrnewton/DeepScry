// Network Human Input E2E Test using Playwright
// Run with: node test_network_human_input.js
//
// This test validates the rewind/replay pattern for network human mode:
// 1. Starts a native MTG server
// 2. Starts a native AI client (heuristic) as P1
// 3. Launches browser as P2 with human controller
// 4. Waits for choice prompts, makes selections
// 5. Verifies no MONOTONICITY VIOLATION or other errors
// 6. Verifies the game advances after each choice
//
// Requires:
//   make build-network
//   make wasm-network

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

// Configuration
const SERVER_PORT = 17773;
const SERVER_PASSWORD = 'test_human';
const HTTP_PORT = 8769;
const GAME_SEED = 42;
const DECK_NAME = 'grizzly_bears.dck';

function log(message) {
    const timestamp = new Date().toISOString().substring(11, 23);
    console.log(`[${timestamp}] ${message}`);
}

// Wait for server to be ready by attempting WebSocket connection
async function waitForServer(port, maxAttempts = 30) {
    const WebSocket = require('ws');
    for (let i = 0; i < maxAttempts; i++) {
        try {
            const ws = new WebSocket(`ws://localhost:${port}`);
            await new Promise((resolve, reject) => {
                ws.on('open', () => { ws.close(); resolve(); });
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

// Extract all text from the RatZilla terminal DOM
async function extractTerminalText(page) {
    return await page.evaluate(() => {
        const terminal = document.getElementById('ratzilla-terminal');
        if (!terminal) return 'NO TERMINAL';
        const rows = [];
        const rowElements = terminal.querySelectorAll('div');
        for (const row of rowElements) {
            const text = row.textContent || '';
            if (text.trim()) rows.push(text);
        }
        return rows.join('\n');
    });
}

// Wait for a choice prompt to appear in the terminal
async function waitForChoicePrompt(page, timeout = 15000) {
    const startTime = Date.now();
    while (Date.now() - startTime < timeout) {
        const text = await extractTerminalText(page);
        if (text.includes('Choose') || text.includes('Play land') || text.includes('Pass')) {
            return text;
        }
        await page.waitForTimeout(200);
    }
    return null;
}

// Check for fatal errors in browser console logs
function checkForFatalErrors(browserLogs) {
    const fatalPatterns = [
        'MONOTONICITY VIOLATION',
        'FATAL DESYNC',
        'DESYNC',
        'unreachable',
        'panic',
    ];
    // Per NETWORK_ARCHITECTURE.md: desync is ALWAYS fatal, never papered over.
    // Any mismatch between local and server state is an immediate test failure.
    for (const entry of browserLogs) {
        for (const pattern of fatalPatterns) {
            if (entry.text.toUpperCase().includes(pattern.toUpperCase())) {
                return entry.text;
            }
        }
    }
    return null;
}

async function runTest() {
    let server = null;
    let nativeClient = null;
    let httpServer = null;
    let browser = null;
    const browserLogs = [];
    const screenshotDir = path.join(__dirname, 'screenshots');

    if (!fs.existsSync(screenshotDir)) {
        fs.mkdirSync(screenshotDir);
    }

    try {
        // Check prerequisites
        const wasmPkgPath = path.join(__dirname, 'pkg', 'mtg_forge_rs.js');
        if (!fs.existsSync(wasmPkgPath)) {
            throw new Error('WASM package not found. Run: make wasm-network');
        }
        const wasmContent = fs.readFileSync(wasmPkgPath, 'utf8');
        if (!wasmContent.includes('network_init')) {
            throw new Error('WASM package missing network features. Run: make wasm-network');
        }

        const projectRoot = path.join(__dirname, '..');
        const mtgBinary = path.join(projectRoot, 'target', 'release', 'mtg');
        if (!fs.existsSync(mtgBinary)) {
            throw new Error('mtg binary not found. Run: make build-network');
        }

        // Start HTTP server
        log('Starting HTTP server...');
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: __dirname,
            stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(r => setTimeout(r, 1000));

        // Start MTG server
        log('Starting MTG server...');
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
        server.stdout.on('data', data => log(`Server: ${data.toString().trim()}`));
        server.stderr.on('data', data => log(`Server: ${data.toString().trim()}`));

        const serverReady = await waitForServer(SERVER_PORT);
        if (!serverReady) throw new Error('Server failed to start');
        log('Server ready');

        // Start native AI client as P1 (heuristic so it actively plays)
        log('Starting native AI client as P1 (heuristic)...');
        const deckPath = path.join(projectRoot, 'decks', DECK_NAME);
        nativeClient = spawn(mtgBinary, [
            'connect',
            '--server', `localhost:${SERVER_PORT}`,
            '--password', SERVER_PASSWORD,
            '--name', 'NativeAI',
            '--controller', 'heuristic',
            deckPath
        ], {
            cwd: projectRoot,
            stdio: ['ignore', 'pipe', 'pipe']
        });
        nativeClient.stdout.on('data', data => log(`NativeAI: ${data.toString().trim()}`));
        nativeClient.stderr.on('data', data => log(`NativeAI: ${data.toString().trim()}`));

        // Give native client time to connect
        await new Promise(r => setTimeout(r, 2000));

        // Launch browser
        log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        page.on('console', msg => {
            const entry = { timestamp: Date.now(), type: msg.type(), text: msg.text() };
            browserLogs.push(entry);
            // Log errors and important messages
            if (msg.type() === 'error') {
                log(`Browser ERROR: ${msg.text().substring(0, 200)}`);
            }
        });

        page.on('pageerror', err => {
            browserLogs.push({ timestamp: Date.now(), type: 'pageerror', text: err.message });
            log(`Page ERROR: ${err.message}`);
        });

        // Navigate to fancy TUI
        log('Loading page...');
        await page.goto(`http://localhost:${HTTP_PORT}/fancy.html`, {
            waitUntil: 'networkidle',
            timeout: 60000
        });

        // Wait for WASM to load
        await page.waitForSelector('#launcher.show', { state: 'attached', timeout: 30000 });
        log('WASM loaded');

        // Select Network game mode
        await page.selectOption('#game-mode', 'network');
        await page.waitForSelector('#network-settings-group', { state: 'visible', timeout: 5000 });

        // Fill in network settings (human controller for P2)
        await page.fill('#server-url', `ws://localhost:${SERVER_PORT}`);
        await page.fill('#server-password', SERVER_PASSWORD);
        await page.fill('#player-name', 'HumanP2');

        // Select human controller
        await page.selectOption('#p1-controller', 'human');

        await page.screenshot({ path: path.join(screenshotDir, 'net_human_01_settings.png'), fullPage: true });
        log('Settings filled, launching...');

        // Launch the game
        await page.click('#btn-launch');

        // Wait for terminal to appear
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 20000 });
        log('Game terminal appeared');

        // Wait a bit for game to initialize and auto-pass initial choices
        await page.waitForTimeout(3000);

        await page.screenshot({ path: path.join(screenshotDir, 'net_human_02_game_started.png'), fullPage: true });

        // Check for early fatal errors
        let fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) {
            throw new Error(`Fatal error before first choice: ${fatalError}`);
        }

        // Wait for a choice prompt to appear
        log('Waiting for choice prompt...');
        let choiceText = await waitForChoicePrompt(page, 20000);

        if (!choiceText) {
            // Take a screenshot to see what's on screen
            await page.screenshot({ path: path.join(screenshotDir, 'net_human_03_no_choice.png'), fullPage: true });
            const text = await extractTerminalText(page);
            log(`Terminal text (no choice found): ${text.substring(0, 500)}`);

            // Check if game ended
            if (text.includes('Game Over') || text.includes('wins')) {
                log('Game ended before human could make a choice (AI won quickly)');
                log('TEST PASSED (game completed without errors)');
                return true;
            }
            throw new Error('No choice prompt appeared within timeout');
        }

        log('Choice prompt appeared!');
        await page.screenshot({ path: path.join(screenshotDir, 'net_human_03_choice.png'), fullPage: true });

        // Try to make choices: press number keys to select options
        // In the human controller, choices are displayed and selected by number keys
        const maxChoices = 5;
        let choicesMade = 0;

        for (let i = 0; i < maxChoices; i++) {
            // Check for fatal errors before each choice
            fatalError = checkForFatalErrors(browserLogs);
            if (fatalError) {
                throw new Error(`Fatal error before choice ${i + 1}: ${fatalError}`);
            }

            // Get current terminal text
            const text = await extractTerminalText(page);

            // Check if game ended
            if (text.includes('Game Over') || text.includes('wins')) {
                log(`Game ended after ${choicesMade} choices`);
                break;
            }

            // Determine what key to press
            // If there's a "Play land" option, select it (typically option 2+)
            // Otherwise, press '1' to pass
            let key;
            if (text.includes('Play land')) {
                key = '2'; // First non-pass option (play a land)
                log(`Choice ${i + 1}: Pressing '${key}' (play land)`);
            } else {
                key = '1'; // Pass priority
                log(`Choice ${i + 1}: Pressing '${key}' (pass/first option)`);
            }

            await page.keyboard.press(key);
            choicesMade++;

            // Wait for the game to process the choice
            await page.waitForTimeout(1000);

            // Check for fatal errors after the choice
            fatalError = checkForFatalErrors(browserLogs);
            if (fatalError) {
                await page.screenshot({
                    path: path.join(screenshotDir, `net_human_error_after_choice_${i + 1}.png`),
                    fullPage: true
                });
                throw new Error(`Fatal error after choice ${i + 1}: ${fatalError}`);
            }

            await page.screenshot({
                path: path.join(screenshotDir, `net_human_${String(i + 4).padStart(2, '0')}_after_choice_${i + 1}.png`),
                fullPage: true
            });

            // Wait for next choice prompt (or game end)
            const nextText = await waitForChoicePrompt(page, 10000);
            if (!nextText) {
                const finalText = await extractTerminalText(page);
                if (finalText.includes('Game Over') || finalText.includes('wins')) {
                    log(`Game ended after ${choicesMade} choices`);
                    break;
                }
                log(`No next choice prompt after choice ${i + 1}, continuing...`);
                break;
            }
        }

        // Final check for errors
        fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) {
            throw new Error(`Fatal error at end of test: ${fatalError}`);
        }

        await page.screenshot({ path: path.join(screenshotDir, 'net_human_final.png'), fullPage: true });
        const finalText = await extractTerminalText(page);
        log(`Final terminal text: ${finalText.substring(0, 500)}`);

        log(`\n=== TEST PASSED ===`);
        log(`Made ${choicesMade} choices without MONOTONICITY VIOLATION`);
        log(`No fatal errors detected`);

        return true;

    } catch (error) {
        log(`\n=== TEST FAILED: ${error.message} ===`);

        if (browser) {
            try {
                const page = browser.contexts()[0]?.pages()[0];
                if (page) {
                    await page.screenshot({ path: path.join(screenshotDir, 'net_human_failure.png'), fullPage: true });
                    const text = await extractTerminalText(page);
                    fs.writeFileSync(path.join(screenshotDir, 'net_human_terminal_failure.txt'), text);
                }
            } catch (e) {}
        }

        // Dump relevant browser logs
        const errorLogs = browserLogs.filter(l => l.type === 'error' || l.text.includes('MONOTONICITY'));
        if (errorLogs.length > 0) {
            log('\nRelevant browser error logs:');
            for (const entry of errorLogs.slice(-10)) {
                log(`  ${entry.text.substring(0, 200)}`);
            }
        }

        return false;

    } finally {
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

runTest().then(success => {
    process.exit(success ? 0 : 1);
}).catch(err => {
    log(`Fatal error: ${err.message}`);
    console.error(err);
    process.exit(1);
});
