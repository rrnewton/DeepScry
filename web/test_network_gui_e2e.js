// Network GUI E2E Test - plays a full networked game through tui_game.html
//
// Tests that the fancy TUI GUI correctly handles a networked game with
// either a random or human controller, verifying no DESYNC errors.
//
// Modes:
//   node test_network_gui_e2e.js          # Random controller (auto-plays)
//   node test_network_gui_e2e.js --human  # Human controller (Playwright presses keys)
//
// Requires:
//   make build-network
//   make wasm-network

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const {
    log,
    getRandomPorts,
    waitForServer,
    extractTerminalText,
    checkForFatalErrors,
    enableReplayVerifier,
    classifyPrompt,
    decideKey,
    submitChoice,
    waitForChoicePrompt,
} = require('./test_network_utils');

// Configuration - ports allocated dynamically to avoid conflicts
const SERVER_PASSWORD = 'test_gui';

// mtg-35z3s page 3: the game pages no longer carry a built-in .dck parser
// (parseDckFormat lived in the deleted launcher). To give the browser client
// the SAME deck as the native client, we parse the .dck via the shared helper
// and seed it into localStorage as a custom deck under `mtg-forge-custom-decks`
// — the page's loadCardsForDecks() reads + registers it by name.
const { parseDckIntoCustomDeck } = require('./game_boot_params');

// Parse CLI arguments: --deck <name> --seed <n> --human
function parseArgs() {
    const args = process.argv.slice(2);
    let deckName = 'grizzly_bears.dck';
    let seed = 42;
    let humanMode = false;
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--deck' && args[i + 1]) {
            deckName = args[++i];
            if (!deckName.endsWith('.dck')) deckName += '.dck';
        } else if (args[i] === '--seed' && args[i + 1]) {
            seed = parseInt(args[++i]);
        } else if (args[i] === '--human') {
            humanMode = true;
        }
    }
    return { deckName, seed, humanMode };
}

const { deckName: DECK_NAME, seed: GAME_SEED, humanMode: HUMAN_MODE } = parseArgs();

// Test limits
const MAX_CHOICES = 200;            // Maximum human choices before declaring success
const GAME_TIMEOUT_MS = 180000;     // 3 minute overall game timeout
const CHOICE_TIMEOUT_MS = 20000;    // 20 second timeout per choice prompt
const POST_CHOICE_WAIT_MS = 500;    // Wait after pressing key before checking

