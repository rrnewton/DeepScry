// Read-only 2-client smoke test against a deployed DeepScry instance.
//
// Drives the landing-page lobby with two browser contexts (alice + bob),
// then verifies:
//   1. Lobby connects (WS handshake to wss://<host>/lobby or :8080).
//   2. alice can create a passcoded game.
//   3. bob sees alice's game in the refreshed list and joins.
//   4. tui_game.html loads under the redirect with no WASM init error.
//   5. Image-source "Local" option is hidden by default (gated) on
//      both tui_game.html and native_game.html, and appears when
//      ?allow_local_img_load=true is set.
//   6. No cards.bin 404 occurs on game launch; per-set bins load instead.
//
// Usage (no baked-in default host):
//   node web/smoke_test_live.js
//     → BASE is derived from the local, gitignored .deepscry-deploy.env as
//       https://${REMOTE_HOST}[:${REMOTE_PORT}] (REMOTE_PORT optional). Same
//       three search locations as scripts/deploy-cloud.sh. If no config is
//       found (or REMOTE_HOST is unset), the test throws with template-fill
//       instructions instead of guessing a host.
//   DEEPSCRY_BASE_URL=https://<host>      node web/smoke_test_live.js  # CF-proxied (real cert)
//   DEEPSCRY_BASE_URL=https://<host>:8080 node web/smoke_test_live.js  # direct VM (CF origin cert)
//     → an explicit override always wins over the config file.
//
// All browser contexts are created with `ignoreHTTPSErrors: true` so the
// test passes regardless of whether the URL terminates at a public CA
// cert (CF-proxied) or the CF Origin Cert at the VM (which browsers do
// not normally trust because the issuer is a private CF CA). Without
// that flag, direct :8080 hits fail with ERR_CERT_AUTHORITY_INVALID.
//
// Screenshots: web/screenshots/live_smoke/
// Findings JSON: web/screenshots/live_smoke_findings.json
// Exits non-zero if any blocking/major finding.

const { chromium } = require('playwright');
const path = require('path');
const fs = require('fs');
const os = require('os');

// Resolve the deploy target host from the local, gitignored
// .deepscry-deploy.env — mirroring the shell loader in
// scripts/load-deploy-env.sh (same three search locations, same
// template-fill error). No hardcoded host fallback: if neither the
// DEEPSCRY_BASE_URL env override nor a config file supplies a host, we
// throw with instructions to fill in the template.
//
// This script lives in web/, so the repo root is one level up and the
// dev-harness parent dir is two levels up.
const REPO_ROOT = path.resolve(__dirname, '..');
const PARENT_DIR = path.resolve(REPO_ROOT, '..');

function configSearchPaths() {
    return [
        path.join(PARENT_DIR, '.deepscry-deploy.env'),
        path.join(REPO_ROOT, '.deepscry-deploy.env'),
        path.join(os.homedir(), '.config', 'deepscry', 'deploy.env'),
    ];
}

// Minimal KEY=VALUE parser for the .deepscry-deploy.env file. Returns a
// plain object of the keys it understands (REMOTE_HOST, REMOTE_PORT).
// Comments (#...) and blank lines are ignored; surrounding quotes on a
// value are stripped. We do NOT shell-source it (no Node equivalent),
// but the file is a flat KEY=VALUE list so a line parse is faithful.
function parseDeployEnv(file) {
    const out = {};
    const text = fs.readFileSync(file, 'utf8');
    for (const raw of text.split('\n')) {
        const line = raw.trim();
        if (!line || line.startsWith('#')) continue;
        const eq = line.indexOf('=');
        if (eq < 0) continue;
        const key = line.slice(0, eq).trim();
        let val = line.slice(eq + 1).trim();
        if ((val.startsWith('"') && val.endsWith('"')) ||
            (val.startsWith("'") && val.endsWith("'"))) {
            val = val.slice(1, -1);
        }
        out[key] = val;
    }
    return out;
}

