// Network Human Input E2E Test using Playwright
// Run with: node test_network_human_input.js
//
// This test validates the rewind/replay pattern for network human mode:
// 1. Starts a native MTG server
// 2. Starts a native AI client (heuristic) as P1
// 3. Launches browser as P2 with human controller
// 4. Waits for choice prompts, makes smart selections
// 5. Verifies no MONOTONICITY VIOLATION, DESYNC, or other errors
// 6. Plays through as many turns as possible
//
// Requires:
//   make build-network
//   make wasm-network

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { getRandomPorts } = require('./test_network_utils');

// Configuration - ports allocated dynamically in runTest()
const SERVER_PASSWORD = 'test_human';
const GAME_SEED = 42;
const DECK_NAME = 'grizzly_bears.dck';

// Test limits
const MAX_CHOICES = 100;           // Maximum choices before declaring success
const GAME_TIMEOUT_MS = 180000;    // 3 minute overall game timeout
const CHOICE_TIMEOUT_MS = 20000;   // 20 second timeout per choice prompt
const POST_CHOICE_WAIT_MS = 500;   // Wait after pressing key before checking

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

// Classify what kind of choice prompt is on screen
// Returns: { type, text } or null if no prompt
function classifyPrompt(terminalText) {
    if (!terminalText) return null;

    // Check for game over first
    if (terminalText.includes('Game Over') || terminalText.includes('wins')) {
        return { type: 'game_over', text: terminalText };
    }

    // Check for various prompt types based on the prompts from controller.rs
    if (terminalText.includes('Declare Attackers')) {
        return { type: 'attackers', text: terminalText };
    }
    if (terminalText.includes('Declare Blockers')) {
        return { type: 'blockers', text: terminalText };
    }
    if (terminalText.includes('Discard') && terminalText.includes('card(s)')) {
        return { type: 'discard', text: terminalText };
    }
    if (terminalText.includes('Choose target for')) {
        return { type: 'targets', text: terminalText };
    }
    if (terminalText.includes('Choose mana source')) {
        return { type: 'mana_source', text: terminalText };
    }
    if (terminalText.includes('Search library')) {
        return { type: 'library_search', text: terminalText };
    }
    if (terminalText.includes('Choose damage order')) {
        return { type: 'damage_order', text: terminalText };
    }
    if (terminalText.includes('sacrifice')) {
        return { type: 'sacrifice', text: terminalText };
    }
    if (terminalText.includes('Choose') && terminalText.includes('mode')) {
        return { type: 'modes', text: terminalText };
    }
    if (terminalText.includes('Priority') && terminalText.includes('Choose action')) {
        return { type: 'spell_ability', text: terminalText };
    }
    // Fallback: any visible choice list (look for [0], [1] etc.)
    // This could be a priority prompt where we just have [0] pass visible
    if (terminalText.match(/\[\d\]/)) {
        // Check if this looks like a spell/ability choice with pass option
        if (terminalText.match(/\[0\] pass/)) {
            return { type: 'spell_ability', text: terminalText };
        }
        return { type: 'unknown_choice', text: terminalText };
    }
    return null;
}

// Wait for a choice prompt to appear in the terminal
// If previousText is provided, waits for the terminal text to CHANGE first
async function waitForChoicePrompt(page, timeout = CHOICE_TIMEOUT_MS, previousText = null) {
    const startTime = Date.now();
    let textChanged = (previousText === null);

    while (Date.now() - startTime < timeout) {
        const text = await extractTerminalText(page);

        // If we need the text to change first, wait for it
        if (!textChanged) {
            if (text !== previousText) {
                textChanged = true;
                // Small delay after text change to let rendering settle
                await page.waitForTimeout(200);
                continue;
            }
            await page.waitForTimeout(100);
            continue;
        }

        const prompt = classifyPrompt(text);
        if (prompt) return prompt;
        await page.waitForTimeout(200);
    }
    return null;
}

