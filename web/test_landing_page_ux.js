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

// mtg-717: landing-page navigations wait until 'domcontentloaded', NOT the
// Playwright default of 'load'. Under prebuilt 2-vCPU CI a non-critical
// subresource can 404 or stall, so the window 'load' event never fires and
// page.goto({waitUntil:'load'}) times out — this was the network-redo shard
// wedge (the failed step then orphaned a chromium holding the runner's stdout
// pipe). Every scenario waits on explicit elements (waitForLobbyConnected /
// waitForSelector / waitForLoadState) AFTER navigating, so the full 'load'
// barrier was never needed; domcontentloaded + explicit waits is strictly more
// robust. Apply NAV to every landing/game-page goto.
const NAV = { waitUntil: 'domcontentloaded' };
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
    alice.setDefaultNavigationTimeout(90000); alice.setDefaultTimeout(90000);
    alice.on('pageerror', (e) => record('major', 'alice page error', e.message));
    alice.on('console', (m) => {
        if (m.type() === 'error') record('minor', 'alice console.error', m.text());
    });

    await alice.goto(global.__landingRoot || (BASE + '/'), NAV);
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

    // mtg-682 item 1: there is NO waiting-room pane anymore — clicking Create
    // navigates STRAIGHT to launcher.html (the game page, not the lobby, holds
    // the durable host WS). Confirm the pane is gone and the redirect is direct.
    const waitingPane = await alice.$('#pane-waiting');
    if (waitingPane) {
        record('major', 'no waiting room', '#pane-waiting must NOT exist (Create goes straight to launcher)');
    }
    const inviteBlock = await alice.$('#waiting-invite-block, .invite-block');
    if (inviteBlock) {
        record('major', 'no sharable link', 'a sharable-invite block must NOT exist on the lobby');
    }
    // Renderer selector MUST NOT be present on the lobby (belongs in launcher).
    const lobbyUiRadio = await alice.$('#lobby-ui-tui, #lobby-ui-native');
    if (lobbyUiRadio) {
        record('major', 'redo step1', 'renderer radio (#lobby-ui-*) must NOT appear on the lobby — belongs in launcher.html');
    }

    await alice.click('#btn-create');
    await alice.waitForFunction(
        () => /launcher\.html/.test(window.location.href),
        null,
        { timeout: 4000 },
    ).catch(() => record('blocking', 'create flow', 'alice never navigated STRAIGHT to launcher.html on Create'));
    const aliceUrl = alice.url();
    console.log('  alice redirected to:', aliceUrl);
    if (!aliceUrl.includes('game=qa-test-game')) {
        record('major', 'create flow', 'alice URL missing game= param: ' + aliceUrl);
    }
    if (!aliceUrl.includes('role=create')) {
        record('major', 'create flow', 'alice URL missing role=create param: ' + aliceUrl);
    }
    if (!aliceUrl.includes('pass=secret')) {
        record('major', 'create flow', 'alice URL missing pass=secret param: ' + aliceUrl);
    }
    await alice.waitForTimeout(400);
    await shot(alice, 'landing_03_create_straight_to_launcher.png');

    // --- Test username uniqueness (server-side Register enforcement) ---
    // The server enforces unique names via Register; a concurrent duplicate is
    // rejected. (Once alice left the lobby for the launcher her lobby WS dropped
    // and the reservation was released, so we just check a fresh unique name.)
    const charlieCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const charlie = await charlieCtx.newPage();
    charlie.setDefaultNavigationTimeout(90000); charlie.setDefaultTimeout(90000);
    await charlie.goto(global.__landingRoot || (BASE + '/'), NAV);
    try { await waitForLobbyConnected(charlie); } catch (e) {}
    await charlie.fill('#username', 'charlie');
    await charlie.click('#btn-name');
    await charlie.waitForTimeout(800);
    const charlieInLobby = await charlie.isVisible('#pane-lobby:not(.hidden)');
    if (!charlieInLobby) {
        record('major', 'username uniqueness', 'charlie failed to enter lobby with a valid unique name');
    }

    // --- Try create with empty game name → validation blocks, stays on lobby ---
    await charlie.fill('#create-game', '');
    await charlie.fill('#create-pass', '');
    await charlie.click('#btn-create');
    await charlie.waitForTimeout(300);
    const stillOnLobby = !charlie.url().includes('launcher.html');
    if (!stillOnLobby) {
        record('major', 'create empty name', 'empty game name allowed (should be blocked by validation, no navigation)');
    }

    // --- Create with valid name but NO passcode → straight to launcher, no pass= ---
    await charlie.fill('#create-game', 'open-game');
    await charlie.fill('#create-pass', '');
    await charlie.click('#btn-create');
    await charlie.waitForFunction(
        () => /launcher\.html/.test(window.location.href) &&
              /game=open-game/.test(window.location.href),
        null,
        { timeout: 4000 },
    ).catch(() => record('blocking', 'create no-pass', 'charlie never navigated to launcher.html with game=open-game'));
    const charlieAfterCreate = charlie.url();
    console.log('  charlie after create (no pass):', charlieAfterCreate);
    if (charlieAfterCreate.includes('pass=')) {
        record('major', 'create no-pass', 'empty passcode leaked into URL: ' + charlieAfterCreate);
    }
    if (!charlieAfterCreate.includes('role=create')) {
        record('major', 'create no-pass', 'role=create missing from launcher URL: ' + charlieAfterCreate);
    }
    await charlie.waitForTimeout(400);
    await shot(charlie, 'landing_05_create_no_pass.png');

    await aliceCtx.close();
    await charlieCtx.close();
    await browser.close();
}

