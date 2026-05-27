// Playwright-driven QA exercise for the landing page + lobby UI.
//
// Self-managed: this script spawns its own http.server and `mtg server` on
// random ports unless MTG_QA_BASE is set, so it can be invoked directly from
// `make validate-network-e2e-step`.
//
// Override with:
//   MTG_QA_BASE=http://localhost:8080 MTG_QA_WS=ws://localhost:17810 \
//     node web/test_landing_page_ux.js
//
// Screenshots are written to web/screenshots/landing_page_qa/.
// Findings (if any) are written to web/screenshots/landing_page_qa_findings.json.
// Process exits non-zero when any BLOCKING or MAJOR finding is recorded.

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

let BASE = process.env.MTG_QA_BASE || null;
let WS_OVERRIDE = process.env.MTG_QA_WS || null;
const SHOTS = path.join(__dirname, 'screenshots', 'landing_page_qa');
fs.mkdirSync(SHOTS, { recursive: true });

const findings = [];
function record(severity, scenario, msg) {
    const line = `[${severity}] ${scenario}: ${msg}`;
    console.log(line);
    findings.push({ severity, scenario, msg });
}

async function shot(page, name) {
    const p = path.join(SHOTS, name);
    await page.screenshot({ path: p, fullPage: true });
    console.log(`  shot: ${name}`);
}

async function waitForLobbyConnected(page, timeoutMs = 8000) {
    await page.waitForFunction(
        () => document.getElementById('ws-state').textContent.trim() === 'Connected',
        null,
        { timeout: timeoutMs },
    );
}

