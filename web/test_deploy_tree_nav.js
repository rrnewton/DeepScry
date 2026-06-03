// test_deploy_tree_nav.js — DEPLOY-TREE navigation regression gate for the CAS
// pure-DAG / immutable-manifest pipeline (mtg-4irju, supersedes the mtg-682
// runtime-manifest version).
//
// WHY THIS EXISTS
// ---------------
// The other web e2e tests run against the SOURCE (fixed-name) tree served out
// of web/. The deploy instead runs `mtg hash-web-assets` to produce a
// CONTENT-HASHED staging tree and serves THAT. The hashing/rewrite step is
// exactly where deploy-only 404s have lived. mtg-4irju reworked that step:
//   - the nav graph is now a strict forward DAG (no native⇄tui switch link,
//     lobby_launcher.js is a leaf, back-edges go through index.html?goto=);
//   - the old stable-named asset_manifest.js loader + asset-manifest.json +
//     [data-asset-href] runtime rewrite are DELETED (they were a cache
//     vulnerability: a stale cached loader/manifest served an old hash → 404);
//   - the manifest is content-hashed → asset-manifest.<token>.json (immutable)
//     and the token is baked into index.html (the SOLE mutable file);
//   - index.html threads release=<token> onto forward links and resolves
//     ?goto= back-edges against asset-manifest.<token>.json.
//
// This gate stages the tree via the SAME `mtg hash-web-assets` code path the
// deploy uses, serves it with `mtg server-web`, and asserts the NEW invariants:
//   (1) ONLY index.html is unhashed; the stale stable-named loader/manifest are
//       GONE (404); exactly one asset-manifest.<token>.json is served.
//   (2) index.html references the HASHED launcher + both HASHED game pages and
//       each resolves 200 (statically-rewritten forward DAG edges).
//   (3) The release-threading wiring is present on the SERVED (post-rename)
//       artifacts: index.html honors release= (param-first, baked-latest
//       fallback) and seeds release onto forward links; lobby_launcher.js's
//       STICKY_PARAM_KEYS carries 'release'; deck_editor's back-edge is the
//       stable index.html?goto=launcher dispatcher URL relaying release.
//   (4) The content-hashed manifest maps every back-edge / nav target
//       (launcher, game pages, deck_editor) to a HASHED name that 200s.
//   (5) STALE-MANIFEST no-404: index.html?goto=launcher with a BOGUS old
//       release= still serves index.html 200 (the dispatcher client-falls-back
//       to latest) — a stale token can NEVER cause the hard 404 this fixes —
//       while the bogus release manifest itself correctly 404s.
//
// It is HTTP-level (no headless browser) so it is hermetic + fast and safe to
// wire into `make validate`. It is LOCAL ONLY (127.0.0.1, no TLS, no cloud VM).
// The RUNTIME release-threading on forward links is JS-driven (applyLaunchLinks
// / forwardStickyParams run in the browser), so this HTTP gate asserts the
// wiring's PRESENCE on the served source; the browser e2e (test_redo_lobby_e2e)
// exercises it live.
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
// `href="..."`/`href='...'` attribute OR a JS string literal. We strip any
// ?query / #fragment so the bare served filename remains; leading `./` or `/`
// is normalised away. The page name is a run of [A-Za-z0-9_.-]+ ending in
// `.html` — the `.` is required so a content-addressed name
// (`launcher.<16hex>.html`) is matched as ONE token, not truncated at the hash.
function htmlPageTokens(src) {
    const tokens = new Set();
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

// Fetch `url` and assert it returns the given status (used for the "must 404"
// stale-release-manifest probe).
async function fetchStatus(base, rel, want, label) {
    const r = await httpGet(base + rel);
    check(r.status === want, `${label}: ${rel} → ${want} (got ${r.status})`);
    return r.status;
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
    const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'mtg-deploy-nav-'));
    log(`Staging deploy tree → ${stage}`);
    fs.cpSync(path.join(WEB_SRC, 'pkg'), path.join(stage, 'pkg'), { recursive: true });
    fs.cpSync(path.join(WEB_SRC, 'data'), path.join(stage, 'data'), { recursive: true });
    for (const f of fs.readdirSync(WEB_SRC)) {
        // Copy every *.html + every NON-TEST *.js sibling so imports resolve.
        // Harmless extras are never hashed unless referenced.
        if (f.endsWith('.html') || (f.endsWith('.js') && !f.startsWith('test_') && f !== 'smoke_test_live.js')) {
            fs.copyFileSync(path.join(WEB_SRC, f), path.join(stage, f));
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

        // ── (1) Only index.html unhashed; stale loader/manifest GONE ────────
        const indexHtml = await fetchOk(base, '/index.html', 'entry');
        check(!!indexHtml, 'index.html served at fixed name (sole mutable URL)');
        if (!indexHtml) throw new Error('index.html did not serve');

        // The deleted runtime-manifest layer must NOT be served anymore.
        await fetchStatus(base, '/asset_manifest.js', 404, 'deleted stable loader');
        await fetchStatus(base, '/asset-manifest.json', 404, 'deleted stable manifest');

        // Exactly one immutable content-hashed manifest, named by the baked token.
        const tokenMatch = indexHtml.match(/MTG_RELEASE_TOKEN\s*=\s*'([0-9a-f]{16})'/);
        check(!!tokenMatch, 'index.html bakes a 16-hex MTG_RELEASE_TOKEN (no placeholder left)');
        check(!indexHtml.includes('__MTG_RELEASE_TOKEN__'), 'release-token placeholder fully replaced');
        const releaseToken = tokenMatch ? tokenMatch[1] : null;
        let manifest = {};
        if (releaseToken) {
            const manifestFile = `asset-manifest.${releaseToken}.json`;
            const mBody = await fetchOk(base, '/' + manifestFile, 'immutable manifest');
            check(!!mBody, `${manifestFile} served (immutable, content-hashed)`);
            if (mBody) {
                try { manifest = JSON.parse(mBody); } catch (_) { check(false, 'manifest parses as JSON'); }
            }
            // The manifest's own name embeds the hash of its bytes → immutable.
            const mr = await httpGet(base + '/' + manifestFile);
            check(
                /immutable/.test(mr.headers['cache-control'] || ''),
                `${manifestFile} served with immutable Cache-Control (got: ${mr.headers['cache-control']})`,
            );
        }

        // ── (2) index → HASHED launcher + both HASHED game pages, each 200 ──
        const indexTokens = htmlPageTokens(indexHtml);
        const launcherHashed = indexTokens.find((t) => /^launcher\.[0-9a-f]{16}\.html$/.test(t));
        check(!!launcherHashed, `index references a HASHED launcher.<hash>.html; saw: ${indexTokens.join(', ')}`);
        const launcherHtml = launcherHashed ? await fetchOk(base, '/' + launcherHashed, 'lobby→launcher') : null;
        check(!!launcherHtml, 'hashed launcher page resolves 200');

        const gamePageTokens = indexTokens.filter((t) => /^(native_game|tui_game)\.[0-9a-f]{16}\.html$/.test(t));
        check(gamePageTokens.length >= 2, `index references BOTH hashed game pages directly; saw: ${gamePageTokens.join(', ')}`);
        for (const gp of gamePageTokens) {
            const body = await fetchOk(base, '/' + gp, 'game page (forward DAG edge)');
            // The game page must NOT emit the old tui⇄native switch link (cycle
            // removed) — its only *.html nav target is the stable index.html.
            if (body) {
                const navOut = htmlPageTokens(body).filter((t) => t !== 'index.html');
                check(
                    navOut.length === 0,
                    `${gp} emits no cross-page nav (switch-renderer link removed); saw: ${navOut.join(', ') || '(none)'}`,
                );
            }
        }

        // ── (4) launcher's forward + the manifest's back-edge targets 200 ───
        let deckEditorHashed = null;
        if (launcherHtml) {
            const launcherTokens = htmlPageTokens(launcherHtml).filter((t) => t !== 'index.html');
            deckEditorHashed = launcherTokens.find((t) => /^deck_editor\.[0-9a-f]{16}\.html$/.test(t)) || null;
            check(!!deckEditorHashed, `launcher forward-links a HASHED deck_editor page; saw: ${launcherTokens.join(', ')}`);
            // Every *.html the launcher emits is a forward DAG edge → hashed 200.
            for (const t of launcherTokens) {
                check(/^[a-z_]+\.[0-9a-f]{16}\.html$/.test(t), `launcher forward link ${t} is HASHED (static DAG edge)`);
                await fetchOk(base, '/' + t, 'launcher forward-nav');
            }
        }

        // The content-hashed manifest must map every back-edge / nav target a
        // browser resolves through the index dispatcher, each to a hashed 200.
        for (const logical of ['launcher.html', 'native_game.html', 'tui_game.html', 'deck_editor.html']) {
            const hashed = manifest[logical];
            check(!!hashed, `manifest maps ${logical} → hashed name (got: ${hashed})`);
            if (hashed) await fetchOk(base, '/' + hashed, `manifest target ${logical}`);
        }

        // ── (3) release-threading wiring present on the SERVED artifacts ────
        // index.html: dispatcher honors release= FIRST, falls back to baked
        // latest, and seeds release onto forward links.
        check(/casResolveAndRedirect/.test(indexHtml), 'index.html carries the ?goto dispatcher');
        check(
            /casLoadManifest\(\s*requested\s*\)/.test(indexHtml),
            'dispatcher resolves the requested release= manifest FIRST (deployment-pinned back-edge)',
        );
        check(
            /casLoadManifest\(\s*MTG_RELEASE_TOKEN\s*\)/.test(indexHtml),
            'dispatcher falls back to the baked-latest manifest (silent-latest)',
        );
        check(
            /RELEASE_BAKED\s*\)\s*p\.set\('release'/.test(indexHtml)
                || /RELEASE_BAKED\)\s*p\.set\("release"/.test(indexHtml),
            'index.html seeds release= onto forward links (applyLaunchLinks)',
        );
        // lobby_launcher.<hash>.js: STICKY_PARAM_KEYS carries 'release' so every
        // hop relays it merged-without-clobbering.
        let llName = null;
        if (gamePageTokens.length) {
            const gpBody = await httpGet(base + '/' + gamePageTokens[0]);
            const m = (gpBody.body || '').match(/(lobby_launcher\.[0-9a-f]{16}\.js)/);
            if (m) llName = m[1];
        }
        check(!!llName, 'lobby_launcher.<hash>.js discovered (imported by a game page, hashed leaf)');
        if (llName) {
            const ll = await fetchOk(base, '/' + llName, 'leaf param module');
            if (ll) {
                check(
                    /STICKY_PARAM_KEYS\s*=\s*\[[^\]]*'release'[^\]]*\]/.test(ll),
                    "lobby_launcher STICKY_PARAM_KEYS includes 'release' (threads + preserves other params)",
                );
                // Leaf-ification: the module must NOT build redirect TARGETS to the
                // game pages (no quoted '<page>.html' nav literal) — that was the
                // old cycle edge. (Header-comment prose like `web/tui_game.html`
                // is not a quoted nav literal, so it does not match.)
                check(
                    !/['"](?:\.\/)?(native_game|tui_game)\.html['"]/.test(ll),
                    'lobby_launcher is a LEAF (no quoted game-page nav literal)',
                );
            }
        }
        // deck_editor: back-edge is the stable dispatcher URL, and it relays release.
        if (deckEditorHashed) {
            const de = await fetchOk(base, '/' + deckEditorHashed, 'launcher→deck_editor (forward)');
            if (de) {
                check(
                    /index\.html\?goto=launcher/.test(de),
                    'deck_editor back-edge is the stable index.html?goto=launcher dispatcher URL',
                );
                check(
                    !/['"](?:\.\/)?launcher\.html['"]/.test(de) && !/launcher\.[0-9a-f]{16}\.html/.test(de),
                    'deck_editor does NOT link launcher directly (no cycle; routed via dispatcher)',
                );
                check(
                    /'release'/.test(de) || /"release"/.test(de),
                    'deck_editor relays release on its launcher back-edge',
                );
            }
        }

        // ── (5) STALE-MANIFEST → no 404 (the bug this fixes) ────────────────
        // A back-edge carrying an OLD/GC'd release token must NOT hard-404: the
        // dispatcher entry (index.html) still serves 200 and client-falls-back
        // to the baked-latest manifest. The bogus release manifest itself 404s
        // (which the dispatcher catches) — proving the fallback path is real.
        await fetchOk(base, '/index.html?goto=launcher&release=' + releaseToken + '&name=alice&deck=foo',
            'back-edge dispatcher entry (current release)');
        await fetchOk(base, '/index.html?goto=launcher&release=deadbeefdeadbeef&name=alice&deck=foo',
            'back-edge dispatcher entry (STALE release → must still 200, not 404)');
        await fetchStatus(base, '/asset-manifest.deadbeefdeadbeef.json', 404,
            'stale release manifest correctly 404s (dispatcher catches → falls back to latest)');

        log('  → CAS pure-DAG deploy-tree navigation graph fully resolved');
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