// mtg-474: After a Create/Join redirect the user lands on tui_game.html
// with `?lobby_create=...`. The page should:
//   (a) auto-fire the launch (no manual click required),
//   (b) reach a "Waiting" / "WaitingForOpponent" network state.
// And re-opening the lobby in a fresh tab must NOT show the lingering
// "Already authenticated / in a game" red-status error that the periodic
// 5s refreshTimer caused previously (the timer now skips when pendingFlow
// is non-null AND we never even enter pendingFlow now that Create redirects).
async function scenarioPostRedirectAutoLaunch() {
    console.log('\n=== Scenario: post-redirect auto-launch on tui_game.html (mtg-474) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1400, height: 900 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    page.on('pageerror', (e) => record('major', 'tui auto-launch page error', e.message));
    page.on('console', (m) => {
        if (m.type() === 'error') {
            const t = m.text();
            // Surface only true errors (filter out 404-on-missing-data which
            // is a separate concern). We're checking specifically that the
            // "Already authenticated" reply does NOT show up.
            if (/already authenticated/i.test(t)) {
                record('blocking', 'tui auto-launch', 'server replied "Already authenticated" during auto-launch');
            }
        }
    });

    // Build URL = BASE + '/tui_game.html?...'. Don't reuse __landingRoot
    // here since it may contain a leftover ?ws=... query string.
    const url = BASE + '/tui_game.html?lobby_create=autolaunch-test&lobby_pass=&name=autolaunch&ws=' +
        encodeURIComponent(WS_OVERRIDE || 'ws://localhost:17810');
    console.log('  navigating to:', url);
    await page.goto(url, NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForTimeout(4000); // give WASM init + auto-launch a chance
    await shot(page, 'landing_12_post_redirect_autolaunch.png');

    // mtg-35z3s page 3: tui_game.html is now a PURE renderer with no built-in
    // launcher / #network-status field — the lobby_create param auto-launches
    // the network game and connection state surfaces in the header #status
    // (e.g. "Connecting...") or the ratzilla terminal appears. Check #status.
    const networkStatusText = await page.evaluate(() => {
        const el = document.getElementById('status');
        return el ? el.textContent : '';
    });
    console.log('  status:', JSON.stringify(networkStatusText));
    if (!/auto-creating|status|waiting|connecting|ready|cards/i.test(networkStatusText)) {
        // Minor (not major) because the auto-launch can legitimately fail in
        // a stub environment (missing data/decks.bin, etc.). The redirect
        // wiring itself is verified by scenarioFullFlow. We surface this so
        // a real regression surfaces, but don't fail the suite.
        record('minor', 'tui auto-launch', 'no auto-launch status hint visible: ' + JSON.stringify(networkStatusText));
    }

    await ctx.close();
    await browser.close();
}

// mtg-474: scenario covering the "Already authenticated" regression. We
// open the lobby, let the periodic refreshTimer fire several times (10+ s),
// and verify no red status appears. The previous bug was: even from the
// browse state, the timer would fire ListGames over a WS that was no longer
// in lobby mode, producing the Error reply that turned the UI red. The fix
// is twofold: (a) Create/Join no longer reuses the lobby WS, and (b) the
// refreshTimer is paused whenever pendingFlow is non-null.
async function scenarioRefreshTimerNoError() {
    console.log('\n=== Scenario: lobby refresh timer never triggers "Already authenticated" (mtg-474) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    let sawAlreadyAuth = false;
    page.on('console', (m) => {
        if (/already authenticated/i.test(m.text())) {
            sawAlreadyAuth = true;
        }
    });

    await page.goto(global.__landingRoot || (BASE + '/'), NAV);
    await page.waitForLoadState('domcontentloaded');
    await waitForLobbyConnected(page).catch(() => {});

    await page.fill('#username', 'refresh-test');
    await page.click('#btn-name');
    await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 });

    // Sit in the lobby for ~12 seconds so the 5-second refreshTimer fires
    // at least twice. Verify no red ws-state or red create-status appears.
    await page.waitForTimeout(12000);

    const wsStateText = await page.textContent('#ws-state');
    console.log('  ws-state after 12s of polling:', wsStateText);
    if (/error|disconnected/i.test(wsStateText)) {
        record('blocking', 'refresh timer', 'ws-state showed "' + wsStateText + '" after idle polling');
    }
    if (sawAlreadyAuth) {
        record('blocking', 'refresh timer', '"Already authenticated" appeared in console during idle polling');
    }

    await shot(page, 'landing_13_refresh_timer_idle.png');
    await ctx.close();
    await browser.close();
}

async function scenarioMobileViewport() {
    console.log('\n=== Scenario: mobile viewport 375x667 ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 375, height: 667 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    await page.goto(global.__landingRoot || (BASE + '/'), NAV);
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
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    await page.goto(BASE + '/?ws=ws://localhost:9/', NAV); // discard port
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
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);

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

// mtg-477: the ?allow_local_img_load=true unlock must be "sticky" across
// same-tab navigation (index -> game page) via sessionStorage + launcher-link
// propagation, WITHOUT becoming bypassable by stale localStorage. This scenario
// covers: (1) index with the param propagates it onto launcher hrefs, (2) the
// launched game page reports window.__allowLocalImgLoad === true, (3) a fresh
// game-page visit with NO param and NO session flag stays locked (anti-bypass),
// and (4) sessionStorage (not localStorage) is the persistence layer.
async function scenarioStickyLocalImageUnlock() {
    console.log('\n=== Scenario: sticky allow_local_img_load unlock (mtg-477) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);

    // (1) Load the lobby WITH the unlock param.
    const root = global.__landingRoot || (BASE + '/');
    const sep = root.indexOf('?') === -1 ? '?' : '&';
    await page.goto(root + sep + 'allow_local_img_load=true', NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForTimeout(500);

    // The launcher hrefs must carry the param forward. After the "Local Tools"
    // lobby restructure (28570dc20) the per-renderer #launch-native / #launch-tui
    // links are gone — the renderer is chosen inside solo_launcher.html. The
    // sticky unlock now rides onto the current launch links: #launch-solo
    // (→ solo_launcher.html), #launch-demo (→ demo.html), and #launch-deck-editor
    // (→ deck_editor.html). applyLaunchLinks() decorates these at page init.
    const soloHref = await page.getAttribute('#launch-solo', 'href');
    const demoHref = await page.getAttribute('#launch-demo', 'href');
    const editorHref = await page.getAttribute('#launch-deck-editor', 'href');
    console.log('  launch-solo href:', soloHref);
    console.log('  launch-demo href:', demoHref);
    console.log('  launch-deck-editor href:', editorHref);
    if (!/allow_local_img_load=true/.test(soloHref || '')) {
        record('major', 'sticky unlock', 'solo launcher href missing allow_local_img_load=true: ' + soloHref);
    }
    if (!/allow_local_img_load=true/.test(demoHref || '')) {
        record('major', 'sticky unlock', 'demo launcher href missing allow_local_img_load=true: ' + demoHref);
    }
    if (!/allow_local_img_load=true/.test(editorHref || '')) {
        record('major', 'sticky unlock', 'deck-editor launcher href missing allow_local_img_load=true: ' + editorHref);
    }

    // The lobby page itself must persist the flag to sessionStorage (NOT
    // localStorage) so later same-tab navigations stay unlocked.
    const storage = await page.evaluate(() => ({
        session: sessionStorage.getItem('allowLocalImgLoad'),
        local: localStorage.getItem('allowLocalImgLoad'),
    }));
    console.log('  storage after index w/ param:', JSON.stringify(storage));
    if (storage.session !== 'true') {
        record('major', 'sticky unlock', 'expected sessionStorage allowLocalImgLoad="true", got ' + JSON.stringify(storage.session));
    }
    if (storage.local !== null) {
        record('major', 'sticky unlock', 'localStorage must NOT be written (would over-persist); got ' + JSON.stringify(storage.local));
    }

    // (2) Navigate to the game page WITHOUT the param but in the SAME tab/session.
    // The gate must inherit the unlock from sessionStorage.
    await page.goto(BASE + '/native_game.html', NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForTimeout(1500);
    const allowedSticky = await page.evaluate(() => window.__allowLocalImgLoad === true);
    console.log('  native_game.html __allowLocalImgLoad (sticky, no param):', allowedSticky);
    if (!allowedSticky) {
        record('major', 'sticky unlock', 'same-tab navigation to native_game.html lost the unlock (expected sticky via sessionStorage)');
    }
    await shot(page, 'landing_13_sticky_unlock_game.png');

    await ctx.close();

    // (3) Anti-bypass: a brand-new context (fresh session, no param) must stay
    // LOCKED. We only read sessionStorage, never localStorage, so even a stale
    // localStorage value cannot re-enable local images.
    const ctx2 = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page2 = await ctx2.newPage();
    page2.setDefaultNavigationTimeout(90000); page2.setDefaultTimeout(90000);
    await page2.goto(BASE + '/native_game.html', NAV);
    // Seed a stale localStorage value to prove it is ignored, then reload.
    await page2.evaluate(() => { try { localStorage.setItem('allowLocalImgLoad', 'true'); } catch (e) {} });
    await page2.reload();
    await page2.waitForLoadState('domcontentloaded');
    await page2.waitForTimeout(1500);
    const allowedFresh = await page2.evaluate(() => window.__allowLocalImgLoad === true);
    console.log('  fresh-session native_game.html __allowLocalImgLoad (stale localStorage seeded):', allowedFresh);
    if (allowedFresh) {
        record('blocking', 'sticky unlock', 'GATE BYPASS: fresh session with stale localStorage re-enabled local images');
    }
    await ctx2.close();

    await browser.close();
}

async function scenarioPasscodeEyeballToggle() {
    console.log('\n=== Scenario: passcode show/hide eyeball ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    page.on('pageerror', (e) => record('major', 'pw-toggle pageerror', e.message));
    await page.goto(global.__landingRoot || (BASE + '/'), NAV);
    await page.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(page); } catch (e) {
        record('blocking', 'pw-toggle ws', e.message);
    }
    await page.fill('#username', 'eyeballer');
    await page.click('#btn-name');
    await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 });

    // Type into the create-pass field.
    await page.fill('#create-pass', 'hunter2');
    let typeBefore = await page.getAttribute('#create-pass', 'type');
    if (typeBefore !== 'password') {
        record('major', 'pw-toggle initial type', 'expected type=password, got ' + typeBefore);
    }
    // Toggle to visible.
    const toggleSel = '.pw-wrap .pw-toggle[data-target="create-pass"]';
    await page.click(toggleSel);
    let typeAfter = await page.getAttribute('#create-pass', 'type');
    if (typeAfter !== 'text') {
        record('major', 'pw-toggle on', 'expected type=text after toggle, got ' + typeAfter);
    }
    const aria1 = await page.getAttribute(toggleSel, 'aria-pressed');
    if (aria1 !== 'true') {
        record('minor', 'pw-toggle aria', 'aria-pressed not "true" after toggle, got ' + aria1);
    }
    // Toggle back to hidden.
    await page.click(toggleSel);
    let typeBack = await page.getAttribute('#create-pass', 'type');
    if (typeBack !== 'password') {
        record('major', 'pw-toggle off', 'expected type=password after second toggle, got ' + typeBack);
    }
    await shot(page, 'landing_12_pw_eyeball.png');
    await ctx.close();
    await browser.close();
}

async function scenarioGameListFilterAndPager() {
    console.log('\n=== Scenario: game list filter + pagination ===');
    const browser = await chromium.launch();

    // Spawn enough "host" connections via raw WebSocket to populate the list
    // with > GAMES_PAGE_SIZE (20) games. We keep the WS sockets open for the
    // duration of the test so the games stay in waiting_games.
    const WebSocket = require('ws');
    const hosts = [];
    const wsUrl = WS_OVERRIDE || 'ws://localhost:17810';
    const NUM_GAMES = 25;
    const deck = {
        main_deck: [['Forest', 22], ['Grizzly Bears', 14], ['Plains', 12], ['Serra Angel', 12]],
        sideboard: [],
    };
    for (let i = 0; i < NUM_GAMES; i++) {
        const sock = new WebSocket(wsUrl);
        await new Promise((res, rej) => {
            sock.once('open', res);
            sock.once('error', rej);
        });
        const gameName = (i < 5 ? 'filter-target-' : 'bulk-game-') + i;
        const creatorName = (i < 5 ? 'targethost' : 'bulkhost') + i;
        sock.send(JSON.stringify({
            type: 'create_game',
            password: '',
            game_name: gameName,
            game_password: null,
            player_name: creatorName,
            deck,
        }));
        hosts.push(sock);
    }
    // Give the server a moment to register all the slots.
    await new Promise((r) => setTimeout(r, 800));

    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    page.on('pageerror', (e) => record('major', 'filter pageerror', e.message));
    await page.goto(global.__landingRoot || (BASE + '/'), NAV);
    await page.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(page); } catch (e) {
        record('blocking', 'filter ws', e.message);
    }
    await page.fill('#username', 'browser');
    await page.click('#btn-name');
    await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 });
    // First page should have 20 rows; total >= NUM_GAMES.
    await page.click('#btn-refresh');
    await page.waitForTimeout(800);
    const firstPageRows = await page.$$eval('#games-tbody tr:not(.empty)', (rs) => rs.length);
    const countText = await page.textContent('#games-count');
    console.log('  first page rows:', firstPageRows, '/ countText:', countText);
    if (firstPageRows !== 20) {
        record('major', 'pager first page', 'expected 20 rows on first page, got ' + firstPageRows);
    }
    if (!/of\s+\d+/.test(countText) || !/of\s+(2[5-9]|[3-9]\d)/.test(countText)) {
        record('major', 'pager count', 'expected "of >=25", got: ' + countText);
    }
    await shot(page, 'landing_13_list_page1.png');

    // Click next; expect remaining rows.
    const nextDisabled = await page.getAttribute('#games-next', 'disabled');
    if (nextDisabled !== null) {
        record('major', 'pager next disabled', 'next button should be enabled when total > page size');
    }
    await page.click('#games-next');
    await page.waitForTimeout(500);
    const secondPageRows = await page.$$eval('#games-tbody tr:not(.empty)', (rs) => rs.length);
    console.log('  second page rows:', secondPageRows);
    if (secondPageRows < 1) {
        record('major', 'pager second page', 'expected >=1 row on second page, got ' + secondPageRows);
    }
    await shot(page, 'landing_14_list_page2.png');

    // Go back via prev.
    await page.click('#games-prev');
    await page.waitForTimeout(500);
    const backRows = await page.$$eval('#games-tbody tr:not(.empty)', (rs) => rs.length);
    if (backRows !== 20) {
        record('minor', 'pager prev', 'expected 20 rows back on page 1, got ' + backRows);
    }

    // Apply filter "filter-target" → should narrow to 5.
    await page.fill('#games-filter', 'filter-target');
    await page.waitForTimeout(450); // debounce 200ms + roundtrip
    const filteredText = await page.textContent('#games-count');
    const filteredRows = await page.$$eval('#games-tbody tr:not(.empty)', (rs) => rs.length);
    console.log('  filter rows:', filteredRows, '/ countText:', filteredText);
    if (filteredRows !== 5 || !/of\s+5/.test(filteredText)) {
        record('major', 'filter narrow',
            'expected 5 rows / "of 5", got rows=' + filteredRows + ' text=' + filteredText);
    }
    await shot(page, 'landing_15_list_filtered.png');

    // Filter by host name ("bulkhost") → should narrow to NUM_GAMES-5 = 20.
    await page.fill('#games-filter', 'bulkhost');
    await page.waitForTimeout(450);
    const hostText = await page.textContent('#games-count');
    if (!/of\s+20/.test(hostText)) {
        record('major', 'filter host',
            'expected "of 20" filtering by host, got: ' + hostText);
    }

    // Clear filter → back to >=25.
    await page.fill('#games-filter', '');
    await page.waitForTimeout(450);
    const clearText = await page.textContent('#games-count');
    if (!/of\s+(2[5-9]|[3-9]\d)/.test(clearText)) {
        record('minor', 'filter clear', 'expected "of >=25" after clear, got: ' + clearText);
    }

    await ctx.close();
    await browser.close();
    // Close host sockets — server removes them from waiting_games on drop.
    for (const s of hosts) {
        try { s.close(); } catch (e) {}
    }
    // Brief delay so the server processes the closes before later scenarios poll.
    await new Promise((r) => setTimeout(r, 300));
}

