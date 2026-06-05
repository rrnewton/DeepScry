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
    // mtg-610: --undo-dump opts INTO the heavyweight per-choice-point server
    // full-undo-log dump (MTG_NET_FULL_UNDO_DUMP). OFF by default so the routine
    // gate stays quiet/fast; turn it on when diagnosing a specific desync. The
    // WASM-side mismatch dump fires regardless (only on a desync, bounded).
    let undoDump = false;
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--deck' && args[i + 1]) {
            deckName = args[++i];
            if (!deckName.endsWith('.dck')) deckName += '.dck';
        } else if (args[i] === '--seed' && args[i + 1]) {
            seed = parseInt(args[++i]);
        } else if (args[i] === '--human') {
            humanMode = true;
        } else if (args[i] === '--undo-dump') {
            undoDump = true;
        }
    }
    return { deckName, seed, humanMode, undoDump };
}

const { deckName: DECK_NAME, seed: GAME_SEED, humanMode: HUMAN_MODE, undoDump: UNDO_DUMP } = parseArgs();

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
    // mtg-610: raw, untruncated accumulators for the full undo-log dumps so a
    // desync can be root-caused by diffing the EXACT diverging entries (the
    // per-line console logging truncates for display). serverRawStderr holds the
    // native server's complete stderr (incl. SERVER_FULL_UNDO_DUMP_* blocks);
    // the WASM full dumps live untruncated in browserLogs[].text.
    let serverRawStderr = '';
    // Debug-dump destination is gitignored (debug/), never tracked.
    const debugDumpDir = path.join(projectRoot, 'debug', 'netarch-undo-dumps');

    if (!fs.existsSync(screenshotDir)) {
        fs.mkdirSync(screenshotDir);
    }
    if (!fs.existsSync(debugDumpDir)) {
        fs.mkdirSync(debugDumpDir, { recursive: true });
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
            stdio: ['ignore', 'pipe', 'pipe'],
            // mtg-610: only enable the server's per-choice-point bounded undo-log
            // dump when explicitly diagnosing (--undo-dump). OFF by default so the
            // routine gate stays fast and quiet; the WASM-side mismatch dump still
            // fires on any desync regardless of this flag.
            env: UNDO_DUMP ? { ...process.env, MTG_NET_FULL_UNDO_DUMP: '1' } : process.env,
        });
        server.stdout.on('data', data => {
            log(`Server: ${data.toString().trim()}`);
        });
        server.stderr.on('data', data => {
            const text = data.toString().trim();
            // mtg-610: keep the COMPLETE raw stderr (untruncated, including the
            // multi-line SERVER_FULL_UNDO_DUMP_* blocks) for file-based diffing.
            serverRawStderr += data.toString();
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
            // Pin the controller master seed (mtg-mb668): WITHOUT this the native
            // RandomController falls back to an ENTROPY seed (main.rs ~1625), so
            // P2's CHOICES differ every run even at a fixed --seed — the deck
            // shuffle is pinned but the AI is not. That non-determinism is what
            // made robots42 "intermittent on a fixed seed" (~37% fail = the
            // fraction of random playthroughs that hit a latent desync). Both
            // clients share the same master seed so derive_player_seed() gives
            // each a consistent per-slot controller seed (the "MUST match" invariant
            // in tui_game.html). Makes the gate deterministically reproducible.
            '--seed-player', GAME_SEED.toString(),
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
            // Pin the WASM controller master seed (mtg-mb668). Without it the page
            // defaults controllerSeed to 0 (tui_game.html ~2473) while the native
            // client uses entropy — so the two controllers' master seeds neither
            // match nor stay fixed across runs. Passing the same GAME_SEED to both
            // makes the FULL game (deck shuffle + both players' choices) reproducible.
            seed: GAME_SEED.toString(),
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

        // --- Regression guard (player-name leak): the LOCAL player's slot must
        // render the USERNAME, NOT the deck name. The historical bug
        // (fancy_tui.rs launch_network_game) set the local player's display name
        // to the deck name, so a player who joined as "WebRandom" with deck
        // "Simple Bolt Test Deck" saw their OWN slot labelled with the deck name.
        // The TUI info bar renders "<name>: <N> life", so we assert (a) the
        // username appears and (b) the deck name is NOT used as a player label.
        const expectedUserName = `Web${HUMAN_MODE ? 'Human' : 'Random'}`;
        const initialTermText = await extractTerminalText(page);
        const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        const deckAsPlayerLabel = new RegExp(escapeRe(browserDeckName) + '\\s*:\\s*\\d+\\s*life', 'i');
        if (!initialTermText.includes(expectedUserName)) {
            throw new Error(
                `Player-name leak regression: local player slot did not render username ` +
                `"${expectedUserName}". Terminal (truncated): ${initialTermText.slice(0, 500)}`);
        }
        if (deckAsPlayerLabel.test(initialTermText)) {
            throw new Error(
                `Player-name leak regression: the DECK name "${browserDeckName}" is rendered ` +
                `as a player label (should be the username "${expectedUserName}").`);
        }
        log(`Player-name check OK: local slot shows username "${expectedUserName}", not deck name "${browserDeckName}"`);

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

        // mtg-610: write the FULL, untruncated undo-log dumps to files so the
        // exact diverging entries can be diffed. The WASM shadow's full dump is
        // in browserLogs (WASM_FULL_UNDO_DUMP_BEGIN/END); the server's is in
        // serverRawStderr (SERVER_FULL_UNDO_DUMP_BEGIN/END, one block per choice
        // point — the LAST one before the failure is at the desync action_count).
        try {
            const stamp = `${prefix}_${path.basename(DECK_NAME).replace(/\.dck$/i, '')}_seed${GAME_SEED}`;
            // All WASM full-dump blocks (keep every one; the last is the desync).
            const wasmDumps = browserLogs
                .map(l => l.text)
                .filter(t => t.includes('WASM_FULL_UNDO_DUMP_BEGIN'));
            const wasmPath = path.join(debugDumpDir, `${stamp}_wasm_undo.log`);
            fs.writeFileSync(wasmPath, wasmDumps.join('\n\n========\n\n') || '(no WASM full-undo dumps captured)\n');
            // Server full dumps: extract every SERVER_FULL_UNDO_DUMP block.
            const serverBlocks = [];
            const re = /SERVER_FULL_UNDO_DUMP_BEGIN[\s\S]*?SERVER_FULL_UNDO_DUMP_END/g;
            let m;
            while ((m = re.exec(serverRawStderr)) !== null) serverBlocks.push(m[0]);
            const serverPath = path.join(debugDumpDir, `${stamp}_server_undo.log`);
            fs.writeFileSync(serverPath, serverBlocks.join('\n\n========\n\n') || '(no SERVER full-undo dumps captured)\n');
            // Also dump the WASM hash-debug mismatch lines for quick orientation.
            const mismatchLines = browserLogs
                .map(l => l.text)
                .filter(t => t.includes('ACTION COUNT MISMATCH') || t.includes('state hash mismatch'));
            const mismatchPath = path.join(debugDumpDir, `${stamp}_mismatch.log`);
            fs.writeFileSync(mismatchPath, mismatchLines.join('\n') || '(no mismatch lines)\n');
            // mtg-mb668 class-A: per-action per-card detail (battlefield id/tapped/ctrl
            // + graveyard ids) from BOTH sides, keyed by action_count, so the EXACT
            // diverging field at the desync AC can be diffed. WASM lines come from the
            // shadow's WASM_CARD_DETAIL log; the server's come from the SERVER STATE
            // block of the NETWORK SYNC MISMATCH box in stderr.
            const wasmCardDetail = browserLogs
                .map(l => l.text)
                .filter(t => t.includes('WASM_CARD_DETAIL') || t.includes('WASM_SUBMIT'));
            const cardDetailPath = path.join(debugDumpDir, `${stamp}_card_detail.log`);
            // mtg-559 deep-ac: the WASM client currently sends debug_info=None, so the
            // server's "DIFFERENCES:" comparison section is suppressed. Capture the box
            // up to its closing ╝ regardless (the "SERVER STATE:" block with
            // battlefield_detail / graveyard_ids / life / hands / libs is printed even
            // when the client side is None) so the server-vs-client field diff can be
            // done by hand against the WASM_CARD_DETAIL lines above.
            const serverMismatchBox = (serverRawStderr.match(/NETWORK SYNC MISMATCH DETECTED[\s\S]*?╚[^\n]*/g) || [])
                .join('\n\n========\n\n');
            fs.writeFileSync(
                cardDetailPath,
                `=== WASM shadow per-action card detail (last ~${wasmCardDetail.length}) ===\n` +
                (wasmCardDetail.join('\n') || '(no WASM_CARD_DETAIL captured)') +
                `\n\n=== SERVER mismatch box (real server detail) ===\n` +
                (serverMismatchBox || '(no server mismatch box captured)') + '\n'
            );
            // mtg-559 deep-ac: also persist the COMPLETE raw server stderr (gitignored
            // debug dir) so nothing is lost to regex assumptions while diagnosing.
            fs.writeFileSync(
                path.join(debugDumpDir, `${stamp}_server_stderr_full.log`),
                serverRawStderr || '(empty)\n'
            );
            // DEEPAC_PROBE (temporary): dump WASM-shadow probe lines from the browser console.
            const deepacWasm = browserLogs.map(l => l.text).filter(t => t.includes('DEEPAC_'));
            fs.writeFileSync(
                path.join(debugDumpDir, `${stamp}_deepac_wasm.log`),
                deepacWasm.join('\n') || '(no DEEPAC_ wasm lines)\n'
            );
            log(`\nmtg-610 full undo-log dumps written:`);
            log(`  WASM  : ${wasmPath} (${wasmDumps.length} block(s))`);
            log(`  SERVER: ${serverPath} (${serverBlocks.length} block(s))`);
            log(`  MISMATCH lines: ${mismatchPath}`);
            log(`  CARD DETAIL: ${cardDetailPath} (${wasmCardDetail.length} WASM line(s))`);
        } catch (e) {
            log(`  (failed to write undo-log dumps: ${e.message})`);
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