function templateFillError() {
    return new Error(
        'no deploy config found. Copy scripts/deepscry-deploy.env.example to\n' +
        '       ' + path.join(PARENT_DIR, '.deepscry-deploy.env') + ' (the dev harness parent dir, preferred)\n' +
        '       — or ' + path.join(REPO_ROOT, '.deepscry-deploy.env') + ', or ~/.config/deepscry/deploy.env —\n' +
        '       and fill in REMOTE_HOST (and REMOTE_USER). Alternatively set DEEPSCRY_BASE_URL=https://<host>.',
    );
}

function resolveBaseUrl() {
    if (process.env.DEEPSCRY_BASE_URL) return process.env.DEEPSCRY_BASE_URL;
    for (const p of configSearchPaths()) {
        if (!fs.existsSync(p)) continue;
        const cfg = parseDeployEnv(p);
        if (!cfg.REMOTE_HOST) {
            throw new Error(
                p + ' is missing REMOTE_HOST.\n' + templateFillError().message,
            );
        }
        // REMOTE_PORT is optional. When set, append it (matches the
        // two-form usage documented in the header comment); when unset,
        // use the bare https://<host> (CF-proxied) form.
        return cfg.REMOTE_PORT
            ? `https://${cfg.REMOTE_HOST}:${cfg.REMOTE_PORT}`
            : `https://${cfg.REMOTE_HOST}`;
    }
    throw templateFillError();
}

const BASE = resolveBaseUrl();
const SHOTS = path.join(__dirname, 'screenshots', 'live_smoke');
fs.mkdirSync(SHOTS, { recursive: true });

const findings = [];
function record(severity, scenario, msg) {
    const line = `[${severity}] ${scenario}: ${msg}`;
    console.log(line);
    findings.push({ severity, scenario, msg });
}
async function shot(page, name) {
    const p = path.join(SHOTS, name);
    try { await page.screenshot({ path: p, fullPage: true }); console.log('  shot:', name); }
    catch (e) { console.log('  shot FAILED', name, e.message); }
}

// Track network failures across a page lifetime.
function attachNetworkWatch(page, label) {
    const events = { failures: [], cardsBin404: false, perSetBins: [], wsConnects: [], wsCloses: [] };
    page.on('response', (resp) => {
        const url = resp.url();
        const status = resp.status();
        if (status >= 400) {
            events.failures.push({ url, status });
            if (/cards\.bin/.test(url)) events.cardsBin404 = true;
        }
        if (/\/data\/sets\/.*\.bin($|\?)/.test(url)) {
            events.perSetBins.push({ url, status });
        }
    });
    page.on('websocket', (ws) => {
        events.wsConnects.push(ws.url());
        ws.on('close', () => events.wsCloses.push(ws.url()));
    });
    page.on('pageerror', (e) => record('major', label + ' pageerror', e.message));
    page.on('console', (m) => {
        if (m.type() === 'error') {
            const t = m.text();
            // Filter out the common "image 404" noise from external CDNs.
            if (/scryfall|gatherer/i.test(t)) return;
            record('minor', label + ' console.error', t);
        }
    });
    return events;
}

async function waitForLobbyConnected(page, timeoutMs = 15000) {
    await page.waitForFunction(
        () => {
            const el = document.getElementById('ws-state');
            return el && el.textContent.trim() === 'Connected';
        },
        null,
        { timeout: timeoutMs },
    );
}

