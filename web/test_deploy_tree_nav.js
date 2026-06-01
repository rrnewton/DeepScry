// test_deploy_tree_nav.js — DEPLOY-TREE navigation regression gate (mtg-35z3s
// deploy-only bug: the lobby-redo launcher hub 404'd on the content-hashed
// deploy even though it worked on the dev fixed-name tree).
//
// WHY THIS EXISTS
// ---------------
// The other web e2e tests (test_redo_lobby_e2e.js, test_deck_editor.js) run
// against the SOURCE (fixed-name) tree served straight out of web/. The deploy,
// however, runs `mtg hash-web-assets` to produce a CONTENT-HASHED staging tree
// (the same staging the deploy rsyncs to the VM) and only THEN serves it. The
// hashing/rewrite step is exactly where the lobby-redo broke: `launcher.html`
// became a SECOND forward-linking hub (Play → native_game/tui_game, Deck Editor
// → deck_editor), but the asset-graph rewriter only forward-rewrites links from
// `index.html`. Any page whose nav links pointed at a now-renamed `<page>.html`
// served a dangling fixed name → 404 / bounce-to-lobby on the live deploy. The
// dev-server e2e never caught it because it serves FIXED names.
//
// This test closes that gap: it builds the STAGED tree via the SAME
// `mtg hash-web-assets` code path the deploy uses, serves it with
// `mtg server-web`, and asserts the FULL lobby→launcher→game/editor navigation
// chain RESOLVES (HTTP 200 — not 404, not a redirect bounce) by following the
// real href / redirect-builder targets a browser would, exactly like the deploy
// post-probe chases references from index.html.
//
// It is HTTP-level (no headless browser) so it is hermetic + fast and safe to
// wire into `make validate`. It is LOCAL ONLY (127.0.0.1, no TLS, no cloud VM).
//
// Usage:
//   node web/test_deploy_tree_nav.js
// Requires: a release `mtg --features network` binary + built web assets
// (pkg/ + data/sets/index.json), exactly like test_web_server_smoke.js. The
// Makefile validate-network-e2e-step builds both before invoking this.

const { spawn, spawnSync } = require('child_process');
const path = require('path');
const fs = require('fs');
const os = require('os');
const http = require('http');
const { LOCALHOST, getRandomPorts, isPortAvailable } = require('./test_network_utils');

const PROJECT_ROOT = path.resolve(__dirname, '..');
const WEB_SRC = __dirname;

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

function findMtgBinary() {
    if (process.env.MTG_BIN && fs.existsSync(process.env.MTG_BIN)) {
        return process.env.MTG_BIN;
    }
    const candidates = [
        path.join(PROJECT_ROOT, 'target', 'release', 'mtg'),
        path.join(PROJECT_ROOT, 'target', 'release-deploy', 'mtg'),
        path.join(PROJECT_ROOT, 'target', 'debug', 'mtg'),
    ];
    for (const c of candidates) {
        if (fs.existsSync(c)) return c;
    }
    throw new Error('mtg binary not found. Run: make build-network (or set $MTG_BIN)');
}

function httpGet(url) {
    return new Promise((resolve, reject) => {
        const req = http.get(url, (res) => {
            const chunks = [];
            res.on('data', (c) => chunks.push(c));
            res.on('end', () =>
                resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks).toString('utf8') }),
            );
        });
        req.on('error', reject);
        req.setTimeout(15000, () => req.destroy(new Error('timeout')));
    });
}

async function waitForHttp(base, maxAttempts = 40) {
    for (let i = 0; i < maxAttempts; i++) {
        try {
            const r = await httpGet(base + '/health');
            if (r.status === 200) return true;
        } catch (_) { /* not up yet */ }
        await new Promise((r) => setTimeout(r, 500));
    }
    return false;
}