// Decide which key to press based on prompt type and available choices
// Network mode uses 0-indexed keys: '0' = choice [0], '1' = choice [1], etc.
function decideKey(prompt) {
    const text = prompt.text;

    switch (prompt.type) {
        case 'spell_ability': {
            // Parse available choices from terminal text
            // Choices are formatted as: [0] pass, [1] play Forest, [2] cast Grizzly Bears
            // (lowercase - from format_spell_ability_choice in controller.rs)

            // Priority 1: Play a land if possible
            const landMatch = text.match(/\[(\d)\] play /);
            if (landMatch) {
                return { key: landMatch[1], reason: 'play land' };
            }

            // Priority 2: Cast a creature spell if available
            // Only cast if we have enough mana (simple heuristic: we have lands on battlefield)
            const castMatch = text.match(/\[(\d)\] cast /);
            if (castMatch) {
                return { key: castMatch[1], reason: 'cast spell' };
            }

            // NOTE: Don't activate abilities by default in this test.
            // Multi-card discard (e.g. Bazaar of Baghdad's "draw 2, discard 3")
            // is now supported via the discard staging UI in fancy_tui.rs, but
            // this fixed-script driver still picks Pass to keep the test
            // deterministic; activate-ability handling can be added separately.

            // Default: Pass priority (choice [0] = "pass")
            return { key: '0', reason: 'pass priority' };
        }

        case 'attackers': {
            // Declare Attackers: [0] Done, [1] creature1, [2] creature2, ...
            // Attack with the first available creature if any
            const creatureMatch = text.match(/\[(\d)\] (?!Done)/);
            if (creatureMatch) {
                return { key: creatureMatch[1], reason: 'attack with creature' };
            }
            // No creatures, select Done
            return { key: '0', reason: 'done (no attackers)' };
        }

        case 'blockers': {
            // Declare Blockers: [0] Done, [1] blocker blocks attacker, ...
            // Block with first available blocker if any
            const blockMatch = text.match(/\[(\d)\] (?!Done)/);
            if (blockMatch) {
                return { key: blockMatch[1], reason: 'block with creature' };
            }
            return { key: '0', reason: 'done (no blockers)' };
        }

        case 'discard': {
            // Discard N card(s): [0] Card1, [1] Card2, ...
            // Discard the first card
            return { key: '0', reason: 'discard first card' };
        }

        case 'targets': {
            // Choose target: [0] No target, [1] target1, ...
            // Select first valid target (not "No target")
            const targetMatch = text.match(/\[(\d)\] (?!No target)/);
            if (targetMatch) {
                return { key: targetMatch[1], reason: 'select target' };
            }
            return { key: '0', reason: 'no target' };
        }

        case 'mana_source': {
            // Choose mana source: [0] Forest, [1] Forest, ...
            // Select first mana source
            return { key: '0', reason: 'first mana source' };
        }

        case 'library_search': {
            // Search library: [0] Fail to find, [1] card1, ...
            // Select first card if available
            const cardMatch = text.match(/\[(\d)\] (?!Fail to find)/);
            if (cardMatch) {
                return { key: cardMatch[1], reason: 'search: select card' };
            }
            return { key: '0', reason: 'fail to find' };
        }

        case 'damage_order': {
            // Choose damage order: [0] creature1, [1] creature2, ...
            return { key: '0', reason: 'first damage order' };
        }

        case 'sacrifice': {
            // Choose permanent to sacrifice: [0] Done, [1] permanent1, ...
            // If we must sacrifice, pick the first option after Done
            const sacMatch = text.match(/\[(\d)\] (?!Done)/);
            if (sacMatch) {
                return { key: sacMatch[1], reason: 'sacrifice permanent' };
            }
            return { key: '0', reason: 'done (sacrifice)' };
        }

        case 'modes': {
            // Choose mode: [0] mode1, [1] mode2, ...
            return { key: '0', reason: 'first mode' };
        }

        default: {
            // Unknown choice: try first option
            return { key: '0', reason: 'unknown prompt (first option)' };
        }
    }
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

    // Allocate random ports to avoid conflicts with other tests
    const { serverPort: SERVER_PORT, httpPort: HTTP_PORT } = await getRandomPorts();

    try {
        log(`Using ports: server=${SERVER_PORT}, http=${HTTP_PORT}`);
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
        await new Promise(r => setTimeout(r, 2000));

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
        const serverErrors = [];
        server.stdout.on('data', data => {
            const text = data.toString().trim();
            log(`Server: ${text}`);
        });
        server.stderr.on('data', data => {
            const text = data.toString().trim();
            // Log server output - highlight action dumps and sync errors
            if (text.includes('SERVER_ACTION_DUMP')) {
                log(`Server ACTION DUMP:\n${text}`);
            } else if (text.includes('SYNC MISMATCH') || text.includes('DESYNC')) {
                log(`Server SYNC ERROR: ${text}`);
            } else {
                log(`Server: ${text}`);
            }
            // Capture sync mismatch and error messages from server
            if (text.includes('SYNC MISMATCH') || text.includes('DESYNC') || text.includes('InvalidAction')) {
                serverErrors.push(text);
            }
        });

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
            // Log WASM network mode messages for debugging
            if (msg.text().includes('NETWORK REPLAY') || msg.text().includes('NETWORK NORMAL')
                || msg.text().includes('COMBAT_DEBUG') || msg.text().includes('REPLAY_DEBUG')) {
                log(`WASM: ${msg.text().substring(0, 400)}`);
            }
            // Log WASM hash debug messages for sync analysis
            if (msg.text().includes('WASM_HASH_DEBUG')) {
                // Show full text for action dumps (they can be long)
                if (msg.text().includes('ACTION COUNT MISMATCH')) {
                    log(`WASM ACTION MISMATCH:\n${msg.text()}`);
                } else {
                    log(`WASM: ${msg.text().substring(0, 300)}`);
                }
            }
            // Log WASM action dumps for comparing with server dumps
            if (msg.text().includes('WASM_ACTION_DUMP')) {
                log(`WASM: ${msg.text()}`);
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

        // Enable debug mode for WASM hash debug logging
        const debugCheckbox = await page.$('#debug-mode');
        if (debugCheckbox) {
            await debugCheckbox.check();
        }

        await page.screenshot({ path: path.join(screenshotDir, 'net_human_01_settings.png'), fullPage: true });
        log('Settings filled, launching...');

        // Launch the game
        await page.click('#btn-launch');

        // Wait for terminal to appear
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 20000 });
        log('Game terminal appeared');

        // Wait for game to initialize
        await page.waitForTimeout(3000);

        await page.screenshot({ path: path.join(screenshotDir, 'net_human_02_game_started.png'), fullPage: true });

        // Check for early fatal errors
        let fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) {
            throw new Error(`Fatal error before first choice: ${fatalError}`);
        }

        // Track server/client process state for crash detection
        let serverExited = false;
        let nativeClientExited = false;
        server.on('exit', (code) => {
            serverExited = true;
            log(`Server process exited with code ${code}`);
        });
        nativeClient.on('exit', (code) => {
            nativeClientExited = true;
            log(`NativeAI process exited with code ${code}`);
        });

        // Main game loop - make choices until game ends or we hit the limit
        const gameStartTime = Date.now();
        let choicesMade = 0;
        let gameEnded = false;
        let lastTurnInfo = '';
        const choiceHistory = [];
        let lastTerminalText = null; // Track previous terminal text for change detection

        log('Starting game play loop...');

        for (let i = 0; i < MAX_CHOICES; i++) {
            // Check overall game timeout
            if (Date.now() - gameStartTime > GAME_TIMEOUT_MS) {
                log(`Game timeout reached (${GAME_TIMEOUT_MS / 1000}s). Made ${choicesMade} choices.`);
                break;
            }

            // Check if server crashed (game is over abnormally)
            if (serverExited && !gameEnded) {
                throw new Error(`Server process exited unexpectedly after ${choicesMade} choices`);
            }

            // Check for fatal errors before each choice (browser + server)
            fatalError = checkForFatalErrors(browserLogs);
            if (fatalError) {
                throw new Error(`Fatal error before choice ${i + 1}: ${fatalError}`);
            }
            if (serverErrors.length > 0) {
                // Wait a moment to capture action dumps that may follow
                await page.waitForTimeout(1000);
                throw new Error(`Server error before choice ${i + 1}: ${serverErrors[0].substring(0, 500)}`);
            }

            // Wait for a choice prompt, requiring text change after first choice
            const prompt = await waitForChoicePrompt(page, CHOICE_TIMEOUT_MS, lastTerminalText);

            if (!prompt) {
                // No prompt - check if game ended
                const text = await extractTerminalText(page);
                if (text.includes('Game Over') || text.includes('wins')) {
                    log(`Game ended (detected during wait)`);
                    gameEnded = true;
                    break;
                }
                // Could be waiting for server
                if (choicesMade === 0) {
                    await page.screenshot({ path: path.join(screenshotDir, 'net_human_03_no_choice.png'), fullPage: true });
                    log(`No initial choice prompt. Terminal: ${text.substring(0, 300)}`);
                    throw new Error('No choice prompt appeared within timeout');
                }
                log(`No prompt after choice ${choicesMade}, continuing...`);
                lastTerminalText = null; // Reset change detection
                continue;
            }

            if (prompt.type === 'game_over') {
                gameEnded = true;
                log(`Game over detected!`);
                break;
            }

            // Decide what key to press
            const decision = decideKey(prompt);

            // Extract turn info for logging
            const turnMatch = prompt.text.match(/Turn (\d+)/);
            const turnInfo = turnMatch ? `T${turnMatch[1]}` : '??';
            if (turnInfo !== lastTurnInfo) {
                log(`--- Turn ${turnInfo} ---`);
                lastTurnInfo = turnInfo;
            }

            log(`Choice ${i + 1} [${prompt.type}]: pressing '${decision.key}' (${decision.reason})`);

            // Save terminal text before pressing key for change detection on next iteration
            lastTerminalText = prompt.text;

            // Press the key
            await page.keyboard.press(decision.key);
            choicesMade++;
            choiceHistory.push({ type: prompt.type, key: decision.key, reason: decision.reason });

            // Brief wait for key to register
            await page.waitForTimeout(POST_CHOICE_WAIT_MS);

            // Check for fatal errors after the choice
            fatalError = checkForFatalErrors(browserLogs);
            if (fatalError) {
                await page.screenshot({
                    path: path.join(screenshotDir, `net_human_error_after_choice_${i + 1}.png`),
                    fullPage: true
                });
                throw new Error(`Fatal error after choice ${i + 1} [${prompt.type}, key='${decision.key}']: ${fatalError}`);
            }

            // Take periodic screenshots (every 10 choices)
            if ((i + 1) % 10 === 0 || i < 5) {
                await page.screenshot({
                    path: path.join(screenshotDir, `net_human_choice_${String(i + 1).padStart(3, '0')}.png`),
                    fullPage: true
                });
            }
        }

        // Final check for errors
        fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) {
            throw new Error(`Fatal error at end of test: ${fatalError}`);
        }

        await page.screenshot({ path: path.join(screenshotDir, 'net_human_final.png'), fullPage: true });
        const finalText = await extractTerminalText(page);

        // Print summary
        const elapsedSec = ((Date.now() - gameStartTime) / 1000).toFixed(1);
        log(`\n=== TEST PASSED ===`);
        log(`Made ${choicesMade} choices in ${elapsedSec}s`);
        log(`Game ended: ${gameEnded}`);
        log(`No DESYNC or MONOTONICITY VIOLATION errors detected`);

        // Print choice type breakdown
        const typeCounts = {};
        for (const c of choiceHistory) {
            typeCounts[c.type] = (typeCounts[c.type] || 0) + 1;
        }
        log(`Choice breakdown: ${JSON.stringify(typeCounts)}`);

        // Print final game state snippet
        log(`Final state: ${finalText.substring(0, 300)}`);

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
        const errorLogs = browserLogs.filter(l =>
            l.type === 'error' ||
            l.text.includes('MONOTONICITY') ||
            l.text.includes('DESYNC') ||
            l.text.includes('panic')
        );
        if (errorLogs.length > 0) {
            log('\nRelevant browser error logs:');
            for (const entry of errorLogs.slice(-10)) {
                log(`  ${entry.text.substring(0, 300)}`);
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