async function scenarioFullFlow() {
    console.log('\n=== Scenario: full lobby flow (alice + bob) ===');
    const browser = await chromium.launch();

    // --- Alice ---
    const aliceCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const alice = await aliceCtx.newPage();
    alice.on('pageerror', (e) => record('major', 'alice page error', e.message));
    alice.on('console', (m) => {
        if (m.type() === 'error') record('minor', 'alice console.error', m.text());
    });

    await alice.goto(global.__landingRoot || (BASE + '/'));
    await alice.waitForLoadState('domcontentloaded');
    await shot(alice, 'landing_01_initial.png');

    // Conn state check
    try {
        await waitForLobbyConnected(alice);
    } catch (e) {
        record('blocking', 'lobby ws connect', 'never reached Connected: ' + e.message);
    }

    // Verify username form is visible.
    const nameInputVisible = await alice.isVisible('#username');
    if (!nameInputVisible) record('blocking', 'username pane', 'username input not visible');

    // Submit "alice".
    await alice.fill('#username', 'alice');
    await alice.click('#btn-name');
    await alice.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 }).catch(() =>
        record('blocking', 'username submit', 'lobby pane never revealed for alice'),
    );
    await shot(alice, 'landing_02_username_entered.png');

    // Verify welcome name + games table shows empty.
    const welcome = await alice.textContent('#welcome-name');
    if (welcome !== 'alice') record('minor', 'welcome name', `expected alice, got "${welcome}"`);

    // Create game "qa-test-game" with passcode "secret".
    await alice.fill('#create-game', 'qa-test-game');
    await alice.fill('#create-pass', 'secret');

    // NEW (mtg-i1ye3 fix): Create no longer redirects. It sends a
    // ClientMessage::CreateGame over the open lobby WebSocket and switches
    // the page to a "Waiting for opponent…" pane in-place.
    await alice.click('#btn-create');
    await alice.waitForSelector('#pane-waiting:not(.hidden)', { timeout: 4000 }).catch(() =>
        record('blocking', 'create flow', 'waiting pane never appeared after create'),
    );
    await alice.waitForFunction(
        () => /waiting for opponent|registered on server/i.test(
            document.getElementById('waiting-status').textContent,
        ),
        null,
        { timeout: 4000 },
    ).catch(() => record('major', 'create flow', 'server never acked game_created / waiting_for_opponent'));
    await shot(alice, 'landing_03_game_created.png');

    const aliceUrl = alice.url();
    console.log('  alice now at:', aliceUrl);
    // Alice should NOT have navigated away — the lobby connection is what
    // holds her game slot on the server.
    if (aliceUrl.includes('native_game.html')) {
        record('major', 'create flow', 'alice was redirected away — that would drop the WS slot');
    }

    // --- Bob ---
    const bobCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const bob = await bobCtx.newPage();
    bob.on('pageerror', (e) => record('major', 'bob page error', e.message));

    await bob.goto(global.__landingRoot || (BASE + '/'));
    await bob.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(bob); } catch (e) {
        record('blocking', 'bob lobby connect', e.message);
    }

    await bob.fill('#username', 'bob');
    await bob.click('#btn-name');
    await bob.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 });

    // Refresh the list explicitly and then look for qa-test-game.
    await bob.click('#btn-refresh');
    // Wait for any games_list reply to flush in.
    await bob.waitForTimeout(1500);

    const gameRows = await bob.$$eval('#games-tbody tr', (rows) =>
        rows.map((r) => r.textContent.trim()),
    );
    console.log('  bob sees rows:', gameRows);

    const sawQaGame = gameRows.some((t) => t.includes('qa-test-game'));
    if (!sawQaGame) {
        record(
            'blocking',
            'lobby visibility',
            'qa-test-game NOT visible in bob lobby — confirms create-game intent is dropped (native_game.html ignores lobby params)',
        );
    }
    await shot(bob, 'landing_04_join_wrong_passcode.png');

    // --- Test username uniqueness check by trying to take "alice" as a third user ---
    const charlieCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const charlie = await charlieCtx.newPage();
    await charlie.goto(global.__landingRoot || (BASE + '/'));
    try { await waitForLobbyConnected(charlie); } catch (e) {}
    await charlie.fill('#username', 'alice');
    await charlie.click('#btn-name');
    await charlie.waitForTimeout(800);
    const charlieInLobby = await charlie.isVisible('#pane-lobby:not(.hidden)');
    // Now that alice has a real waiting game, the best-effort uniqueness
    // check fires against `creator_name`. Charlie's lobby SHOULD reject.
    // (True server-side uniqueness is still tracked in mtg-6uuto.)
    if (charlieInLobby) {
        record(
            'major',
            'username uniqueness',
            'charlie entered as "alice" even though alice hosts a waiting game — best-effort nameIsTaken failed',
        );
    }

    // --- Try create with empty game name ---
    await bob.fill('#create-game', '');
    await bob.fill('#create-pass', '');
    await bob.click('#btn-create');
    await bob.waitForTimeout(300);
    const stillOnLobby = !bob.url().includes('native_game.html') && !bob.url().includes('tui_game.html');
    if (!stillOnLobby) {
        record('major', 'create empty name', 'empty game name allowed (should be blocked)');
    }

    // --- Try create with valid name but NO passcode ---
    // (bob's lobby pane should still be active here — empty-name attempt above
    // does nothing and leaves him in lobby.)
    await bob.fill('#create-game', 'open-game');
    await bob.fill('#create-pass', '');
    await bob.click('#btn-create');
    await bob.waitForSelector('#pane-waiting:not(.hidden)', { timeout: 4000 }).catch(() =>
        record('blocking', 'create no-pass', 'waiting pane never appeared'),
    );
    const bobAfterCreate = bob.url();
    console.log('  bob after create (no pass):', bobAfterCreate);
    if (bobAfterCreate.includes('native_game.html')) {
        record('major', 'create no-pass', 'bob redirected away — slot would drop');
    }

    await shot(bob, 'landing_05_joined.png');

    await aliceCtx.close();
    await bobCtx.close();
    await charlieCtx.close();
    await browser.close();
}

async function scenarioMobileViewport() {
    console.log('\n=== Scenario: mobile viewport 375x667 ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 375, height: 667 } });
    const page = await ctx.newPage();
    await page.goto(global.__landingRoot || (BASE + '/'));
    await page.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(page); } catch (e) {}
    await shot(page, 'landing_06_mobile_initial.png');
    await page.fill('#username', 'mobile-user');
    await page.click('#btn-name');
    await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 }).catch(() => {});
    await shot(page, 'landing_07_mobile_lobby.png');
    await ctx.close();
    await browser.close();
}