async function scenarioFullFlow() {
    console.log('\n=== Scenario: full lobby flow vs', BASE, '===');
    const browser = await chromium.launch();

    const aliceCtx = await browser.newContext({ viewport: { width: 1280, height: 800 }, ignoreHTTPSErrors: true });
    const alice = await aliceCtx.newPage();
    const aliceNet = attachNetworkWatch(alice, 'alice');

    await alice.goto(BASE + '/');
    await alice.waitForLoadState('domcontentloaded');
    await shot(alice, '01_alice_initial.png');
    try { await waitForLobbyConnected(alice); }
    catch (e) { record('blocking', 'alice ws', 'never reached Connected: ' + e.message); }

    const gameName = 'smoke-' + Date.now();
    const passcode = 'test';

    if (!(await alice.isVisible('#username'))) {
        record('blocking', 'alice username pane', 'username input not visible');
    }
    await alice.fill('#username', 'alice');
    await alice.click('#btn-name');
    await alice.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 6000 })
        .catch(() => record('blocking', 'alice lobby pane', 'lobby pane never revealed'));
    await shot(alice, '02_alice_lobby.png');

    await alice.fill('#create-game', gameName);
    await alice.fill('#create-pass', passcode);
    await alice.click('#btn-create');
    await alice.waitForFunction(
        () => /tui_game\.html|native_game\.html/.test(window.location.href),
        null,
        { timeout: 6000 },
    ).catch(() => record('blocking', 'alice create redirect', 'never redirected to game page'));
    const aliceUrl = alice.url();
    console.log('  alice redirected to:', aliceUrl);
    if (!aliceUrl.includes('lobby_create=' + gameName)) {
        record('major', 'alice create URL', 'missing lobby_create param: ' + aliceUrl);
    }
    if (!aliceUrl.includes('lobby_pass=' + passcode)) {
        record('major', 'alice create URL', 'missing/incorrect lobby_pass param: ' + aliceUrl);
    }
    // Allow WASM to load + auto-CreateGame.
    await alice.waitForTimeout(5000);
    await shot(alice, '03_alice_game_page.png');

    // --- Bob joins ---
    const bobCtx = await browser.newContext({ viewport: { width: 1280, height: 800 }, ignoreHTTPSErrors: true });
    const bob = await bobCtx.newPage();
    const bobNet = attachNetworkWatch(bob, 'bob');
    await bob.goto(BASE + '/');
    await bob.waitForLoadState('domcontentloaded');
    try { await waitForLobbyConnected(bob); }
    catch (e) { record('blocking', 'bob ws', e.message); }

    await bob.fill('#username', 'bob');
    await bob.click('#btn-name');
    await bob.waitForSelector('#pane-lobby:not(.hidden)', { timeout: 6000 });
    await bob.click('#btn-refresh');
    await bob.waitForTimeout(2000);
    await shot(bob, '04_bob_lobby_list.png');

    const gameRows = await bob.$$eval('#games-tbody tr', (rows) =>
        rows.map((r) => r.textContent.trim()),
    );
    console.log('  bob sees ' + gameRows.length + ' rows; looking for ' + gameName);
    const sawGame = gameRows.some((t) => t.includes(gameName));
    if (!sawGame) {
        record('blocking', 'bob lobby visibility',
            'bob did NOT see alice\'s game "' + gameName + '"; rows: ' + JSON.stringify(gameRows.slice(0, 5)));
    } else {
        // Join flow (matches the lobby HTML in web/index.html):
        //   * Each row has an inline <input type="password" id="pw-<gamename>">
        //     for the passcode (if the game requires one).
        //   * The Join button calls joinGame(g) directly via .onclick;
        //     joinGame reads the passcode from the per-row pw input.
        // So we MUST fill the pw input BEFORE clicking Join.
        try {
            // Fill the per-row passcode input first. (CSS.escape lives
            // in the browser, not node — escape inside the evaluate.)
            const filled = await bob.evaluate(
                ({ gn, code }) => {
                    const sel = '#pw-' + CSS.escape(gn);
                    const el = document.querySelector(sel);
                    if (!el) return false;
                    el.value = code;
                    el.dispatchEvent(new Event('input', { bubbles: true }));
                    return true;
                },
                { gn: gameName, code: passcode },
            );
            if (!filled) {
                record('minor', 'bob join pass', `no #pw-${gameName} input found in matching row`);
            }
            // Now click the row's Join button.
            const joined = await bob.evaluate((gn) => {
                const rows = Array.from(document.querySelectorAll('#games-tbody tr'));
                for (const r of rows) {
                    if (r.textContent.includes(gn)) {
                        const btn = r.querySelector('button:not(.pw-toggle)');
                        if (btn) { btn.click(); return true; }
                    }
                }
                return false;
            }, gameName);
            if (!joined) {
                record('minor', 'bob join click', 'no Join button found in matching row');
            } else {
                await bob.waitForFunction(
                    () => /tui_game\.html|native_game\.html/.test(window.location.href),
                    null,
                    { timeout: 8000 },
                ).catch(() => record('major', 'bob join redirect',
                    'bob did not redirect to game page after join click'));
                console.log('  bob redirected to:', bob.url());
                await bob.waitForTimeout(5000);
                await shot(bob, '05_bob_game_page.png');
            }
        } catch (e) {
            record('major', 'bob join flow', 'exception while joining: ' + e.message);
        }
    }

    // --- Network-layer assertions ---
    for (const [who, ev] of [['alice', aliceNet], ['bob', bobNet]]) {
        console.log('  ' + who + ' WS connections:', ev.wsConnects);
        console.log('  ' + who + ' HTTP failures:', ev.failures.length);
        console.log('  ' + who + ' per-set bins loaded:', ev.perSetBins.length);
        if (ev.cardsBin404) {
            record('major', who + ' cards.bin', '404 observed on cards.bin (slim binary regression?)');
        }
        // Filter out CDN image 404s which are expected/non-blocking.
        const realFailures = ev.failures.filter((f) => !/scryfall|gatherer/i.test(f.url));
        if (realFailures.length) {
            for (const f of realFailures.slice(0, 5)) {
                record('major', who + ' http failure', f.status + ' ' + f.url);
            }
        }
        if (ev.wsConnects.length === 0) {
            record('blocking', who + ' ws connect', 'no websocket connections opened at all');
        }
    }

    await aliceCtx.close();
    await bobCtx.close();
    await browser.close();
}

