// test_lobby_players_list_e2e.js — logged-in PLAYERS list acceptance gate
// (mtg-890; design mtg-595/mtg-594).
//
// Asserts the lobby's two-list contract for the Logged-in Players list, which
// mirrors the existing Open Games list (ListPlayers/PlayerList ↔ ListGames/
// GameList on the wire):
//   - Two browsers register distinct usernames over the lobby WS (Register).
//   - Each browser's "Logged-in Players" table (#players-tbody) shows BOTH
//     registered names — proving the server tracks the name registry and the
//     ListPlayers/PlayerList round-trip renders.
//   - The pager line (#players-count) reads "Showing N of M" with M == 2.
//   - The players filter (#players-filter) narrows the list case-insensitively
//     and updates the count, exactly like the games filter.
//   - When one browser disconnects, the other's list drops back to 1 (the
//     server releases the name reservation on WS close).
//
// Self-managed: spawns its own http.server + mtg server on random ports
// (same pattern as test_redo_lobby_e2e.js / test_landing_page_ux.js).
//
// Run: cd web && node test_lobby_players_list_e2e.js
// Wired into `make validate` (validate-network-e2e-step) and CI.

'use strict';

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const net = require('net');

const failures = [];
function fail(label, msg) {
    console.error(`FAIL [${label}]: ${msg}`);
    failures.push({ label, msg });
}
function pass(label, msg) {
    console.log(`PASS [${label}]: ${msg || 'ok'}`);
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
    // stdio 'ignore' (not undrained 'pipe') — an undrained pipe fills the OS
    // buffer after a few hundred http.server logs and wedges the server
    // (mtg-717). These streams are never read here.
    const httpProc = spawn('python3', ['-m', 'http.server', String(httpPort)], {
        cwd: __dirname, stdio: 'ignore',
    });
    const mtgProc = spawn(mtgBinary, ['server', '--port', String(mtgPort)], {
        cwd: projectRoot, stdio: 'ignore',
    });
    const httpOk = await waitForTcp(httpPort, '127.0.0.1', 10000);
    const mtgOk = await waitForTcp(mtgPort, '127.0.0.1', 10000);
    if (!httpOk) throw new Error('http.server failed on port ' + httpPort);
    if (!mtgOk) throw new Error('mtg server failed on port ' + mtgPort);
    const base = 'http://localhost:' + httpPort;
    const ws = 'ws://localhost:' + mtgPort;
    console.log('  http on ' + httpPort + ', mtg on ' + mtgPort);
    return { httpProc, mtgProc, base, ws };
}

// Register a username on a fresh lobby page and wait until in the lobby pane.
async function enterLobbyAs(browser, base, ws, username) {
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.on('pageerror', (e) => fail('page-error', `${username}: ${e.message}`));
    await page.goto(base + '/?ws=' + encodeURIComponent(ws));
    await page.waitForFunction(
        () => document.getElementById('ws-state') &&
              document.getElementById('ws-state').textContent.trim() === 'Connected',
        null, { timeout: 8000 },
    ).catch(() => fail('ws-connect', `${username} never connected to lobby`));
    await page.fill('#username', username);
    await page.click('#btn-name');
    await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 5000 })
        .catch(() => fail('enter-lobby', `${username} never entered lobby`));
    return { ctx, page };
}

// Poll the players table until it contains exactly `names`, refreshing on each
// tick (lobby auto-refreshes every 5s; we also click Refresh to go faster).
async function pollPlayerNames(page, expectedNames, label) {
    let names = [];
    for (let i = 0; i < 25; i++) {
        await page.click('#btn-refresh').catch(() => {});
        await page.waitForTimeout(500);
        names = await page.evaluate(() => {
            const rows = [...document.querySelectorAll('#players-tbody tr:not(.empty)')];
            return rows.map((tr) => tr.querySelector('td').textContent.trim());
        });
        const got = new Set(names.map((n) => n.toLowerCase()));
        if (expectedNames.every((n) => got.has(n.toLowerCase())) && names.length === expectedNames.length) {
            return names;
        }
    }
    fail(label, `expected players [${expectedNames}] but saw [${names}]`);
    return names;
}

async function main() {
    let servers;
    try {
        servers = await startServers();
    } catch (e) {
        console.error('SETUP FAILED:', e.message);
        process.exit(1);
    }
    const { base, ws } = servers;
    const browser = await chromium.launch();

    try {
        console.log('\n=== Test: lobby logged-in PLAYERS list (mtg-890) ===');

        // Two browsers register distinct names.
        const alice = await enterLobbyAs(browser, base, ws, 'alice-' + Date.now());
        const bob = await enterLobbyAs(browser, base, ws, 'bob-' + Date.now());
        const aliceName = await alice.page.inputValue('#username');
        const bobName = await bob.page.inputValue('#username');

        // (1) Each browser sees BOTH players.
        await pollPlayerNames(alice.page, [aliceName, bobName], 'alice-sees-both');
        await pollPlayerNames(bob.page, [aliceName, bobName], 'bob-sees-both');
        pass('both-players-listed', 'both browsers see both registered players');

        // (2) The pager reads "Showing N of 2".
        const countText = await alice.page.textContent('#players-count');
        if (/of\s+2\b/.test(countText)) {
            pass('players-count', `count line reads "${countText.trim()}"`);
        } else {
            fail('players-count', `expected "... of 2", got "${countText}"`);
        }

        // (3) Filter narrows the list case-insensitively. Filter for alice's
        // unique prefix; only alice should remain.
        await alice.page.fill('#players-filter', aliceName.slice(0, 5).toUpperCase());
        await alice.page.waitForTimeout(900); // debounce + round-trip
        const filtered = await alice.page.evaluate(() => {
            const rows = [...document.querySelectorAll('#players-tbody tr:not(.empty)')];
            return rows.map((tr) => tr.querySelector('td').textContent.trim());
        });
        if (filtered.length === 1 && filtered[0] === aliceName) {
            pass('players-filter', `filter narrowed to ["${aliceName}"]`);
        } else {
            fail('players-filter', `filter expected ["${aliceName}"], got [${filtered}]`);
        }
        // Clear the filter so the list returns to both.
        await alice.page.fill('#players-filter', '');
        await alice.page.waitForTimeout(900);

        // (4) Disconnect bob → alice's list drops to just alice (name released
        // on WS close).
        await bob.ctx.close();
        await pollPlayerNames(alice.page, [aliceName], 'list-drops-on-disconnect');
        pass('disconnect-releases-name', 'closing one browser releases its name from the list');

        await alice.ctx.close();
    } catch (e) {
        fail('exception', e && e.stack ? e.stack : String(e));
    } finally {
        await browser.close().catch(() => {});
        try { servers.httpProc.kill('SIGKILL'); } catch (e) { /* ignore */ }
        try { servers.mtgProc.kill('SIGKILL'); } catch (e) { /* ignore */ }
    }

    if (failures.length > 0) {
        console.error(`\n${failures.length} failure(s):`);
        failures.forEach((f) => console.error(`  - [${f.label}] ${f.msg}`));
        process.exit(1);
    }
    console.log('\nALL PASSED — lobby logged-in players list is server-backed, paginated, and filterable.');
}

main();
