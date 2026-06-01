// test_redo_lobby_e2e.js — Step 1 acceptance gate (mtg-35z3s lobby redo).
//
// Asserts the Step 1 contract:
//   - Renderer selector (#lobby-ui-*) is ABSENT from the lobby.
//   - Deck picker (#wr-deck-select) is ABSENT from the waiting room.
//   - "Create" → waiting room → "Go to Launcher" navigates to
//     launcher.html?game=<name>&role=create&name=<user>&ws=<wsurl>
//     (no deck, no renderer, no lobby_create/lobby_join in URL).
//   - The launcher placeholder page loads (200, displays received params).
//
// Self-managed: spawns its own http.server + mtg server on random ports
// (same pattern as test_landing_page_ux.js).
//
// Run: cd web && node test_redo_lobby_e2e.js
// NOT in make validate yet (gated on full redo completion per mtg-35z3s).

'use strict';

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const net = require('net');

const SHOTS = path.join(__dirname, 'screenshots', 'redo_lobby_e2e');
fs.mkdirSync(SHOTS, { recursive: true });

let BASE = process.env.MTG_QA_BASE || null;
let WS_OVERRIDE = process.env.MTG_QA_WS || null;

const failures = [];
function fail(label, msg) {
    const line = `FAIL [${label}]: ${msg}`;
    console.error(line);
    failures.push({ label, msg });
}
function pass(label, msg) {
    console.log(`PASS [${label}]: ${msg || 'ok'}`);
}

async function shot(page, name) {
    const p = path.join(SHOTS, name);
    await page.screenshot({ path: p, fullPage: true });
}

function pickPort() {
    return new Promise((resolve, reject) => {
        const srv = net.createServer();
        srv.unref();
        srv.on('error', reject);
        srv.listen(0, () => {
            const p = srv.address().port;
            srv.close(() => resolve(p));
        });
    });
}

async function waitForTcp(port, host, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const ok = await new Promise((res) => {
            const s = net.createConnection({ port, host });
            s.once('connect', () => { s.end(); res(true); });
            s.once('error', () => res(false));
            setTimeout(() => { s.destroy(); res(false); }, 500);
        });
        if (ok) return true;
        await new Promise((r) => setTimeout(r, 250));
    }
    return false;
}

async function startServers() {
    const projectRoot = path.join(__dirname, '..');
    const mtgBinary = path.join(projectRoot, 'target', 'release', 'mtg');
    if (!fs.existsSync(mtgBinary)) {
        throw new Error('mtg binary not found at ' + mtgBinary + '. Run: make build-network');
    }
    const httpPort = await pickPort();
    const mtgPort = await pickPort();
    const httpProc = spawn('python3', ['-m', 'http.server', String(httpPort)], {
        cwd: __dirname, stdio: ['ignore', 'pipe', 'pipe'],
    });
    const mtgProc = spawn(mtgBinary, ['server', '--port', String(mtgPort)], {
        cwd: projectRoot, stdio: ['ignore', 'pipe', 'pipe'],
    });
    const httpOk = await waitForTcp(httpPort, '127.0.0.1', 10000);
    const mtgOk = await waitForTcp(mtgPort, '127.0.0.1', 10000);
    if (!httpOk) throw new Error('http.server failed on port ' + httpPort);
    if (!mtgOk) throw new Error('mtg server failed on port ' + mtgPort);
    BASE = 'http://localhost:' + httpPort;
    WS_OVERRIDE = 'ws://localhost:' + mtgPort;
    console.log('  http on ' + httpPort + ', mtg on ' + mtgPort);
    return { httpProc, mtgProc };
}