const failures = [];
function check(cond, msg) {
    if (cond) {
        log(`  ✓ ${msg}`);
    } else {
        log(`  ✗ FAIL: ${msg}`);
        failures.push(msg);
    }
}

// Extract every distinct `*.html` filename token that `src` references via an
// `href="..."`/`href='...'` attribute OR a JS string literal (the lobby's
// `window.location.href = 'launcher.html?...'` redirect builder and
// lobby_launcher.js's `GAME_PAGE` map both use bare string literals). We strip
// any ?query / #fragment so the bare served filename remains. Leading `./` or
// `/` is normalised away. We deliberately do NOT follow `index.html` links
// (that is the entry; following it would loop) — the caller filters as needed.
function htmlPageTokens(src) {
    const tokens = new Set();
    // href attributes (single or double quoted) and bare JS string literals.
    // The page name is a run of [A-Za-z0-9_-]+ ending in `.html`, bounded by a
    // quote on the left (optionally with a ./ or / prefix) and a quote, ?, or #
    // on the right.
    // The page name is a run of [A-Za-z0-9_.-]+ ending in `.html` — the `.` is
    // required so a content-addressed name (`launcher.<16hex>.html`) is matched
    // as a single token, not truncated at the hash dot.
    const re = /['"](?:\.\/|\/)?([A-Za-z0-9_.-]+\.html)(?:['"?#])/g;
    let m;
    while ((m = re.exec(src)) !== null) {
        tokens.add(m[1]);
    }
    return [...tokens];
}

async function startServer(staticDir, port) {
    const mtg = findMtgBinary();
    log(`Launching: ${mtg} server-web --bind ${LOCALHOST}:${port} --static-dir ${staticDir}`);
    const proc = spawn(
        mtg,
        [
            'server-web',
            '--bind', `${LOCALHOST}:${port}`,
            '--static-dir', staticDir,
            '--cardsfolder', path.join(PROJECT_ROOT, 'cardsfolder'),
        ],
        { cwd: PROJECT_ROOT, stdio: ['ignore', 'pipe', 'pipe'] },
    );
    proc.stdout.on('data', (d) => log(`server: ${d.toString().trim()}`));
    proc.stderr.on('data', (d) => log(`server: ${d.toString().trim()}`));
    return proc;
}

function killProc(proc) {
    if (!proc || proc.killed) return;
    try { process.kill(proc.pid, 'SIGTERM'); } catch (_) { /* gone */ }
}

// Fetch `url`, assert 200, and return its body. Records a failure (with the
// observed status) and returns null otherwise. A redirect (3xx) is ALSO a
// failure for navigation links — a launcher→game link that bounces back to the
// lobby is exactly the live-deploy symptom we are guarding against.
async function fetchOk(base, rel, label) {
    const r = await httpGet(base + rel);
    check(r.status === 200, `${label}: ${rel} → 200 (got ${r.status})`);
    return r.status === 200 ? r.body : null;
}

async function main() {
    // --- 0. Preconditions: built assets must exist (same as smoke test) ---
    for (const [f, hint] of [
        [path.join(WEB_SRC, 'data', 'sets', 'index.json'), 'make wasm-export'],
        [path.join(WEB_SRC, 'pkg', 'mtg_engine.js'), 'make wasm-network'],
        [path.join(WEB_SRC, 'pkg', 'mtg_engine_bg.wasm'), 'make wasm-network'],
    ]) {
        if (!fs.existsSync(f)) throw new Error(`missing required asset ${f} (run: ${hint})`);
    }

    const mtg = findMtgBinary();

    // --- 1. Stage a copy of web/ and run the REAL deploy hashing pipeline ---
    // We copy ALL *.html (so launcher.html is present — the second hub that the
    // bug lived in) + the JS leaves + pkg + data, then run `mtg hash-web-assets`
    // exactly like deploy-cloud.sh does.
    const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'mtg-deploy-nav-'));
    log(`Staging deploy tree → ${stage}`);
    fs.cpSync(path.join(WEB_SRC, 'pkg'), path.join(stage, 'pkg'), { recursive: true });
    fs.cpSync(path.join(WEB_SRC, 'data'), path.join(stage, 'data'), { recursive: true });
    const JS_LEAVES = ['server-config.js', 'network.js', 'bug_report.js', 'lobby_launcher.js', 'help_dialog.js'];
    for (const f of fs.readdirSync(WEB_SRC)) {
        if (f.endsWith('.html') || JS_LEAVES.includes(f) || f.endsWith('.js')) {
            // Copy every JS sibling too (game_boot_params.js etc.) so imports
            // resolve; harmless extras are never hashed unless listed.
            if (f.endsWith('.html') || f.endsWith('.js')) {
                fs.copyFileSync(path.join(WEB_SRC, f), path.join(stage, f));
            }
        }
    }
    const hashRes = spawnSync(mtg, ['hash-web-assets', stage], { encoding: 'utf8' });
    log(hashRes.stdout || '');
    if (hashRes.status !== 0) {
        throw new Error(`mtg hash-web-assets failed: ${hashRes.stderr || hashRes.stdout}`);
    }

    // --- 2. Serve the staged tree and walk the navigation graph ---
    let serverPort;
    {
        const ports = await getRandomPorts();
        serverPort = ports.serverPort;
        if (!(await isPortAvailable(serverPort))) serverPort = ports.httpPort;
    }
    const base = `http://${LOCALHOST}:${serverPort}`;
    const server = await startServer(stage, serverPort);

    let exitCode = 1;
    try {
        const up = await waitForHttp(base);
        check(up, 'server-web came up and /health returned 200');
        if (!up) throw new Error('server never came up');

        // Load the runtime manifest (logical → hashed) the server serves. Soft
        // cycle-edge nav links survive as LOGICAL names that a browser resolves
        // through this manifest; we mirror that resolution here. On a tree with
        // no cycles the manifest is empty and `resolve` is the identity.
        let manifest = {};
        {
            const mr = await httpGet(base + '/asset-manifest.json');
            if (mr.status === 200) {
                try { manifest = JSON.parse(mr.body); } catch (_) { /* keep empty */ }
            }
        }
        const resolve = (name) => (Object.prototype.hasOwnProperty.call(manifest, name) ? manifest[name] : name);
        // Fetch a logical nav target the way a browser would: resolve through
        // the manifest first, then GET. Asserts the RESOLVED name is a 200.
        const fetchNav = async (logical, label) => {
            const served = resolve(logical);
            return fetchOk(base, '/' + served, `${label} (${logical}→${served})`);
        };

        // (a) Entry: index.html must serve at its fixed name (the sole stable URL).
        const indexHtml = await fetchOk(base, '/index.html', 'entry');
        check(!!indexHtml, 'index.html served at fixed name');
        if (!indexHtml) throw new Error('index.html did not serve');

        // (b) THE REGRESSION (auto-discovery): index.html's lobby redirect points
        //     at the HASHED launcher page (launcher.<hash>.html). On the OLD
        //     renamer launcher.html was never hashed → index still emitted a bare
        //     'launcher.html' → 404. Assert index references a HASHED launcher
        //     and that it resolves 200.
        const indexTokens = htmlPageTokens(indexHtml);
        const launcherHashed = indexTokens.find((t) => /^launcher\.[0-9a-f]{16}\.html$/.test(t));
        check(
            !!launcherHashed,
            `index.html references a HASHED launcher.<hash>.html (auto-discovery); saw: ${indexTokens.join(', ')}`,
        );
        const launcherHtml = launcherHashed ? await fetchOk(base, '/' + launcherHashed, 'lobby→launcher') : null;
        check(!!launcherHtml, 'hashed launcher page resolves 200 on the deploy tree');

        // (c) The launcher hub's forward nav — Deck Editor → deck_editor — must
        //     resolve to a HASHED 200, NOT be flattened to index.html (the
        //     rejected hack). Walk every *.html token the served launcher emits.
        if (launcherHtml) {
            const launcherTokens = htmlPageTokens(launcherHtml).filter((t) => t !== 'index.html');
            check(
                launcherTokens.some((t) => /^deck_editor\.[0-9a-f]{16}\.html$/.test(t)),
                `launcher forward-links to a HASHED deck_editor page; saw: ${launcherTokens.join(', ')}`,
            );
            for (const t of launcherTokens) {
                await fetchNav(t, 'launcher forward-nav');
            }
        }

        // (d) The game pages (discovered from index.html, hashed) must resolve,
        //     and EVERY nav link they emit (the tui ⇄ native cross-link — the
        //     cycle) must resolve too: either already-hashed in the page, or a
        //     LOGICAL name routed through the runtime manifest. This is the exact
        //     cycle the old entry-redirect hack flattened and the un-hash backoff
        //     avoided; here we prove it resolves on the HASHED tree.
        const gamePageTokens = indexTokens.filter((t) =>
            /^(native_game|tui_game)\.[0-9a-f]{16}\.html$/.test(t),
        );
        check(gamePageTokens.length >= 2, `index references both hashed game pages; saw: ${gamePageTokens.join(', ')}`);
        let llName = null;
        for (const gp of gamePageTokens) {
            const body = await fetchOk(base, '/' + gp, 'game page');
            if (!body) continue;
            // Its cross-page nav (logical or hashed) must resolve.
            for (const t of htmlPageTokens(body).filter((t) => t !== 'index.html')) {
                await fetchNav(t, 'game-page cross-nav');
            }
            const m = body.match(/(lobby_launcher\.[0-9a-f]{16}\.js)/);
            if (m) llName = m[1];
        }

        // (e) lobby_launcher.js (the redirect BUILDER, hashed leaf): its
        //     GAME_PAGE string literals are LOGICAL names resolved at runtime via
        //     resolveAsset(manifest). Assert each one resolves through the
        //     manifest to a 200 — the deploy-correctness proof for the redirect.
        check(!!llName, 'lobby_launcher.<hash>.js discovered via a game page (imported, hashed)');
        if (llName) {
            const ll = await fetchOk(base, '/' + llName, 'redirect-builder module');
            if (ll) {
                const llTokens = htmlPageTokens(ll).filter((t) => t !== 'index.html');
                check(
                    llTokens.includes('tui_game.html') || llTokens.includes('native_game.html'),
                    `lobby_launcher.js keeps LOGICAL game-page names (manifest-resolved); saw: ${llTokens.join(', ')}`,
                );
                for (const t of llTokens) {
                    check(
                        Object.prototype.hasOwnProperty.call(manifest, t),
                        `lobby_launcher's logical '${t}' is in the runtime manifest`,
                    );
                    await fetchNav(t, 'lobby_launcher redirect target');
                }
            }
        }

        log('  → HASHED deploy-tree navigation graph fully resolved');
    } finally {
        killProc(server);
        await new Promise((r) => setTimeout(r, 500));
        if (server && !server.killed) {
            try { process.kill(server.pid, 'SIGKILL'); } catch (_) { /* gone */ }
        }
        try { fs.rmSync(stage, { recursive: true, force: true }); } catch (_) { /* best effort */ }
    }

    log('');
    if (failures.length === 0) {
        log('=== DEPLOY-TREE NAVIGATION GATE: PASS ===');
        exitCode = 0;
    } else {
        log(`=== DEPLOY-TREE NAVIGATION GATE: FAIL (${failures.length}) ===`);
        for (const f of failures) log(`   - ${f}`);
        exitCode = 1;
    }
    process.exit(exitCode);
}

main().catch((e) => {
    console.error('UNCAUGHT', e);
    process.exit(1);
});
