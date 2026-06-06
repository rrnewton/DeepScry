#!/usr/bin/env node
/**
 * test_redo_ai_network_e2e.js — Step-0 foundational harness for the lobby redo (mtg-35z3s).
 *
 * THE QUESTION: Does an AI-vs-AI game over the NETWORKED web path actually run
 * to completion, with the UI updating and no freeze?
 *
 * This is the minimal harness that proves (or disproves) that an AI-driven
 * networked web game advances ≥3 full turns. It bypasses the broken lobby by
 * constructing the direct tui_game.html URL with the correct query params,
 * opening TWO headless browser contexts (one per player), both running a
 * random AI controller. If the game freezes or desyncs, we capture WHERE.
 *
 * Architecture:
 *   - mtg server (WebSocket game server, port picked dynamically)
 *   - python http.server (serves web/tui_game.html + WASM pkg, port picked dynamically)
 *   - Browser P1: tui_game.html with game-mode=network, controller=random, lobby_create
 *   - Browser P2: tui_game.html with game-mode=network, controller=random, lobby_join
 *
 * IMPORTANT: The lobby bypass uses the URL param contract from lobby_launcher.js:
 *   ?lobby_create=<name>&ws=<ws_url>&name=<player_name>&deck=<deck_name>&mode=network
 *   ?lobby_join=<name>&ws=<ws_url>&name=<player_name>&deck=<deck_name>&mode=network
 *
 * But since applyLobbyParamsToForm() always sets controller to 'human', we
 * drive the launcher form directly via Playwright instead of relying on
 * auto-launch. This is the "network mode, random controller" path tested by
 * test_network_gui_e2e.js — we just open two browser clients instead of one
 * browser + one native AI.
 *
 * Usage:
 *   node test_redo_ai_network_e2e.js
 *   node test_redo_ai_network_e2e.js --deck grizzly_bears.dck
 *   node test_redo_ai_network_e2e.js --seed 42
 *   node test_redo_ai_network_e2e.js --min-turns 5
 *
 * NOT part of make validate (results may vary while redo is in progress).
 * Run manually: cd web && node test_redo_ai_network_e2e.js
 *
 * Requires:
 *   ../target/release/mtg  (built with: make build-network)
 *   web/pkg/mtg_engine.js  (built with: make wasm-network)
 */

'use strict';

const { chromium } = require('playwright');
const { spawn }    = require('child_process');
const path         = require('path');
const fs           = require('fs');

const {
    log,
    getRandomPorts,
    waitForServer,
    checkForFatalErrors,
    enableReplayVerifier,
    extractTerminalText,
} = require('./test_network_utils');
const { parseDckIntoCustomDeck } = require('./game_boot_params');

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------
function parseArgs() {
    const args = process.argv.slice(2);
    let deckName  = 'grizzly_bears.dck';
    let seed      = 42;
    let minTurns  = 3;   // minimum turns required to pass
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--deck'      && args[i+1]) { deckName = args[++i]; if (!deckName.endsWith('.dck')) deckName += '.dck'; }
        else if (args[i] === '--seed' && args[i+1]) { seed     = parseInt(args[++i]); }
        else if (args[i] === '--min-turns' && args[i+1]) { minTurns = parseInt(args[++i]); }
    }
    return { deckName, seed, minTurns };
}

const { deckName: DECK_NAME, seed: GAME_SEED, minTurns: MIN_TURNS } = parseArgs();

const SERVER_PASSWORD = 'redo_e2e';
const GAME_NAME       = 'redo-test-game';
const GAME_TIMEOUT_MS = 240_000;  // 4 minutes

// ---------------------------------------------------------------------------
// Wait for HTTP server
// ---------------------------------------------------------------------------
async function waitForHttp(port, maxAttempts = 30) {
    const http = require('http');
    for (let i = 0; i < maxAttempts; i++) {
        try {
            await new Promise((resolve, reject) => {
                const req = http.get(`http://127.0.0.1:${port}/tui_game.html`, res => {
                    if (res.statusCode === 200) resolve();
                    else reject(new Error(`HTTP ${res.statusCode}`));
                    res.resume();
                });
                req.on('error', reject);
                req.setTimeout(1000, () => reject(new Error('timeout')));
            });
            return true;
        } catch (_) {
            await new Promise(r => setTimeout(r, 500));
        }
    }
    return false;
}