// ---------------------------------------------------------------------------
// Test: Step 1 lobby→launcher handoff
// ---------------------------------------------------------------------------
async function testLobbyToLauncherHandoff() {
    console.log('\n=== Test: lobby → launcher.html handoff (mtg-35z3s Step 1) ===');
    const browser = await chromium.launch();
    const rootUrl = BASE + '/?ws=' + encodeURIComponent(WS_OVERRIDE);

    // ---- (a) Renderer selector absent from lobby ----
    {
        const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
        const page = await ctx.newPage();
        await page.goto(rootUrl);
        await page.waitForLoadState('domcontentloaded');
        await page.waitForFunction(
            () => document.getElementById('ws-state').textContent.trim() === 'Connected',
            null, { timeout: 8000 },
        ).catch(() => {});

        await page.fill('#username', 'step1-alice');
        await page.click('#btn-name');
        await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 5000 });

        const lobbyUiRadio = await page.$('input[name="lobby-ui"]');
        if (lobbyUiRadio) {
            fail('renderer-absent', 'name="lobby-ui" radio still present on lobby — must be removed');
        } else {
            pass('renderer-absent', 'no lobby-ui radio found on lobby page');
        }
        const tuiRadio = await page.$('#lobby-ui-tui');
        if (tuiRadio) {
            fail('renderer-absent-tui', '#lobby-ui-tui must not exist on lobby');
        } else {
            pass('renderer-absent-tui', '#lobby-ui-tui absent');
        }
        await shot(page, '01_lobby_no_renderer_radio.png');
        await ctx.close();
    }

    // ---- (b) Create → waiting room → launcher.html?game=...&role=create&name=... ----
    {
        const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
        const page = await ctx.newPage();
        page.on('pageerror', (e) => fail('page-error', e.message));
        await page.goto(rootUrl);
        await page.waitForLoadState('domcontentloaded');
        await page.waitForFunction(
            () => document.getElementById('ws-state').textContent.trim() === 'Connected',
            null, { timeout: 8000 },
        ).catch(() => fail('ws-connect', 'lobby never connected'));

        await page.fill('#username', 'step1-creator');
        await page.click('#btn-name');
        await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 5000 })
            .catch(() => fail('lobby-appear', 'lobby pane never appeared'));

        // Deck picker must be absent from the create form.
        const deckPickerInForm = await page.$('#create-deck, select[name="deck"]');
        if (deckPickerInForm) {
            fail('deck-absent-form', 'deck picker must not appear in the Create form');
        } else {
            pass('deck-absent-form', 'no deck picker in Create form');
        }

        // Create a game.
        await page.fill('#create-game', 'step1-test-game');
        await page.fill('#create-pass', 'abc123');
        await page.click('#btn-create');
        await page.waitForSelector('#pane-waiting:not(.hidden)', { timeout: 5000 })
            .catch(() => fail('waiting-room', 'waiting room never appeared after Create'));

        // Deck picker must be absent from the waiting room.
        const deckPickerInWr = await page.$('#wr-deck-select');
        if (deckPickerInWr) {
            fail('deck-absent-wr', '#wr-deck-select must not be in the waiting room (deck moves to launcher.html)');
        } else {
            pass('deck-absent-wr', '#wr-deck-select absent from waiting room');
        }

        await shot(page, '02_waiting_room_no_deck.png');

        // Click "Go to Launcher".
        await page.click('#btn-launch-game');
        await page.waitForFunction(
            () => /launcher\.html/.test(window.location.href),
            null, { timeout: 5000 },
        ).catch(() => fail('launcher-redirect', '"Go to Launcher" did not navigate to launcher.html'));

        const url = page.url();
        console.log('  launcher URL:', url);
        const parsed = new URL(url);

        // Assert required params.
        const game = parsed.searchParams.get('game');
        const role = parsed.searchParams.get('role');
        const passParam = parsed.searchParams.get('pass');
        const name = parsed.searchParams.get('name');
        const ws = parsed.searchParams.get('ws');

        if (game === 'step1-test-game') {
            pass('param-game', 'game=step1-test-game');
        } else {
            fail('param-game', 'expected game=step1-test-game, got: ' + game);
        }
        if (role === 'create') {
            pass('param-role', 'role=create');
        } else {
            fail('param-role', 'expected role=create, got: ' + role);
        }
        if (passParam === 'abc123') {
            pass('param-pass', 'pass=abc123');
        } else {
            fail('param-pass', 'expected pass=abc123, got: ' + passParam);
        }
        if (name === 'step1-creator') {
            pass('param-name', 'name=step1-creator');
        } else {
            fail('param-name', 'expected name=step1-creator, got: ' + name);
        }
        if (ws) {
            pass('param-ws', 'ws= present');
        } else {
            fail('param-ws', 'ws= param missing from launcher URL');
        }
        // Old params must NOT appear.
        if (!parsed.searchParams.has('lobby_create')) {
            pass('no-old-lobby-create', 'lobby_create absent from launcher URL');
        } else {
            fail('no-old-lobby-create', 'lobby_create must NOT appear in launcher URL');
        }

        // The launcher placeholder must display the received params.
        await page.waitForLoadState('domcontentloaded');
        await page.waitForTimeout(300);
        const bodyText = await page.textContent('body').catch(() => '');
        if (bodyText.includes('step1-test-game')) {
            pass('launcher-content', 'launcher.html displays game name');
        } else {
            fail('launcher-content', 'launcher.html body does not contain game name "step1-test-game"');
        }
        if (bodyText.includes('step1-creator')) {
            pass('launcher-content-name', 'launcher.html displays player name');
        } else {
            fail('launcher-content-name', 'launcher.html body does not contain player name "step1-creator"');
        }

        // launcher.html exposes params as window.__launcherParams for test assertions.
        const launcherParams = await page.evaluate(() => window.__launcherParams || null);
        console.log('  window.__launcherParams:', JSON.stringify(launcherParams));
        if (launcherParams && launcherParams.game === 'step1-test-game') {
            pass('launcher-params-js', 'window.__launcherParams.game correct');
        } else {
            fail('launcher-params-js', 'window.__launcherParams.game wrong: ' + JSON.stringify(launcherParams));
        }
        if (launcherParams && launcherParams.role === 'create') {
            pass('launcher-params-role', 'window.__launcherParams.role=create');
        } else {
            fail('launcher-params-role', 'window.__launcherParams.role wrong: ' + (launcherParams && launcherParams.role));
        }

        await shot(page, '03_launcher_placeholder.png');
        await ctx.close();
    }

    // ---- (c) launcher.html reachable directly (200, not 404) ----
    {
        const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
        const page = await ctx.newPage();
        const resp = await page.goto(BASE + '/launcher.html', { waitUntil: 'domcontentloaded' });
        const status = resp ? resp.status() : 'n/a';
        if (status === 200) {
            pass('launcher-200', 'launcher.html returns 200');
        } else {
            fail('launcher-200', 'launcher.html returned status ' + status);
        }
        await ctx.close();
    }

    await browser.close();
}

(async () => {
    let spawned = null;
    if (!BASE) {
        spawned = await startServers();
    }
    try {
        await testLobbyToLauncherHandoff();
    } catch (e) {
        fail('harness', 'uncaught: ' + e.message);
        console.error(e);
    } finally {
        if (spawned) {
            try { spawned.httpProc.kill('SIGTERM'); } catch (_) {}
            try { spawned.mtgProc.kill('SIGTERM'); } catch (_) {}
        }
    }

    console.log('\n=== RESULTS ===');
    if (failures.length === 0) {
        console.log('ALL TESTS PASSED');
        process.exit(0);
    } else {
        for (const f of failures) {
            console.error('FAIL [' + f.label + ']: ' + f.msg);
        }
        console.error('\n' + failures.length + ' test(s) FAILED');
        process.exit(1);
    }
})();