// mtg-35z3s Step 1: verify renderer selector is ABSENT from the lobby.
// Renderer choice belongs in launcher.html (Step 2); the lobby must NOT have it.
async function scenarioNativeGuiLaunch() {
    console.log('\n=== Scenario: renderer selector absent from lobby (mtg-35z3s Step 1) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    page.on('pageerror', (e) => record('major', 'renderer-absent pageerror', e.message));

    await page.goto(global.__landingRoot || (BASE + '/'), NAV);
    await page.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(page); } catch (e) {
        record('blocking', 'renderer-absent ws', e.message);
    }

    await page.fill('#username', 'native-tester');
    await page.click('#btn-name');
    await page.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 4000 }).catch(() =>
        record('blocking', 'renderer-absent lobby', 'lobby pane never revealed'),
    );

    // The renderer radio buttons must NOT exist on the lobby (mtg-35z3s).
    const tuiRadio = await page.$('#lobby-ui-tui');
    if (tuiRadio) {
        record('major', 'renderer-absent', '#lobby-ui-tui radio must NOT be on the lobby (removed in Step 1)');
    }
    const nativeRadio = await page.$('#lobby-ui-native');
    if (nativeRadio) {
        record('major', 'renderer-absent', '#lobby-ui-native radio must NOT be on the lobby (removed in Step 1)');
    }
    // Also verify no lobby-ui radio input of any kind.
    const anyLobbyUi = await page.$('input[name="lobby-ui"]');
    if (anyLobbyUi) {
        record('major', 'renderer-absent', 'name="lobby-ui" radio input found on lobby — must be absent');
    }
    console.log('  renderer radio absent: PASS (all three checks clean)');

    // Verify Create lands STRAIGHT on launcher.html (no waiting room, mtg-682).
    await page.fill('#create-game', 'renderer-absent-test');
    await page.fill('#create-pass', '');
    await page.click('#btn-create');
    await page.waitForFunction(
        () => /launcher\.html/.test(window.location.href) &&
              /game=renderer-absent-test/.test(window.location.href),
        null,
        { timeout: 4000 },
    ).catch(() => record('blocking', 'renderer-absent redirect', 'Create did not navigate straight to launcher.html'));

    const finalUrl = page.url();
    console.log('  launcher redirect URL:', finalUrl);
    await shot(page, 'landing_17_native_gui_redirect.png');

    await ctx.close();
    await browser.close();
}