async function scenarioImageGate() {
    console.log('\n=== Scenario: image-source gate (default OFF vs ?allow_local_img_load=true) ===');
    const browser = await chromium.launch();
    const ctx = await browser.newContext({ viewport: { width: 1280, height: 800 }, ignoreHTTPSErrors: true });

    for (const page_name of ['tui_game.html', 'native_game.html']) {
        const page = await ctx.newPage();
        page.on('pageerror', (e) => record('minor', page_name + ' pageerror', e.message));
        const offUrl = BASE + '/' + page_name;
        await page.goto(offUrl);
        await page.waitForLoadState('domcontentloaded');
        await page.waitForTimeout(2000);
        const hasLocalOff = await page.evaluate(() => !!document.getElementById('img-src-local-label'));
        const gateNoteShownOff = await page.evaluate(() => {
            const n = document.getElementById('img-src-local-gate-note');
            if (!n) return false;
            return getComputedStyle(n).display !== 'none';
        });
        console.log('  ' + page_name + ' default: localLabel=' + hasLocalOff + ' gateNoteShown=' + gateNoteShownOff);
        if (hasLocalOff) {
            record('major', 'image gate default',
                page_name + ': "Local" label is still present without ?allow_local_img_load=true');
        }
        await shot(page, 'gate_' + page_name.replace('.html', '') + '_default.png');
        await page.close();

        const page2 = await ctx.newPage();
        const onUrl = BASE + '/' + page_name + '?allow_local_img_load=true';
        await page2.goto(onUrl);
        await page2.waitForLoadState('domcontentloaded');
        await page2.waitForTimeout(2000);
        const hasLocalOn = await page2.evaluate(() => !!document.getElementById('img-src-local-label'));
        console.log('  ' + page_name + ' with allow_local=true: localLabel=' + hasLocalOn);
        if (!hasLocalOn) {
            record('major', 'image gate override',
                page_name + ': "Local" label MISSING even with ?allow_local_img_load=true');
        }
        await shot(page2, 'gate_' + page_name.replace('.html', '') + '_enabled.png');
        await page2.close();
    }

    await ctx.close();
    await browser.close();
}

(async () => {
    console.log('Live smoke test against', BASE);
    console.log('Screenshots →', SHOTS);
    try {
        await scenarioFullFlow();
        await scenarioImageGate();
    } catch (e) {
        console.error('UNCAUGHT', e);
        record('blocking', 'harness', e.message);
    }

    console.log('\n=== FINDINGS ===');
    for (const f of findings) {
        console.log(`[${f.severity}] ${f.scenario}: ${f.msg}`);
    }
    fs.writeFileSync(
        path.join(SHOTS, '..', 'live_smoke_findings.json'),
        JSON.stringify({ base: BASE, when: new Date().toISOString(), findings }, null, 2),
    );
    const fatal = findings.filter((f) => f.severity === 'blocking' || f.severity === 'major').length;
    console.log(`\nTotal findings: ${findings.length} (fatal: ${fatal})`);
    process.exit(fatal > 0 ? 1 : 0);
})();