async function runTest() {
    let server = null;
    let nativeClient = null;
    let httpServer = null;
    let browser = null;
    const browserLogs = [];
    const serverErrors = [];
    const screenshotDir = path.join(__dirname, 'screenshots');
    const projectRoot = path.join(__dirname, '..');

    if (!fs.existsSync(screenshotDir)) {
        fs.mkdirSync(screenshotDir);
    }

    const prefix = HUMAN_MODE ? 'gui_human' : 'gui_random';

    // Allocate random ports to avoid conflicts with other tests
    const { serverPort: SERVER_PORT, httpPort: HTTP_PORT } = await getRandomPorts();

    try {
        log(`=== Network GUI E2E Test (${HUMAN_MODE ? 'human' : 'random'} mode) ===`);
        log(`Using ports: server=${SERVER_PORT}, http=${HTTP_PORT}`);

        // Check prerequisites
        const wasmPkgPath = path.join(__dirname, 'pkg', 'mtg_engine.js');
        if (!fs.existsSync(wasmPkgPath)) {
            throw new Error('WASM package not found. Run: make wasm-network');
        }
        const wasmContent = fs.readFileSync(wasmPkgPath, 'utf8');
        if (!wasmContent.includes('network_init')) {
            throw new Error('WASM package missing network features. Run: make wasm-network');
        }

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
        server.stdout.on('data', data => {
            log(`Server: ${data.toString().trim()}`);
        });
        server.stderr.on('data', data => {
            const text = data.toString().trim();
            if (text.includes('SYNC MISMATCH') || text.includes('DESYNC') || text.includes('InvalidAction')) {
                serverErrors.push(text);
                log(`Server SYNC ERROR: ${text}`);
            } else if (text.includes('SERVER_ACTION_DUMP')) {
                log(`Server ACTION DUMP:\n${text}`);
            } else {
                log(`Server: ${text}`);
            }
        });

        const serverReady = await waitForServer(SERVER_PORT);
        if (!serverReady) throw new Error('Server failed to start');
        log('Server ready');

        // Start native AI client as P1
        log('Starting native AI client as P1...');
        // Resolve deck path: accepts "grizzly_bears.dck" or "decks/old_school/foo.dck"
        const deckPath = DECK_NAME.includes('/')
            ? path.join(projectRoot, DECK_NAME)
            : path.join(projectRoot, 'decks', DECK_NAME);
        // Read the .dck content so we can give the SAME deck to the browser P2
        // (a single --deck applies to BOTH seats -> deterministic mirror match).
        // Many top-level decks (monored, counterspells, ...) are NOT in the WASM
        // export globs, so we inject the deck into the browser as a custom deck
        // rather than relying on the dropdown's filtered built-in list.
        const deckContent = fs.readFileSync(deckPath, 'utf8');
        // Pull the deck's display name out of [metadata] (same field the
        // browser's parseDckFormat keys on) for the dropdown selection.
        const deckNameMatch = deckContent.match(/^\s*Name\s*=\s*(.+)$/im);
        const browserDeckName = deckNameMatch
            ? deckNameMatch[1].trim()
            : path.basename(DECK_NAME).replace(/\.(dck|txt)$/i, '');
        nativeClient = spawn(mtgBinary, [
            'connect',
            '--server', `localhost:${SERVER_PORT}`,
            '--password', SERVER_PASSWORD,
            '--name', 'NativeAI',
            '--controller', 'random',
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
            if (msg.type() === 'error') {
                log(`Browser ERROR: ${msg.text().substring(0, 200)}`);
            }
            if (msg.text().includes('NETWORK REPLAY') || msg.text().includes('NETWORK NORMAL') ||
                msg.text().includes('WASM_HASH_DEBUG') || msg.text().includes('WASM_ACTION_DUMP')) {
                log(`WASM: ${msg.text().substring(0, 300)}`);
            }
        });

        page.on('pageerror', err => {
            browserLogs.push({ timestamp: Date.now(), type: 'pageerror', text: err.message });
            log(`Page ERROR: ${err.message}`);
        });

        // mtg-35z3s page 3: tui_game.html is a PURE renderer with no built-in
        // launcher. Boot network mode entirely from URL params (auto-match: no
        // lobby_create/join — the server pairs this web client with the native
        // `mtg connect` client). The browser gets the SAME deck as the native
        // P1: seed it into localStorage as a custom deck (reusing the page's own
        // getCustomDecks + register_custom_deck load path, which works for ANY
        // .dck including ones outside the WASM export globs) and pass it via
        // &deck=. controller=random|human is the new AI-driver override param.
        const controllerType = HUMAN_MODE ? 'human' : 'random';
        const customDeck = parseDckIntoCustomDeck(deckContent, browserDeckName);
        await page.addInitScript(({ name, deck }) => {
            const KEY = 'mtg-forge-custom-decks';
            let decks = {};
            try { decks = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch (e) { /* ignore */ }
            decks[name] = deck;
            localStorage.setItem(KEY, JSON.stringify(decks));
        }, { name: browserDeckName, deck: customDeck });

        log('Loading tui_game.html (network auto-match boot via params)...');
        const bootUrl = `http://localhost:${HTTP_PORT}/tui_game.html?` + new URLSearchParams({
            mode: 'network',
            ws: `ws://localhost:${SERVER_PORT}`,
            server_pass: SERVER_PASSWORD,
            name: `Web${HUMAN_MODE ? 'Human' : 'Random'}`,
            deck: browserDeckName,
            controller: controllerType,
        }).toString();
        await page.goto(bootUrl, { waitUntil: 'networkidle', timeout: 60000 });

        // Belt-and-braces: enableReplayVerifier (the page enables it only in
        // debug mode, which is no longer reachable via params). checkForFatalErrors
        // matches REWIND/REPLAY FATAL too.
        const verifierEnabled = await enableReplayVerifier(page);
        log(`Replay verifier enabled: ${verifierEnabled}`);

        await page.screenshot({ path: path.join(screenshotDir, `${prefix}_01_settings.png`), fullPage: true });
        log(`Network boot: ${controllerType} controller, deck "${browserDeckName}", connecting...`);

        // Wait for terminal to appear (page auto-connects + renders on game ready)
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 30000 });
        log('Game terminal appeared');

        // Wait for game to initialize
        await page.waitForTimeout(3000);
        await page.screenshot({ path: path.join(screenshotDir, `${prefix}_02_game_started.png`), fullPage: true });

        // Check for early fatal errors
        let fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) {
            throw new Error(`Fatal error before gameplay: ${fatalError}`);
        }

        // Track process state
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

        if (HUMAN_MODE) {
            // === Human mode: make choices via keyboard ===
            await runHumanMode(page, browserLogs, serverErrors, screenshotDir, prefix, serverExited);
        } else {
            // === Random mode: auto-plays, just wait for completion ===
            await runRandomMode(page, browserLogs, serverErrors, screenshotDir, prefix, serverExited);
        }

        // Final error check
        fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) {
            throw new Error(`Fatal error at end of test: ${fatalError}`);
        }
        if (serverErrors.length > 0) {
            throw new Error(`Server sync error: ${serverErrors[0].substring(0, 500)}`);
        }

        await page.screenshot({ path: path.join(screenshotDir, `${prefix}_final.png`), fullPage: true });

        log('\n=== TEST PASSED ===');
        log('No DESYNC or MONOTONICITY VIOLATION errors detected');
        return true;

    } catch (error) {
        log(`\n=== TEST FAILED: ${error.message} ===`);

        if (browser) {
            try {
                const page = browser.contexts()[0]?.pages()[0];
                if (page) {
                    await page.screenshot({ path: path.join(screenshotDir, `${prefix}_failure.png`), fullPage: true });
                    const text = await extractTerminalText(page);
                    fs.writeFileSync(path.join(screenshotDir, `${prefix}_terminal_failure.txt`), text);
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

// Random mode: auto-plays via Random controller, just poll for completion
async function runRandomMode(page, browserLogs, serverErrors, screenshotDir, prefix, serverExited) {
    log('Random mode: waiting for game to auto-play...');

    const startWait = Date.now();
    let gameOver = false;
    let lastLogCount = 0;
    let choiceCount = 0;
    let turnCount = 0;

    while (!gameOver && (Date.now() - startWait) < GAME_TIMEOUT_MS) {
        // Check browser logs for game ended
        const newLogs = browserLogs.slice(lastLogCount);
        lastLogCount = browserLogs.length;

        for (const logEntry of newLogs) {
            if (logEntry.text.includes('"type":"game_ended"') ||
                logEntry.text.includes('type":"game_ended')) {
                gameOver = true;
                log('Game completed (GameEnded message received)!');
                break;
            }
            const choiceMatch = logEntry.text.match(/"choice_seq":(\d+)/);
            if (choiceMatch) {
                const newChoice = parseInt(choiceMatch[1]);
                if (newChoice > choiceCount) choiceCount = newChoice;
            }
        }

        if (gameOver) break;

        // Check terminal for game over
        const termText = await extractTerminalText(page);
        if (termText.includes('Game Over') || termText.includes('wins!') ||
            termText.includes('has won') || termText.includes('defeated')) {
            gameOver = true;
            log('Game completed (terminal message)!');
            break;
        }

        // Track turn progress
        const turnMatch = termText.match(/Turn (\d+)/);
        if (turnMatch) {
            const newTurn = parseInt(turnMatch[1]);
            if (newTurn > turnCount) {
                turnCount = newTurn;
                log(`Game progressing: Turn ${turnCount}, Choice ${choiceCount}`);
            }
        }

        // Check for fatal errors during play
        const fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) throw new Error(`Fatal error during random play: ${fatalError}`);
        if (serverErrors.length > 0) throw new Error(`Server error during random play: ${serverErrors[0].substring(0, 500)}`);

        await new Promise(r => setTimeout(r, 2000));
    }

    const elapsed = ((Date.now() - startWait) / 1000).toFixed(1);
    if (gameOver) {
        log(`Random game completed in ${elapsed}s (${choiceCount} choices, ${turnCount} turns)`);
    } else if (choiceCount > 0) {
        log(`Random game progressed: ${choiceCount} choices, ${turnCount} turns in ${elapsed}s (partial success)`);
    } else {
        throw new Error('Random game did not progress');
    }
}

// Human mode: Playwright presses keys to make choices
async function runHumanMode(page, browserLogs, serverErrors, screenshotDir, prefix, serverExited) {
    log('Human mode: making choices via keyboard...');

    const gameStartTime = Date.now();
    let choicesMade = 0;
    let gameEnded = false;
    let lastTurnInfo = '';
    const choiceHistory = [];
    let lastTerminalText = null;

    for (let i = 0; i < MAX_CHOICES; i++) {
        // Check overall timeout
        if (Date.now() - gameStartTime > GAME_TIMEOUT_MS) {
            log(`Game timeout reached. Made ${choicesMade} choices.`);
            break;
        }

        // Check for fatal errors
        const fatalError = checkForFatalErrors(browserLogs);
        if (fatalError) throw new Error(`Fatal error before choice ${i + 1}: ${fatalError}`);
        if (serverErrors.length > 0) throw new Error(`Server error before choice ${i + 1}: ${serverErrors[0].substring(0, 500)}`);

        // Wait for choice prompt
        const prompt = await waitForChoicePrompt(page, CHOICE_TIMEOUT_MS, lastTerminalText);

        if (!prompt) {
            const text = await extractTerminalText(page);
            if (text.includes('Game Over') || text.includes('wins')) {
                log('Game ended (detected during wait)');
                gameEnded = true;
                break;
            }
            if (choicesMade === 0) {
                await page.screenshot({ path: path.join(screenshotDir, `${prefix}_no_choice.png`), fullPage: true });
                throw new Error('No choice prompt appeared within timeout');
            }
            log(`No prompt after choice ${choicesMade}, continuing...`);
            lastTerminalText = null;
            continue;
        }

        if (prompt.type === 'game_over') {
            gameEnded = true;
            log('Game over detected!');
            break;
        }

        // Decide what to press
        const decision = decideKey(prompt);

        // Log turn progress
        const turnMatch = prompt.text.match(/Turn (\d+)/);
        const turnInfo = turnMatch ? `T${turnMatch[1]}` : '??';
        if (turnInfo !== lastTurnInfo) {
            log(`--- Turn ${turnInfo} ---`);
            lastTurnInfo = turnInfo;
        }

        const numChoices = prompt.numChoices || 0;
        log(`Choice ${i + 1} [${prompt.type}]: key='${decision.key}' (${decision.reason}) [${numChoices} choices]`);

        // Save text for change detection
        lastTerminalText = prompt.text;

        // Submit the choice (handles multi-digit input)
        await submitChoice(page, decision.key, numChoices);
        choicesMade++;
        choiceHistory.push({ type: prompt.type, key: decision.key, reason: decision.reason });

        // Brief wait for key to register
        await page.waitForTimeout(POST_CHOICE_WAIT_MS);

        // Check for errors after choice
        const postError = checkForFatalErrors(browserLogs);
        if (postError) {
            await page.screenshot({
                path: path.join(screenshotDir, `${prefix}_error_choice_${i + 1}.png`),
                fullPage: true
            });
            throw new Error(`Fatal error after choice ${i + 1}: ${postError}`);
        }

        // Periodic screenshots
        if ((i + 1) % 20 === 0 || i < 3) {
            await page.screenshot({
                path: path.join(screenshotDir, `${prefix}_choice_${String(i + 1).padStart(3, '0')}.png`),
                fullPage: true
            });
        }
    }

    const elapsed = ((Date.now() - gameStartTime) / 1000).toFixed(1);
    log(`Human mode: ${choicesMade} choices in ${elapsed}s, game ended: ${gameEnded}`);

    // Print choice breakdown
    const typeCounts = {};
    for (const c of choiceHistory) {
        typeCounts[c.type] = (typeCounts[c.type] || 0) + 1;
    }
    log(`Choice breakdown: ${JSON.stringify(typeCounts)}`);
}

runTest().then(success => {
    process.exit(success ? 0 : 1);
}).catch(err => {
    log(`Fatal error: ${err.message}`);
    console.error(err);
    process.exit(1);
});