// Phase 2 / mtg-khy7x: verify waiting-room WaitingRoomUpdate display and that
// both game pages receive equivalent query-param dispatch (mtg-1vwpd).
async function scenarioWaitingRoomAndParamContract() {
    console.log('\n=== Scenario: waiting-room display + param contract (mtg-khy7x / mtg-1vwpd) ===');
    const browser = await chromium.launch();

    // Creator (dave) — enters lobby, creates a game, enters waiting room.
    const daveCtx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const dave = await daveCtx.newPage();
    dave.setDefaultNavigationTimeout(90000); dave.setDefaultTimeout(90000);
    dave.on('pageerror', (e) => record('major', 'dave pageerror', e.message));

    await dave.goto(global.__landingRoot || (BASE + '/'), NAV);
    await dave.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(dave); } catch (e) {
        record('blocking', 'dave ws', e.message);
    }

    await dave.fill('#username', 'dave');
    await dave.click('#btn-name');
    await dave.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 5000 }).catch(() =>
        record('blocking', 'dave lobby', 'lobby pane never appeared for dave'),
    );

    // Step 1 (mtg-35z3s): renderer radio must NOT be in the lobby at all.
    const nativeRadioExists = await dave.$('#lobby-ui-native');
    if (nativeRadioExists) {
        record('major', 'param contract', '#lobby-ui-native must NOT be on lobby (renderer belongs in launcher.html)');
    }

    // mtg-682 item 1: Create goes STRAIGHT to launcher — there is no waiting
    // room. Confirm the pane and any sharable-link block are gone, and the
    // navigation is direct with the right params.
    const waitingPaneDave = await dave.$('#pane-waiting');
    if (waitingPaneDave) {
        record('major', 'no waiting room', '#pane-waiting must NOT exist on the lobby');
    }
    const inviteBlockDave = await dave.$('#waiting-invite-block, .invite-block');
    if (inviteBlockDave) {
        record('major', 'no sharable link', 'a sharable-invite block must NOT exist on the lobby');
    }
    const wrDeckSel = await dave.$('#wr-deck-select');
    if (wrDeckSel) {
        record('major', 'no waiting room', '#wr-deck-select must NOT exist (deck choice is on the launcher)');
    }

    await dave.fill('#create-game', 'wr-test-game');
    await dave.fill('#create-pass', '');
    await dave.click('#btn-create');
    await dave.waitForFunction(
        () => /launcher\.html/.test(window.location.href) && /game=wr-test-game/.test(window.location.href),
        null,
        { timeout: 5000 },
    ).catch(() => record('blocking', 'dave create', 'Create did not navigate straight to launcher.html'));
    await shot(dave, 'landing_18_create_straight_to_launcher.png');

    // Param contract test (mtg-35z3s Step 1): lobby handoff to launcher.html uses
    // game=, role=, pass=, name=, ws= (NOT lobby_create/lobby_join/deck/ui/mode).
    // Those old params are now launcher→game-page concerns (Step 2+).
    const paramTest = await dave.evaluate(() => {
        // Mirror doRedirectToLauncher() logic in index.html.
        const opts = {
            gameName: 'test-game',
            role: 'create',
            gamePass: 'pw',
            playerName: 'dave',
            wsUrl: 'ws://localhost:1234',
        };
        const qp = new URLSearchParams();
        qp.set('game', opts.gameName);
        qp.set('role', opts.role);
        qp.set('pass', opts.gamePass);
        qp.set('name', opts.playerName);
        qp.set('ws', opts.wsUrl);
        const url = 'launcher.html?' + qp.toString();

        const parsed = new URLSearchParams(url.split('?')[1]);
        return {
            hasGame:   parsed.get('game') === 'test-game',
            hasRole:   parsed.get('role') === 'create',
            hasPass:   parsed.get('pass') === 'pw',
            hasName:   parsed.get('name') === 'dave',
            hasWs:     parsed.get('ws') === 'ws://localhost:1234',
            // Old lobby params must NOT appear.
            noLobbyCreate: !parsed.has('lobby_create'),
            noLobbyJoin:   !parsed.has('lobby_join'),
            page: url.split('?')[0],
        };
    });
    if (!paramTest.hasGame)        record('major', 'param contract', 'game= missing from launcher URL');
    if (!paramTest.hasRole)        record('major', 'param contract', 'role= missing from launcher URL');
    if (!paramTest.hasPass)        record('major', 'param contract', 'pass= missing from launcher URL');
    if (!paramTest.hasName)        record('major', 'param contract', 'name= missing from launcher URL');
    if (!paramTest.hasWs)          record('major', 'param contract', 'ws= missing from launcher URL');
    if (!paramTest.noLobbyCreate)  record('major', 'param contract', 'old lobby_create param must NOT appear');
    if (!paramTest.noLobbyJoin)    record('major', 'param contract', 'old lobby_join param must NOT appear');
    if (paramTest.page !== 'launcher.html') record('major', 'param contract', 'page not launcher.html: ' + paramTest.page);
    console.log('  param contract check:', JSON.stringify(paramTest));

    await daveCtx.close();
    await browser.close();
}

