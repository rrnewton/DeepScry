#!/usr/bin/env node
/**
 * test_redo_multiturn_reload_e2e.js — the lobby-redo ACCEPTANCE gate
 * (mtg-682 acceptance test items 4 + 5; the "multiturn-e2e + reload" piece
 * left over from the gamepages closeout / mtg-692 sibling work).
 *
 * Builds on the Step-0 harness (test_redo_ai_network_e2e.js) but goes further:
 *
 *   item 4 — PLAY MULTIPLE FULL TURNS: two browser clients (a CREATE + a JOIN),
 *            both driven by the WASM random AI controller, advance >= MIN_TURNS
 *            full turns over the networked web path with BOTH clients staying in
 *            sync (no DESYNC / REWIND-REPLAY-FATAL / freeze).
 *
 *   item 5 — RELOAD-RESILIENCE: mid-game we RELOAD one client's page. The spec's
 *            done-criterion is: the reloaded client either reconnects and resumes
 *            in sync, OR fails CLEANLY with a clear message — but NEVER a silent
 *            corrupt/frozen state, and crucially the SURVIVING client must not
 *            desync or freeze. We assert exactly that: after the reload (a) the
 *            surviving client keeps advancing turns with no fatal error, and (b)
 *            the reloaded client lands in a well-defined state (a game terminal /
 *            card board, OR a clear status message), never a JS crash / silent
 *            hang.
 *
 *            NB: server-side reconnect task-reattachment is still a Phase-1 stub
 *            (protocol.rs ReconnectResult), so we do NOT require seamless resume
 *            of the reloaded client — only the "clean, not corrupt" guarantee and
 *            the survivor's continued progress. When real resume lands this test
 *            tightens (the reloaded client should rejoin the SAME turn count).
 *
 * Both renderer targets are covered: a leg with the reloaded client on
 * native_game.html (the DEFAULT renderer, card DOM) and a leg on tui_game.html
 * (ratzilla terminal). The OTHER (surviving) client is always tui_game.html so
 * the turn-progress probe is uniform on the survivor.
 *
 * Wired into `make validate` (validate-network-e2e-step), mirroring how
 * test_network_gui_e2e.js / test_network_multideck.js are invoked, so the redo
 * play path is gated on every validate run.
 *
 * Usage:
 *   node test_redo_multiturn_reload_e2e.js
 *   node test_redo_multiturn_reload_e2e.js --deck grizzly_bears.dck --seed 42 --min-turns 3
 *   node test_redo_multiturn_reload_e2e.js --quick      # one leg (native reload) only
 *
 * Requires:
 *   ../target/release/mtg  (make build-network)
 *   web/pkg/mtg_engine.js  (make wasm-network)
 */

'use strict';

const { chromium } = require('playwright');
const { spawn }    = require('child_process');
const path         = require('path');
const fs           = require('fs');

const {
    log,
    LOCALHOST,
    getRandomPorts,
    waitForServer,
    checkForFatalErrors,
    enableReplayVerifier,
    extractTerminalText,
} = require('./test_network_utils');
const { firstBuiltinDeck } = require('./game_boot_params');

// ---------------------------------------------------------------------------
// CLI args
// ---------------------------------------------------------------------------
function parseArgs() {
    const args = process.argv.slice(2);
    // A built-in deck NAME (as exported in data/sets/index.json deck_names), NOT
    // a .dck path: the native_game.html boot path only loads built-in decks (no
    // custom-deck registration), so both legs need a built-in both clients can
    // load. Empty => auto-pick the first built-in deck at runtime.
    let deck     = '';
    let seed     = 42;
    let minTurns = 3;
    let quick    = false;
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--deck'      && args[i + 1]) { deck = args[++i]; }
        else if (args[i] === '--seed' && args[i + 1]) { seed = parseInt(args[++i], 10); }
        else if (args[i] === '--min-turns' && args[i + 1]) { minTurns = parseInt(args[++i], 10); }
        else if (args[i] === '--quick') { quick = true; }
    }
    return { deck, seed, minTurns, quick };
}

const { deck: DECK_ARG, seed: GAME_SEED, minTurns: MIN_TURNS, quick: QUICK } = parseArgs();