// ---------------------------------------------------------------------------
// Launch one browser client into a network game
//
// mode: 'create' | 'join'
// Returns the Playwright page object.
// ---------------------------------------------------------------------------
async function launchBrowserClient(browser, httpPort, serverPort, mode, playerName, deckContent, browserLogs) {
    const page = await browser.newPage();

    page.on('console', msg => {
        const entry = { timestamp: Date.now(), type: msg.type(), text: msg.text() };
        browserLogs.push(entry);
        // Log errors and key network/wasm events
        if (msg.type() === 'error') {
            log(`[${playerName}] Browser ERROR: ${msg.text().substring(0, 200)}`);
        }
        if (msg.text().includes('WASM_HASH_DEBUG') || msg.text().includes('DESYNC') ||
            msg.text().includes('MONOTONICITY') || msg.text().includes('REWIND/REPLAY FATAL')) {
            log(`[${playerName}] WASM: ${msg.text().substring(0, 300)}`);
        }
    });

    page.on('pageerror', err => {
        browserLogs.push({ timestamp: Date.now(), type: 'pageerror', text: err.message });
        log(`[${playerName}] Page ERROR: ${err.message}`);
    });

    // mtg-35z3s page 3: tui_game.html is a PURE renderer with no built-in
    // launcher. Boot the networked AI game ENTIRELY from URL params — the lobby
    // param contract (lobby_create/lobby_join) + the new &controller= AI-driver
    // override + &deck= pointing at a custom deck seeded into localStorage. This
    // is exactly the spec's "AI controllers over networked web play" strategy.
    const deckNameMatch = deckContent.match(/^\s*Name\s*=\s*(.+)$/im);
    const displayName = deckNameMatch
        ? deckNameMatch[1].trim()
        : path.basename(DECK_NAME).replace(/\.(dck|txt)$/i, '');

    // Seed the deck into localStorage as a custom deck (the page's
    // loadCardsForDecks reads + registers it by name). Works for ANY .dck,
    // including ones outside the WASM export globs.
    const customDeck = parseDckIntoCustomDeck(deckContent);
    await page.addInitScript(({ name, deck }) => {
        const KEY = 'mtg-forge-custom-decks';
        let decks = {};
        try { decks = JSON.parse(localStorage.getItem(KEY) || '{}'); } catch (e) { /* ignore */ }
        decks[name] = deck;
        localStorage.setItem(KEY, JSON.stringify(decks));
    }, { name: displayName, deck: customDeck });

    // Build the lobby boot URL: lobby_create/lobby_join + random AI controller.
    const lobbyKey = mode === 'create' ? 'lobby_create' : 'lobby_join';
    const qp = new URLSearchParams({
        ws: `ws://127.0.0.1:${serverPort}`,
        server_pass: SERVER_PASSWORD,
        name: playerName,
        deck: displayName,
        controller: 'random',
        mode: 'network',
        // mtg-780: web games start PAUSED now; this unattended AI-vs-AI run
        // opts back into auto-advancing with ?auto_run=true (matches the
        // "auto_run=true handles it automatically" note further below).
        auto_run: 'true',
    });
    qp.set(lobbyKey, GAME_NAME);
    const bootUrl = `http://127.0.0.1:${httpPort}/tui_game.html?` + qp.toString();
    await page.goto(bootUrl, { waitUntil: 'networkidle', timeout: 60_000 });
    log(`[${playerName}] Booted ${mode} "${GAME_NAME}" (network, random AI, deck "${displayName}")`);

    // Enable replay verifier
    const verifierOn = await enableReplayVerifier(page);
    log(`[${playerName}] Replay verifier: ${verifierOn}`);

    // Wait for terminal to appear (page auto-connects + renders on game ready)
    try {
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 20_000 });
        log(`[${playerName}] Game terminal appeared`);
    } catch (e) {
        const status = await page.evaluate(() =>
            document.getElementById('status')?.textContent || 'no status');
        log(`[${playerName}] Terminal not visible yet. Status: "${status}"`);
        // Give it a bit more time
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 20_000 });
    }

    return page;
}