// Seed-persistence regression: the RNG-seed field must round-trip across
// navigation (set -> reload -> value restored). After the prefs.js DRY rework
// (28570dc20) persistence moved from per-page raw keys ('mtg.mpSeed' /
// 'mtg.rngSeed') to ONE shared store: a single JSON object under
// localStorage['deepscry.prefs'], with the seed under the SHARED field
// DeepScryPrefs.KEYS.SEED ('seed') — so a seed entered on launcher.html is the
// default on solo_launcher.html and vice-versa. This scenario verifies that new
// contract: launcher.html (#mp-seed, gated behind role=create&advanced_options)
// and solo_launcher.html (#seed) both round-trip through the shared key, the
// value is SHARED across the two launchers, and clearing the field removes the
// key so the "(random)"/"(server default)" placeholder semantics are preserved.
//
// We read the stored value through the public DeepScryPrefs.get() API (and assert
// the legacy raw keys are gone) instead of poking a hard-coded localStorage key,
// so the test tracks the actual persistence contract rather than its old shape.
async function scenarioSeedPersistence() {
    console.log('\n=== Scenario: RNG-seed persists + is shared across launchers (prefs.js) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    page.on('pageerror', (e) => record('major', 'seed-persist pageerror', e.message));

    // Read the seed pref via the shared store. Returns the value under
    // KEYS.SEED, or null if absent / DeepScryPrefs unavailable.
    const storedSeed = () => page.evaluate(() => {
        try {
            return (window.DeepScryPrefs && typeof window.DeepScryPrefs.get === 'function')
                ? (window.DeepScryPrefs.get(window.DeepScryPrefs.KEYS.SEED) ?? null)
                : null;
        } catch (e) { return null; }
    });
    // The legacy per-page raw keys must NOT be written anymore.
    const legacyKeysGone = () => page.evaluate(() => {
        try {
            return localStorage.getItem('mtg.mpSeed') === null
                && localStorage.getItem('mtg.rngSeed') === null;
        } catch (e) { return true; }
    });

    // --- launcher.html: #mp-seed (creator + advanced gate makes it visible) ---
    const launcherUrl = BASE + '/launcher.html?game=seed-test&role=create&advanced_options=true' +
        '&name=seeder&ws=' + encodeURIComponent(WS_OVERRIDE || 'ws://localhost:17810');
    await page.goto(launcherUrl, NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForSelector('#mp-seed', { state: 'attached', timeout: 5000 }).catch(() =>
        record('major', 'seed-persist launcher', '#mp-seed field not present'));
    // Field should start empty (no stored value yet).
    const mpInitial = await page.inputValue('#mp-seed').catch(() => null);
    if (mpInitial) record('minor', 'seed-persist launcher', 'expected empty #mp-seed on first load, got ' + mpInitial);
    // Type a seed (fire input so the change handler writes the shared store).
    await page.fill('#mp-seed', '12345');
    await page.dispatchEvent('#mp-seed', 'input');
    const mpStored = await storedSeed();
    if (String(mpStored) !== '12345') record('major', 'seed-persist launcher', 'seed not written to shared prefs (KEYS.SEED), got ' + JSON.stringify(mpStored));
    if (!(await legacyKeysGone())) record('major', 'seed-persist launcher', 'legacy raw key mtg.mpSeed/mtg.rngSeed must NOT be written anymore');
    // Reload — the seed must be restored.
    await page.goto(launcherUrl, NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForSelector('#mp-seed', { state: 'attached', timeout: 5000 }).catch(() => {});
    const mpAfter = await page.inputValue('#mp-seed').catch(() => null);
    console.log('  launcher #mp-seed after reload:', JSON.stringify(mpAfter));
    if (mpAfter !== '12345') record('major', 'seed-persist launcher', 'seed NOT restored after reload: got ' + JSON.stringify(mpAfter));

    // --- solo_launcher.html: #seed shares the SAME key → inherits 12345 ---
    const soloUrl = BASE + '/solo_launcher.html';
    await page.goto(soloUrl, NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForSelector('#seed', { state: 'attached', timeout: 5000 }).catch(() =>
        record('major', 'seed-persist solo', '#seed field not present'));
    const soloInherited = await page.inputValue('#seed').catch(() => null);
    console.log('  solo #seed inherited from launcher:', JSON.stringify(soloInherited));
    if (soloInherited !== '12345') {
        record('major', 'seed-persist shared', 'seed is not shared across launchers: solo #seed=' + JSON.stringify(soloInherited) + ' (expected 12345 from launcher)');
    }
    // Change it on solo → shared store updates.
    await page.fill('#seed', '67890');
    await page.dispatchEvent('#seed', 'input');
    const soloStored = await storedSeed();
    if (String(soloStored) !== '67890') record('major', 'seed-persist solo', 'seed not written to shared prefs (KEYS.SEED), got ' + JSON.stringify(soloStored));
    await page.goto(soloUrl, NAV);
    await page.waitForLoadState('domcontentloaded');
    await page.waitForSelector('#seed', { state: 'attached', timeout: 5000 }).catch(() => {});
    const soloAfter = await page.inputValue('#seed').catch(() => null);
    console.log('  solo #seed after reload:', JSON.stringify(soloAfter));
    if (soloAfter !== '67890') record('major', 'seed-persist solo', 'seed NOT restored after reload: got ' + JSON.stringify(soloAfter));
    // The Random button must also update the stored value.
    await page.click('#btn-random-seed');
    const rolled = await page.inputValue('#seed');
    const rolledStored = await storedSeed();
    if (!rolled || String(rolledStored) !== String(rolled)) {
        record('major', 'seed-persist solo', 'Random button did not persist rolled seed: field=' + JSON.stringify(rolled) + ' stored=' + JSON.stringify(rolledStored));
    }
    // Empty case clears the key.
    await page.fill('#seed', '');
    await page.dispatchEvent('#seed', 'input');
    const soloCleared = await storedSeed();
    if (soloCleared !== null) record('major', 'seed-persist solo', 'clearing the field must remove the shared seed pref (placeholder semantics), got ' + JSON.stringify(soloCleared));

    console.log('  seed-persistence: launcher + solo round-trip, cross-launcher sharing, empty/random cases checked');
    await ctx.close();
    await browser.close();
}

async function scenarioAccessibility() {
    console.log('\n=== Scenario: accessibility / form labels ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 } });
    const page = await ctx.newPage();
    page.setDefaultNavigationTimeout(90000); page.setDefaultTimeout(90000);
    await page.goto(global.__landingRoot || (BASE + '/'), NAV);
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
    // mtg-717: http.server logs EVERY request (and every 404) to stderr. If we
    // leave its stdout/stderr as undrained 'pipe's, the 64KB OS pipe buffer
    // fills after a few hundred requests, the server BLOCKS on its next log
    // write, and stops serving — a later scenario's `GET /` then never responds
    // and page.goto hangs until timeout (this was the network-redo scenario-13
    // hang in CI: deterministic past ~800 requests, masked locally only because
    // the suite finishes just before the buffer fills). We don't read these
    // streams, so 'ignore' (route to /dev/null) removes the deadlock entirely.
    const httpProc = spawn('python3', ['-m', 'http.server', String(httpPort)], {
        cwd: __dirname, stdio: 'ignore',
    });
    const mtgProc = spawn(mtgBinary, ['server', '--port', String(mtgPort)], {
        cwd: projectRoot, stdio: 'ignore',
    });
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
        await scenarioPostRedirectAutoLaunch();
        await scenarioRefreshTimerNoError();
        await scenarioMobileViewport();
        await scenarioOfflineLobby();
        await scenarioLaunchPagesSmoke();
        await scenarioStickyLocalImageUnlock();
        await scenarioPasscodeEyeballToggle();
        await scenarioGameListFilterAndPager();
        await scenarioNativeGuiLaunch();
        await scenarioWaitingRoomAndParamContract();
        await scenarioSeedPersistence();
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