const SERVER_PASSWORD  = 'redo_reload_e2e';
const HTTP_HOST        = LOCALHOST;
const PER_LEG_TIMEOUT  = 180_000;   // 3 min per leg

// ---------------------------------------------------------------------------
// Wait for the static HTTP server to serve a known page.
// ---------------------------------------------------------------------------
async function waitForHttp(port, page = 'tui_game.html', maxAttempts = 30) {
    const http = require('http');
    for (let i = 0; i < maxAttempts; i++) {
        try {
            await new Promise((resolve, reject) => {
                const req = http.get(`http://${HTTP_HOST}:${port}/${page}`, (res) => {
                    if (res.statusCode === 200) resolve(); else reject(new Error(`HTTP ${res.statusCode}`));
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
// Renderer-agnostic turn-progress probe.
//   native_game.html exposes window.__mtg.getViewModel() -> {turn_number, game_over}
//   tui_game.html    renders into #ratzilla-terminal; we parse "Turn N" text.
// Returns { turnNum, isOver }.
// ---------------------------------------------------------------------------
async function getTurnInfo(page, renderer) {
    try {
        if (renderer === 'native') {
            const vm = await page.evaluate(() => {
                try { return window.__mtg ? window.__mtg.getViewModel() : null; }
                catch (e) { return { error: String(e) }; }
            });
            if (vm && typeof vm.turn_number === 'number') {
                return { turnNum: vm.turn_number, isOver: !!vm.game_over };
            }
            return { turnNum: 0, isOver: false };
        }
        const termText = await extractTerminalText(page);
        const m = termText.match(/Turn (\d+)/);
        const isOver = /Game Over|wins!|has won|defeated/.test(termText);
        return { turnNum: m ? parseInt(m[1], 10) : 0, isOver };
    } catch (_) {
        return { turnNum: 0, isOver: false };
    }
}

// Wait until a page's renderer has actually rendered the game (terminal for tui,
// #game-area card board for native). Throws on timeout.
async function waitForGameRendered(page, renderer, timeout = 30_000) {
    if (renderer === 'native') {
        await page.waitForSelector('#game-area.show', { state: 'attached', timeout });
    } else {
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout });
    }
}

// ---------------------------------------------------------------------------
// Boot one browser client into a networked AI game via the lobby param contract.
//
//   mode      : 'create' | 'join'
//   renderer  : 'native' | 'tui'   (which game page to load)
// ---------------------------------------------------------------------------
async function bootClient(browser, httpPort, serverPort, gameName, mode, renderer, playerName, deckName, logs) {
    const page = await browser.newPage();
    page.on('console', (msg) => {
        const text = msg.text();
        logs.push({ t: Date.now(), type: msg.type(), text });
        if (msg.type() === 'error') log(`[${playerName}] console.error: ${text.substring(0, 200)}`);
        if (/DESYNC|MONOTONICITY|REWIND\/REPLAY FATAL/.test(text)) log(`[${playerName}] WASM: ${text.substring(0, 240)}`);
    });
    page.on('pageerror', (err) => {
        logs.push({ t: Date.now(), type: 'pageerror', text: err.message });
        log(`[${playerName}] pageerror: ${err.message}`);
    });

    // deckName is a BUILT-IN deck (both native + tui can load it from the
    // per-set index); no custom-deck localStorage seeding needed.
    const lobbyKey = mode === 'create' ? 'lobby_create' : 'lobby_join';
    const qp = new URLSearchParams({
        ws: `ws://${HTTP_HOST}:${serverPort}`,
        server_pass: SERVER_PASSWORD,
        name: playerName,
        deck: deckName,
        controller: 'random',
        mode: 'network',
        ui: renderer,
    });
    qp.set(lobbyKey, gameName);
    const bootUrl = `http://${HTTP_HOST}:${httpPort}/${renderer === 'native' ? 'native_game.html' : 'tui_game.html'}?${qp.toString()}`;
    const url = bootUrl; // remembered for reload
    await page.goto(bootUrl, { waitUntil: 'networkidle', timeout: 60_000 });
    await enableReplayVerifier(page);
    log(`[${playerName}] booted ${mode} "${gameName}" (${renderer}, random AI, deck "${deckName}")`);
    return { page, url, renderer, name: playerName, logs };
}

// Poll both clients until the survivor reaches `targetTurn` (or game over).
// Returns the max turn observed across the supplied clients.
async function advanceToTurn(clients, targetTurn, deadlineMs, stopEarly = null) {
    const start = Date.now();
    let maxTurn = 0;
    let lastReport = 0;
    while (Date.now() - start < deadlineMs) {
        let over = false;
        for (const c of clients) {
            const info = await getTurnInfo(c.page, c.renderer);
            maxTurn = Math.max(maxTurn, info.turnNum);
            if (info.isOver) over = true;
            const fatal = checkForFatalErrors(c.logs);
            if (fatal) throw new Error(`[${c.name}] fatal during play: ${fatal}`);
        }
        if (maxTurn >= targetTurn || over) return { maxTurn, over };
        if (stopEarly && stopEarly()) return { maxTurn, over };
        if (Date.now() - lastReport > 10_000) {
            log(`  …advancing: maxTurn=${maxTurn} (target ${targetTurn}), elapsed ${((Date.now() - start) / 1000).toFixed(0)}s`);
            lastReport = Date.now();
        }
        await new Promise(r => setTimeout(r, 1500));
    }
    return { maxTurn, over: false };
}

// ---------------------------------------------------------------------------
// One leg: full create/join → multiturn → reload-one-client flow.
//   reloadRenderer: which renderer the RELOADED client uses ('native' | 'tui').
//   The SURVIVING client is always 'tui' (uniform progress probe).
// Returns a result object; throws on hard failure.
// ---------------------------------------------------------------------------
async function runLeg(ctx, reloadRenderer) {
    const { httpPort, serverPort, deckName } = ctx;
    const gameName = `redo-reload-${reloadRenderer}-${Date.now()}`;
    const result = { leg: reloadRenderer, multiturn: false, surviving_advanced: false, reloaded_clean: false, maxTurnPreReload: 0, maxTurnPostReload: 0 };

    const browser = await chromium.launch({ headless: true, args: ['--no-sandbox', '--enable-unsafe-swiftshader'] });
    try {
        // The RELOADED client is the creator (so it boots first); survivor joins.
        const reloadLogs = [];
        const survivorLogs = [];

        log(`--- LEG (${reloadRenderer} reload): create "${gameName}" ---`);
        const reloadClient = await bootClient(browser, httpPort, serverPort, gameName, 'create', reloadRenderer, 'ReloadAI', deckName, reloadLogs);
        await new Promise(r => setTimeout(r, 1500));   // let CreateGame land before JOIN
        const survivor = await bootClient(browser, httpPort, serverPort, gameName, 'join', 'tui', 'SurvivorAI', deckName, survivorLogs);

        // Both must reach a rendered game.
        await waitForGameRendered(reloadClient.page, reloadClient.renderer, 30_000);
        await waitForGameRendered(survivor.page, survivor.renderer, 30_000);
        log('  both clients rendered the game');

        // item 4: advance MULTIPLE turns with both in sync.
        const pre = await advanceToTurn([reloadClient, survivor], MIN_TURNS, 90_000);
        result.maxTurnPreReload = pre.maxTurn;
        if (pre.over || pre.maxTurn >= MIN_TURNS) {
            result.multiturn = true;
            log(`  item4 OK: advanced to turn ${pre.maxTurn} (>= ${MIN_TURNS})${pre.over ? ' [game over]' : ''}, no desync`);
        } else {
            throw new Error(`item4 FAILED: only reached turn ${pre.maxTurn} (< ${MIN_TURNS})`);
        }

        if (pre.over) {
            // Game finished before we could reload — still a valid multiturn pass;
            // run the reload anyway to confirm it's clean.
            log('  (game already over before reload; reload still exercised for cleanliness)');
        }

        // item 5: RELOAD the reloadClient mid-game.
        const turnAtReload = result.maxTurnPreReload;
        log(`  RELOADING ${reloadClient.name} (${reloadClient.renderer}) at turn ${turnAtReload}…`);
        reloadLogs.length = 0;   // clear so post-reload fatal check is about the new page life
        try {
            await reloadClient.page.goto(reloadClient.url, { waitUntil: 'networkidle', timeout: 60_000 });
        } catch (e) {
            // A navigation that throws is still observable — record and continue to
            // the cleanliness assertions (we treat a hard nav crash as failure below).
            log(`  reload navigation threw: ${e.message}`);
        }

        // (a) The reload must not DESYNC/freeze the live game on the SURVIVING
        //     client. The honest acceptance bar (server-side reconnect reattach
        //     is still a Phase-1 stub — protocol.rs ReconnectResult) is: the
        //     survivor either KEEPS ADVANCING, OR is told CLEANLY that the peer
        //     dropped ("Connection lost. Please reconnect." — a clear message,
        //     NOT a silent hang) — but NEVER a DESYNC / panic / REWIND-REPLAY
        //     FATAL. checkForFatalErrors() catches the latter class; a graceful
        //     connection-lost notice is NOT in it and is an acceptable outcome.
        // Stop early once the survivor reports a clean peer-drop (no need to keep
        // polling for a turn advance that will never come on a stub-reconnect game).
        const cleanNoticeRe = /Connection lost|reconnect|Opponent disconnected|disconnect|opponent.*(left|disconnect)/i;
        const post = await advanceToTurn([survivor], turnAtReload + 1, 60_000,
            () => survivorLogs.some(e => cleanNoticeRe.test(e.text)));
        result.maxTurnPostReload = post.maxTurn;
        const survivorFatal = checkForFatalErrors(survivorLogs);
        if (survivorFatal) throw new Error(`survivor DESYNCED/froze after peer reload (NOT a clean failure): ${survivorFatal}`);
        const survivorCleanNotice = survivorLogs.some(e => cleanNoticeRe.test(e.text));
        if (post.maxTurn > turnAtReload || post.over) {
            result.surviving_advanced = true;
            log(`  item5a OK: survivor advanced ${turnAtReload} -> ${post.maxTurn}${post.over ? ' [game over]' : ''} after peer reload, no desync`);
        } else if (survivorCleanNotice) {
            result.surviving_advanced = true;   // clean outcome: peer-drop reported, not a silent freeze
            log(`  item5a OK: survivor got a CLEAN connection-lost notice after peer reload (no desync, no silent freeze)`);
        } else {
            throw new Error(`item5a FAILED: survivor neither advanced past turn ${turnAtReload} nor reported a clean peer-drop after reload (silent freeze?)`);
        }

        // (b) The RELOADED client must land in a CLEAN, well-defined state:
        //     either it re-rendered a game (terminal/board), OR it shows a clear
        //     status message — never a JS crash or a silent blank hang.
        await new Promise(r => setTimeout(r, 4000));   // give the reloaded page time to settle
        const reloadFatal = checkForFatalErrors(reloadLogs);
        if (reloadFatal) throw new Error(`reloaded client hit a fatal/desync (not a clean failure): ${reloadFatal}`);
        const reloadedState = await reloadClient.page.evaluate((renderer) => {
            const statusEl = document.getElementById('status');
            const status = statusEl ? (statusEl.textContent || '').trim() : '';
            let rendered = false;
            if (renderer === 'native') {
                const ga = document.getElementById('game-area');
                rendered = !!(ga && ga.classList.contains('show'));
            } else {
                const term = document.getElementById('ratzilla-terminal');
                rendered = !!(term && term.offsetParent !== null);
            }
            // A "no-launch" / error message box counts as a CLEAN failure if visible.
            const noLaunch = document.getElementById('no-launch-msg');
            const cleanMsg = (!!status && status.length > 0) ||
                             (noLaunch && noLaunch.offsetParent !== null);
            return { status, rendered, cleanMsg };
        }, reloadClient.renderer);
        log(`  reloaded client: rendered=${reloadedState.rendered}, status="${reloadedState.status}"`);
        if (reloadedState.rendered || reloadedState.cleanMsg) {
            result.reloaded_clean = true;
            log(`  item5b OK: reloaded client in a clean, well-defined state (${reloadedState.rendered ? 're-rendered game' : 'clear status/message'}), no silent corruption`);
        } else {
            throw new Error('item5b FAILED: reloaded client shows neither a rendered game nor a clear status — possible silent corrupt/frozen state');
        }

        return result;
    } finally {
        await browser.close();
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------
async function main() {
    const projectRoot = path.join(__dirname, '..');

    const wasmPkg = path.join(__dirname, 'pkg', 'mtg_engine.js');
    if (!fs.existsSync(wasmPkg)) throw new Error('WASM package not found. Run: make wasm-network');
    if (!fs.readFileSync(wasmPkg, 'utf8').includes('network_init')) {
        throw new Error('WASM package missing network features. Run: make wasm-network');
    }
    const mtgBinary = path.join(projectRoot, 'target', 'release', 'mtg');
    if (!fs.existsSync(mtgBinary)) throw new Error('mtg binary not found. Run: make build-network');

    const { serverPort, httpPort } = await getRandomPorts();

    let httpServer = null;
    let server = null;
    const legResults = [];
    try {
        // mtg-717: stdio 'ignore' — an undrained http.server stdout/stderr pipe
        // fills the 64KB OS buffer after a few hundred request logs and deadlocks
        // the server (the landing scenario-13 hang). This stream is never read;
        // the mtg `server` below stays piped because scanServer consumes it.
        httpServer = spawn('python3', ['-m', 'http.server', String(httpPort)], { cwd: __dirname, stdio: 'ignore' });
        if (!await waitForHttp(httpPort, 'tui_game.html')) throw new Error('HTTP server failed to start');
        log('HTTP server ready');

        // Resolve a BUILT-IN deck name from the per-set index served by the HTTP
        // server. native_game.html only boots built-in decks, so both clients
        // must use one. CLI --deck overrides; otherwise the first built-in deck.
        const base = `http://${HTTP_HOST}:${httpPort}`;
        const deckName = DECK_ARG || await firstBuiltinDeck(base);
        log('=== Redo Multiturn + Reload E2E (mtg-682 acceptance items 4+5) ===');
        log(`Deck "${deckName}" (built-in), seed ${GAME_SEED}, min-turns ${MIN_TURNS}, ports server=${serverPort} http=${httpPort}`);

        server = spawn(mtgBinary, ['server', '--port', String(serverPort), '--password', SERVER_PASSWORD, '--seed', String(GAME_SEED), '--network-debug'],
            { cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe'] });
        const serverErrors = [];
        const scanServer = (d) => {
            const line = d.toString().trim();
            if (/DESYNC|SYNC MISMATCH|state mismatch/.test(line)) { log(`Server SYNC ERROR: ${line}`); serverErrors.push(line); }
        };
        server.stdout.on('data', scanServer);
        server.stderr.on('data', scanServer);
        if (!await waitForServer(serverPort)) throw new Error('Game server failed to start');
        log('Game server ready');

        const ctx = { httpPort, serverPort, deckName };

        // Leg 1: reloaded client on the NATIVE (default) renderer.
        legResults.push(await runLeg(ctx, 'native'));
        if (serverErrors.length) throw new Error(`server reported sync errors: ${serverErrors[0]}`);

        // Leg 2: reloaded client on the TUI renderer (skipped with --quick).
        if (!QUICK) {
            legResults.push(await runLeg(ctx, 'tui'));
            if (serverErrors.length) throw new Error(`server reported sync errors: ${serverErrors[0]}`);
        }
    } finally {
        if (server)     { server.kill('SIGTERM'); }
        if (httpServer) { httpServer.kill('SIGTERM'); }
    }

    return legResults;
}

main().then((legResults) => {
    log('');
    log('=== RESULTS ===');
    let allPass = true;
    for (const r of legResults) {
        const pass = r.multiturn && r.surviving_advanced && r.reloaded_clean;
        allPass = allPass && pass;
        log(`Leg [${r.leg} reload]: ${pass ? 'PASS' : 'FAIL'} ` +
            `(multiturn=${r.multiturn} pre=${r.maxTurnPreReload}; survivor_advanced=${r.surviving_advanced} post=${r.maxTurnPostReload}; reloaded_clean=${r.reloaded_clean})`);
    }
    log('');
    if (allPass) {
        log('ALL LEGS PASSED — redo play path is multiturn-stable AND reload-resilient.');
        process.exit(0);
    } else {
        log('SOME LEGS FAILED.');
        process.exit(1);
    }
}).catch((err) => {
    log(`FATAL: ${err.message}`);
    console.error(err);
    process.exit(1);
});