async function scenarioOfflineLobby() {
    console.log('\n=== Scenario: lobby with server DOWN (override ws URL to dead port) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    await page.goto(BASE + '/?ws=ws://localhost:9/'); // discard port
    await page.waitForTimeout(2000);
    await shot(page, 'landing_08_ws_down.png');
    const stateText = await page.textContent('#ws-state');
    console.log('  ws state with dead server:', stateText);
    if (!/error|Disconnected|Cannot/i.test(stateText)) {
        record('minor', 'offline ws', `expected error/disconnected state, got "${stateText}"`);
    }
    await ctx.close();
    await browser.close();
}

async function scenarioLaunchPagesSmoke() {
    console.log('\n=== Scenario: launch-pages smoke (native_game / tui_game / demo) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();

    const targets = [
        { url: BASE + '/native_game.html', shot: 'landing_09_native_game.png' },
        { url: BASE + '/tui_game.html', shot: 'landing_10_tui_game.png' },
        { url: BASE + '/demo.html', shot: 'landing_11_demo.png' },
    ];
    for (const t of targets) {
        try {
            const resp = await page.goto(t.url, { waitUntil: 'domcontentloaded', timeout: 10000 });
            const status = resp ? resp.status() : 'n/a';
            console.log(`  ${t.url} → status ${status}`);
            if (resp && resp.status() >= 400) {
                record('major', 'launch page', `${t.url} returned ${status}`);
            }
            await page.waitForTimeout(1500); // let any JS settle
            await shot(page, t.shot);
        } catch (e) {
            record('major', 'launch page', `${t.url} navigation failed: ${e.message}`);
        }
    }
    await ctx.close();
    await browser.close();
}

async function scenarioAccessibility() {
    console.log('\n=== Scenario: accessibility / form labels ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    await page.goto(global.__landingRoot || (BASE + '/'));
    await page.waitForLoadState('domcontentloaded');

    // Username input should have an associated label.
    const usernameLabel = await page.$eval('label[for=username]', (el) => el.textContent.trim()).catch(() => null);
    if (!usernameLabel) record('minor', 'a11y label', 'username input missing <label for>');

    // Tab from initial focus and see if username gets focus.
    await page.keyboard.press('Tab');
    const focused = await page.evaluate(() => document.activeElement && document.activeElement.id);
    console.log('  initial focus after Tab:', focused);

    await ctx.close();
    await browser.close();
}

function pickPort() {
    return new Promise((resolve, reject) => {
        const net = require('net');
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
    const net = require('net');
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

async function startSelfManagedServers() {
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
    // Surface server stderr in case of crash; quiet on normal info logs.
    mtgProc.stderr.on('data', (d) => { /* swallow info */ });
    const httpOk = await waitForTcp(httpPort, '127.0.0.1', 10000);
    const mtgOk = await waitForTcp(mtgPort, '127.0.0.1', 10000);
    if (!httpOk) throw new Error('http.server failed to start on port ' + httpPort);
    if (!mtgOk) throw new Error('mtg server failed to start on port ' + mtgPort);
    BASE = 'http://localhost:' + httpPort;
    WS_OVERRIDE = 'ws://localhost:' + mtgPort;
    console.log('  spawned http on ' + httpPort + ', mtg on ' + mtgPort);
    return { httpProc, mtgProc };
}

(async () => {
    let spawned = null;
    if (!BASE) {
        spawned = await startSelfManagedServers();
    }
    // If we own the ws URL, pass it via query string so the page connects to
    // our random port instead of the default 17810.
    const baseWithWs = WS_OVERRIDE
        ? BASE + '/?ws=' + encodeURIComponent(WS_OVERRIDE)
        : BASE + '/';
    // Patch BASE so all scenarios append ?ws automatically when navigating to
    // the root. Scenarios that navigate to subpages (native_game.html etc.)
    // don't need the ws override.
    const origBase = BASE;
    BASE = origBase;  // keep subpage navigations clean
    // Override scenarioFullFlow / mobile / offline goto-root to include ws.
    global.__landingRoot = baseWithWs;
    try {
        await scenarioFullFlow();
        await scenarioMobileViewport();
        await scenarioOfflineLobby();
        await scenarioLaunchPagesSmoke();
        await scenarioAccessibility();
    } catch (e) {
        console.error('UNCAUGHT', e);
        record('blocking', 'harness', e.message);
    } finally {
        if (spawned) {
            try { spawned.httpProc.kill('SIGTERM'); } catch (e) {}
            try { spawned.mtgProc.kill('SIGTERM'); } catch (e) {}
        }
    }

    console.log('\n=== FINDINGS ===');
    for (const f of findings) {
        console.log(`[${f.severity}] ${f.scenario}: ${f.msg}`);
    }
    fs.writeFileSync(
        path.join(SHOTS, '..', 'landing_page_qa_findings.json'),
        JSON.stringify(findings, null, 2),
    );
    console.log(`\nTotal findings: ${findings.length}`);
    const fatal = findings.filter((f) => f.severity === 'blocking' || f.severity === 'major').length;
    if (fatal > 0) {
        console.error(`FAIL: ${fatal} blocking/major finding(s)`);
        process.exit(1);
    }
})();