// ---------------------------------------------------------------------------
// Poll for turn progress on a page
// ---------------------------------------------------------------------------
async function getTurnInfo(page) {
    try {
        const termText = await extractTerminalText(page);
        const turnMatch = termText.match(/Turn (\d+)/);
        const turnNum = turnMatch ? parseInt(turnMatch[1]) : 0;
        const isOver  = termText.includes('Game Over') || termText.includes('wins!') ||
                        termText.includes('has won')    || termText.includes('defeated');
        return { turnNum, isOver, text: termText };
    } catch (_) {
        return { turnNum: 0, isOver: false, text: '' };
    }
}

// ---------------------------------------------------------------------------
// Main test
// ---------------------------------------------------------------------------
async function runTest() {
    const projectRoot  = path.join(__dirname, '..');
    const screenshotDir = path.join(__dirname, '..', 'debug', 'redo_e2e_screenshots');
    fs.mkdirSync(screenshotDir, { recursive: true });

    // Prerequisites check
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
        throw new Error('mtg binary not found. Run: make build-network (or cargo build --release --features network)');
    }

    // Load deck content for injection
    const deckPath = DECK_NAME.includes('/')
        ? path.join(projectRoot, DECK_NAME)
        : path.join(projectRoot, 'decks', DECK_NAME);
    const deckContent = fs.readFileSync(deckPath, 'utf8');

    // Allocate ports
    const { serverPort: SERVER_PORT, httpPort: HTTP_PORT } = await getRandomPorts();
    log(`=== Redo AI Network E2E Test ===`);
    log(`Deck: ${DECK_NAME}, Seed: ${GAME_SEED}, Min turns: ${MIN_TURNS}`);
    log(`Ports: server=${SERVER_PORT}, http=${HTTP_PORT}`);

    let server     = null;
    let httpServer = null;
    let browser    = null;

    const p1Logs = [];
    const p2Logs = [];

    const result = {
        passed:      false,
        turns:       0,
        choices:     0,
        gameOver:    false,
        desyncFound: false,
        freezeAt:    null,
        errors:      [],
        timeline:    [],
    };

    try {
        // Start HTTP server for static web assets
        log('Starting HTTP server...');
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: __dirname,
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        const httpReady = await waitForHttp(HTTP_PORT);
        if (!httpReady) throw new Error('HTTP server failed to start');
        log('HTTP server ready');

        // Start game server
        log('Starting game server...');
        const serverLogs = [];
        server = spawn(mtgBinary, [
            'server',
            '--port',     SERVER_PORT.toString(),
            '--password', SERVER_PASSWORD,
            '--seed',     GAME_SEED.toString(),
            '--network-debug',
        ], {
            cwd: projectRoot,
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        server.stdout.on('data', d => {
            const line = d.toString().trim();
            serverLogs.push(line);
            if (line.includes('ERROR') || line.includes('DESYNC') || line.includes('SYNC MISMATCH')) {
                log(`Server ERROR: ${line}`);
                result.errors.push({ source: 'server', text: line });
            }
        });
        server.stderr.on('data', d => {
            const line = d.toString().trim();
            serverLogs.push(line);
            // Only treat lines that are genuinely game-sync failures.
            // The server also logs [ERROR] for benign disconnect events when the
            // test browser closes first — those are expected and not sync bugs.
            const isSyncFailure =
                line.includes('DESYNC') ||
                line.includes('SYNC MISMATCH') ||
                (line.includes('InvalidAction') && !line.includes('P1 handler') && !line.includes('P2 handler')) ||
                (line.includes('[ERROR') && line.includes('state mismatch'));
            if (isSyncFailure) {
                log(`Server SYNC ERROR: ${line}`);
                result.errors.push({ source: 'server-stderr', text: line });
            }
        });

        const serverReady = await waitForServer(SERVER_PORT);
        if (!serverReady) throw new Error('Game server failed to start');
        log('Game server ready');

        // Launch browser with two contexts
        log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader'],
        });

        // Open P1 first (creator) — P2 joins after
        log('Opening P1 (creator) browser context...');
        const p1Page = await launchBrowserClient(
            browser, HTTP_PORT, SERVER_PORT, 'create', 'AI_P1', deckContent, p1Logs
        );
        result.timeline.push({ event: 'p1_connected', time: Date.now() });

        // Brief pause before P2 connects so CreateGame completes first
        await new Promise(r => setTimeout(r, 2000));

        log('Opening P2 (joiner) browser context...');
        const p2Page = await launchBrowserClient(
            browser, HTTP_PORT, SERVER_PORT, 'join', 'AI_P2', deckContent, p2Logs
        );
        result.timeline.push({ event: 'p2_connected', time: Date.now() });

        // Give both clients a moment to sync game state
        await new Promise(r => setTimeout(r, 3000));

        // Screenshot initial state
        await p1Page.screenshot({ path: path.join(screenshotDir, '01_p1_game_started.png') });
        await p2Page.screenshot({ path: path.join(screenshotDir, '01_p2_game_started.png') });

        // Check for early fatal errors
        let fatalError = checkForFatalErrors(p1Logs) || checkForFatalErrors(p2Logs);
        if (fatalError) throw new Error(`Fatal error before gameplay: ${fatalError}`);

        // Poll for game progress
        log('Polling for game progress (both AI controllers should auto-run)...');
        const startWait   = Date.now();
        let lastLogReport = Date.now();
        let p1Choices     = 0;
        let p2Choices     = 0;
        let lastP1Turn    = 0;
        let lastP2Turn    = 0;

        while (Date.now() - startWait < GAME_TIMEOUT_MS) {
            // Check for choices/turn progress in browser logs
            for (const entry of p1Logs) {
                const m = entry.text.match(/"choice_seq":(\d+)/);
                if (m) p1Choices = Math.max(p1Choices, parseInt(m[1]));
            }
            for (const entry of p2Logs) {
                const m = entry.text.match(/"choice_seq":(\d+)/);
                if (m) p2Choices = Math.max(p2Choices, parseInt(m[1]));
            }

            // Check game_ended messages
            const p1GameEnded = p1Logs.some(e =>
                e.text.includes('"type":"game_ended"') || e.text.includes('type":"game_ended'));
            const p2GameEnded = p2Logs.some(e =>
                e.text.includes('"type":"game_ended"') || e.text.includes('type":"game_ended'));
            if (p1GameEnded || p2GameEnded) {
                log('game_ended message received!');
                result.gameOver = true;
                break;
            }

            // Get turn info from both terminals
            const [p1Info, p2Info] = await Promise.all([
                getTurnInfo(p1Page),
                getTurnInfo(p2Page),
            ]);

            if (p1Info.isOver || p2Info.isOver) {
                log('Terminal shows game over!');
                result.gameOver = true;
                break;
            }

            const maxTurn = Math.max(p1Info.turnNum, p2Info.turnNum);
            result.turns = maxTurn;

            if (maxTurn > lastP1Turn || maxTurn > lastP2Turn) {
                log(`Progress: Turn ${maxTurn} | P1 choices=${p1Choices} | P2 choices=${p2Choices}`);
                lastP1Turn = p1Info.turnNum;
                lastP2Turn = p2Info.turnNum;
                result.timeline.push({ event: 'turn', turn: maxTurn, time: Date.now() });
            } else if (Date.now() - lastLogReport > 10_000) {
                log(`Still running: Turn ${maxTurn} | P1 choices=${p1Choices} | P2 choices=${p2Choices} | elapsed=${((Date.now()-startWait)/1000).toFixed(0)}s`);
                lastLogReport = Date.now();
            }

            // Check for MIN_TURNS threshold early exit
            if (maxTurn >= MIN_TURNS && (p1Choices > 0 || p2Choices > 0)) {
                log(`Reached minimum ${MIN_TURNS} turns — sufficient for foundational test.`);
                result.passed = true;
                break;
            }

            // Check for fatal errors during play
            fatalError = checkForFatalErrors(p1Logs) || checkForFatalErrors(p2Logs);
            if (fatalError) {
                result.freezeAt = {
                    turn: result.turns,
                    p1Choices, p2Choices,
                    error: fatalError,
                };
                throw new Error(`Fatal error during play: ${fatalError}`);
            }

            if (result.errors.length > 0) {
                result.freezeAt = {
                    turn: result.turns,
                    p1Choices, p2Choices,
                    error: result.errors[0].text,
                };
                throw new Error(`Server error during play: ${result.errors[0].text.substring(0, 300)}`);
            }

            await new Promise(r => setTimeout(r, 2000));
        }

        result.choices = Math.max(p1Choices, p2Choices);

        // Passed if: game over, or ≥MIN_TURNS and choices > 0
        if (result.gameOver) {
            result.passed = true;
        } else if (result.turns >= MIN_TURNS && result.choices > 0) {
            result.passed = true;
        } else if (result.turns === 0 && result.choices === 0) {
            result.passed = false;
            result.freezeAt = { turn: 0, choices: 0, error: 'Game never progressed (no turns, no choices)' };
        }

        // Final screenshots
        await p1Page.screenshot({ path: path.join(screenshotDir, '02_p1_final.png') });
        await p2Page.screenshot({ path: path.join(screenshotDir, '02_p2_final.png') });

        // Final fatal error check
        fatalError = checkForFatalErrors(p1Logs) || checkForFatalErrors(p2Logs);
        if (fatalError && !result.desyncFound) {
            result.desyncFound = true;
            result.freezeAt = result.freezeAt || { turn: result.turns, error: fatalError };
            result.passed = false;
        }

    } catch (err) {
        log(`Test error: ${err.message}`);
        result.errors.push({ source: 'test', text: err.message });
        if (!result.passed) result.passed = false;
        if (!result.freezeAt) {
            result.freezeAt = { turn: result.turns, error: err.message };
        }
    } finally {
        if (browser)     { log('Closing browser...');      await browser.close(); }
        if (server)      { log('Stopping server...');      server.kill('SIGTERM'); }
        if (httpServer)  { log('Stopping HTTP server...');  httpServer.kill('SIGTERM'); }
    }

    return result;
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------
runTest().then(result => {
    log('');
    log('=== TEST RESULTS ===');
    log(`RESULT:   ${result.passed ? 'PASSED' : 'FAILED'}`);
    log(`Turns:    ${result.turns}  (required ≥${MIN_TURNS})`);
    log(`Choices:  ${result.choices}`);
    log(`Game over: ${result.gameOver}`);
    log(`Desync:   ${result.desyncFound}`);
    if (result.freezeAt) {
        log(`FROZE AT: Turn ${result.freezeAt.turn} — ${result.freezeAt.error || ''}`);
    }
    if (result.timeline.length > 0) {
        log('Timeline:');
        const t0 = result.timeline[0].time;
        for (const ev of result.timeline) {
            log(`  +${((ev.time - t0)/1000).toFixed(1)}s ${ev.event}${ev.turn ? ` (T${ev.turn})` : ''}`);
        }
    }
    log('');
    log('ANSWER TO THE QUESTION:');
    if (result.passed && !result.desyncFound) {
        log('  AI-over-network web play WORKS. ≥3 turns advanced with no freeze/desync.');
        log('  The Space-advance mechanism is NOT needed: auto_run=true handles it automatically.');
        log('  AI-over-network IS a viable e2e driver for the redo.');
    } else if (result.desyncFound) {
        log('  DESYNC DETECTED — AI-over-network web play has a gating engine/network bug.');
        log(`  Froze/desynced at: turn=${result.freezeAt?.turn}, choices=${result.choices}`);
    } else {
        log('  GAME DID NOT PROGRESS — possible freeze or hang.');
        log(`  ${result.freezeAt?.error || 'No progress observed.'}`);
    }

    process.exit(result.passed ? 0 : 1);
}).catch(err => {
    log(`Fatal: ${err.message}`);
    console.error(err);
    process.exit(1);
});
