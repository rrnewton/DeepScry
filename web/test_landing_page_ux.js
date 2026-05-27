// Playwright-driven QA exercise for the new landing page + lobby UI
// (commit d8b2448f, branch playwright-qa-landing-page).
//
// Preconditions:
//   - Static file server at http://localhost:8080/ serving web/
//   - Native Rust lobby server at ws://localhost:17810 (`mtg server --port 17810`)
//
// Run from repo root:
//   node web/test_landing_page_ux.js
//
// Screenshots are written to web/screenshots/landing_page_qa/.
// Findings are appended to the QA report (the report itself is written by hand
// based on what this harness logs to stdout).

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');

const BASE = process.env.MTG_QA_BASE || 'http://localhost:8080';
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

    await alice.goto(BASE + '/');
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

    // The create button triggers window.location.href = native_game.html?...
    // We intercept by listening for navigation.
    const [aliceNav] = await Promise.all([
        alice.waitForEvent('framenavigated', { timeout: 4000 }).catch(() => null),
        alice.click('#btn-create'),
    ]);
    await alice.waitForLoadState('domcontentloaded').catch(() => {});
    await shot(alice, 'landing_03_game_created.png');

    const aliceUrl = alice.url();
    console.log('  alice navigated to:', aliceUrl);
    if (!aliceUrl.includes('native_game.html')) {
        record('major', 'create redirect', `expected native_game.html, got ${aliceUrl}`);
    }
    if (!aliceUrl.includes('lobby=create') || !aliceUrl.includes('game=qa-test-game')) {
        record('major', 'create redirect query', `lobby params missing: ${aliceUrl}`);
    }
    if (!aliceUrl.includes('pass=secret')) {
        record('major', 'create redirect query', `pass param missing: ${aliceUrl}`);
    }

    // CRITICAL CHECK: native_game.html does it actually do anything with the lobby params?
    // From grep, native_game.html has zero references to "lobby" / "searchParams" / "URLSearchParams".
    // So the create-game flow lands on a page that ignores the create intent entirely.
    // Verify: poll the WebSocket connections on the server (out-of-band) is hard from here,
    // but we can verify by going back to the lobby as bob and confirming NO game appears.

    // --- Bob ---
    const bobCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const bob = await bobCtx.newPage();
    bob.on('pageerror', (e) => record('major', 'bob page error', e.message));

    await bob.goto(BASE + '/');
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
    await charlie.goto(BASE + '/');
    try { await waitForLobbyConnected(charlie); } catch (e) {}
    await charlie.fill('#username', 'alice');
    await charlie.click('#btn-name');
    await charlie.waitForTimeout(500);
    const charlieInLobby = await charlie.isVisible('#pane-lobby:not(.hidden)');
    // Without a real waiting game, the uniqueness check (against creator_name of waiting games)
    // will not flag this — so charlie WILL enter the lobby as "alice".
    if (charlieInLobby) {
        record(
            'major',
            'username uniqueness',
            'Two browsers can simultaneously hold "alice" — uniqueness check only fires against waiting-game host names, not currently-connected users.',
        );
    }

    // --- Try create with empty game name ---
    await bob.fill('#create-game', '');
    await bob.fill('#create-pass', '');
    await bob.click('#btn-create');
    await bob.waitForTimeout(300);
    const stillOnLobby = bob.url().endsWith('/') || bob.url().endsWith('/index.html');
    if (!stillOnLobby) {
        record('major', 'create empty name', 'empty game name allowed (should be blocked)');
    }

    // --- Try create with valid name but NO passcode ---
    await bob.fill('#create-game', 'open-game');
    await bob.fill('#create-pass', '');
    const [bobNav] = await Promise.all([
        bob.waitForEvent('framenavigated', { timeout: 4000 }).catch(() => null),
        bob.click('#btn-create'),
    ]);
    await bob.waitForLoadState('domcontentloaded').catch(() => {});
    const bobAfterCreate = bob.url();
    console.log('  bob after create (no pass):', bobAfterCreate);
    if (!bobAfterCreate.includes('native_game.html')) {
        record('major', 'create no-pass', `expected native_game.html, got ${bobAfterCreate}`);
    }
    if (bobAfterCreate.includes('pass=')) {
        record('minor', 'create no-pass', 'pass param leaked into URL with empty value');
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
    await page.goto(BASE + '/');
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
    await page.goto(BASE + '/');
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

(async () => {
    try {
        await scenarioFullFlow();
        await scenarioMobileViewport();
        await scenarioOfflineLobby();
        await scenarioLaunchPagesSmoke();
        await scenarioAccessibility();
    } catch (e) {
        console.error('UNCAUGHT', e);
        record('blocking', 'harness', e.message);
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
})();
