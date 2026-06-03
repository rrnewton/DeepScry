// Hermetic PRE-DEPLOY smoke test for the content-addressed web-asset pipeline
// (mtg-571 + mtg-620). Distinct from web/smoke_test_live.js: this is LOCAL
// ONLY (no deepscry.net / cloud VM), so it is safe to wire into `make
// validate` and CI.
//
// After mtg-620 the invariant is the FULL asset graph rooted at index.html
// is content-addressed and `index.html` is the SOLE unhashed entrypoint.
// This test asserts exactly that.
//
// What it does:
//   1. Stage a hashed copy of web/ via `mtg hash-web-assets` (the Rust
//      replacement for the retired scripts/hash_web_assets.sh, now extended
//      to the full asset graph) into a temp dir.
//   2. Launch `mtg server-web` on a temp localhost port serving that staged,
//      hashed tree.
//   3. Assert, over plain HTTP against 127.0.0.1 (no TLS, no external host):
//        a. GET /                (landing page)        → 200 + short-TTL
//        b. GET /index.html                            → 200 + short-TTL (entry)
//        c. The OLD fixed names of every now-hashed asset (`/server-config.js`,
//           `/network.js`, `/bug_report.js`, `/data/sets/index.json`,
//           `/native_game.html`, `/tui_game.html`, `/demo.html`, the pkg pair)
//           all return 404 on the hashed tree — proof the rewrite renamed them.
//        d. The HASHED names that index.html now references all return 200 +
//           IMMUTABLE. We extract those names from the rewritten index.html
//           and from a sample game page, exactly like a browser would.
//        e. A per-set hashed .bin (logical→hashed via index.json) → 200 +
//           IMMUTABLE.
//        f. Source-tree invariant: against the un-hashed source tree, the
//           FIXED-name pkg (`/pkg/mtg_engine.js`) is served short-TTL (NOT
//           immutable) — the immutability INVARIANT preserved for the
//           `make validate` e2e tests that run pre-hash.
//
// This is the deploy gate's structural guarantee: if the pipeline did not
// hash + rewrite correctly, the hashed-asset fetches 404 and the test fails
// BEFORE any rsync touches the VM.
//
// Usage:
//   node web/test_web_server_smoke.js
// Requires: a release `mtg --features network` binary (build-network) and
// built web assets (wasm-network: pkg/ + data/sets/index.json). The Makefile
// `validate-network-e2e-step` builds both before invoking this.

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

// Locate a release/debug `mtg` binary that has the network feature (so it has
// both `server-web` and `hash-web-assets`). An explicit $MTG_BIN wins (the
// deploy gate passes the exact binary it just built); otherwise prefer the
// plain release build that `make build-network` produces.
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

// Minimal HTTP GET returning { status, headers, body (Buffer) }.
function httpGet(url) {
    return new Promise((resolve, reject) => {
        const req = http.get(url, (res) => {
            const chunks = [];
            res.on('data', (c) => chunks.push(c));
            res.on('end', () =>
                resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) }),
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
        } catch (_) {
            /* not up yet */
        }
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

// Cache-Control header assertions.
function isImmutable(cc) {
    return !!cc && /immutable/.test(cc) && /max-age=31536000/.test(cc);
}
// Cache tiers (mtg-620 + mtg-727):
//   - content-addressed (`<stem>.<16hex>.<ext>`) → immutable, max-age=1y.
//     This includes the hashed data set-index `index.<h>.json` and the
//     release manifest `asset-manifest.<token>.json` — the data index is
//     FOLDED INTO the CAS graph like every other asset (mtg-727), NOT a
//     special-cased mutable/no-cache resolver. On a deploy tree the FIXED
//     `/data/sets/index.json` 404s (renamed to hashed) — see the
//     `fixed404s` list below; only the hashed name resolves.
//   - `index.html` → `public, max-age=60`. On a clean deploy it is the
//     SOLE short-TTL fixed-name asset (recoverable: the CAS dispatcher
//     falls back to latest for a stale token).
function isShortTtl(cc) {
    return !!cc && /max-age=60\b/.test(cc) && !/immutable/.test(cc);
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

// Kill a child process and its group precisely (no broad pkill — there are
// sibling agents/builds running).
function killProc(proc) {
    if (!proc || proc.killed) return;
    try {
        process.kill(proc.pid, 'SIGTERM');
    } catch (_) {
        /* already gone */
    }
}

async function main() {
    // --- 0. Preconditions: built assets must exist ---
    const idxPath = path.join(WEB_SRC, 'data', 'sets', 'index.json');
    const pkgJs = path.join(WEB_SRC, 'pkg', 'mtg_engine.js');
    const pkgWasm = path.join(WEB_SRC, 'pkg', 'mtg_engine_bg.wasm');
    for (const [f, hint] of [
        [idxPath, 'make wasm-export'],
        [pkgJs, 'make wasm-network'],
        [pkgWasm, 'make wasm-network'],
    ]) {
        if (!fs.existsSync(f)) {
            throw new Error(`missing required asset ${f} (run: ${hint})`);
        }
    }

    const mtg = findMtgBinary();

    // --- 1. Stage a HASHED copy of web/ (exactly like the deploy path) ---
    const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'mtg-web-smoke-'));
    log(`Staging hashed web tree → ${stage}`);
    // Copy only what we need (pkg + data + html + JS leaves); skip the huge
    // images/ tree. After mtg-620 every HTML page + JS leaf participates in
    // hashing, so all three JS leaves must be staged.
    fs.cpSync(path.join(WEB_SRC, 'pkg'), path.join(stage, 'pkg'), { recursive: true });
    fs.cpSync(path.join(WEB_SRC, 'data'), path.join(stage, 'data'), { recursive: true });
    const JS_LEAVES = ['server-config.js', 'network.js', 'bug_report.js', 'lobby_launcher.js', 'help_dialog.js'];
    for (const f of fs.readdirSync(WEB_SRC)) {
        if (f.endsWith('.html') || JS_LEAVES.includes(f)) {
            fs.copyFileSync(path.join(WEB_SRC, f), path.join(stage, f));
        }
    }
    // Run the Rust pipeline subcommand to content-address the pkg pair +
    // rewrite the HTML. This is the SAME code path deploy-cloud.sh uses.
    const hashRes = spawnSync(mtg, ['hash-web-assets', stage], { encoding: 'utf8' });
    log(hashRes.stdout || '');
    if (hashRes.status !== 0) {
        throw new Error(`mtg hash-web-assets failed: ${hashRes.stderr || hashRes.stdout}`);
    }

    // index.html stays unhashed; read it and discover every hashed name it
    // now references — exactly the resolution path a browser performs.
    check(fs.existsSync(path.join(stage, 'index.html')), 'index.html remains unhashed (sole entrypoint)');
    const indexHtml = fs.readFileSync(path.join(stage, 'index.html'), 'utf8');

    // Hashed pkg pair (rewritten by web_pkg).
    const jsMatch = indexHtml.match(/(?:\.\/)?pkg\/(mtg_engine\.[0-9a-f]{16}\.js)/);
    const wasmMatch = indexHtml.match(/(?:\.\/)?pkg\/(mtg_engine_bg\.[0-9a-f]{16}\.wasm)/);
    // After the solo-launcher rework the game pages are NO LONGER linked
    // directly from index.html: the solo path is index → solo_launcher → game
    // pages (mtg-704 forward DAG). Discover the hashed game-page names from the
    // hashed solo_launcher page that index references — exactly the resolution
    // path a browser performs when clicking a Solo Launcher link.
    let tuiHashedName = null;
    let nativeHashedName = null;
    const soloMatch = indexHtml.match(/(solo_launcher\.[0-9a-f]{16}\.html)/);
    check(!!soloMatch, 'index.html references hashed solo_launcher.<h>.html');
    if (soloMatch) {
        const soloPath = path.join(stage, soloMatch[1]);
        check(fs.existsSync(soloPath), `hashed ${soloMatch[1]} present on disk`);
        const soloHtml = fs.readFileSync(soloPath, 'utf8');
        const tm = soloHtml.match(/(tui_game\.[0-9a-f]{16}\.html)/);
        if (tm) tuiHashedName = tm[1];
        const nm = soloHtml.match(/(native_game\.[0-9a-f]{16}\.html)/);
        if (nm) nativeHashedName = nm[1];
    }
    let serverCfgHashed = null;
    const cfgMatch = indexHtml.match(/(server-config\.[0-9a-f]{16}\.js)/);
    if (cfgMatch) serverCfgHashed = cfgMatch[1];
    check(!!tuiHashedName, 'solo_launcher references hashed tui_game.<h>.html');
    check(!!nativeHashedName, 'solo_launcher references hashed native_game.<h>.html');
    check(!!serverCfgHashed, 'index.html references hashed server-config.<h>.js');

    // From the hashed tui_game.html, discover the wasm + data-index + JS-leaf hashed names.
    let hashedJs = jsMatch ? jsMatch[1] : null;
    let hashedWasm = wasmMatch ? wasmMatch[1] : null;
    let dataIndexHashed = null;
    let networkJsHashed = null;
    if (tuiHashedName) {
        const tuiPath = path.join(stage, tuiHashedName);
        check(fs.existsSync(tuiPath), `hashed ${tuiHashedName} present on disk`);
        const tuiHtml = fs.readFileSync(tuiPath, 'utf8');
        if (!hashedJs) {
            const m = tuiHtml.match(/(?:\.\/)?pkg\/(mtg_engine\.[0-9a-f]{16}\.js)/);
            if (m) hashedJs = m[1];
        }
        if (!hashedWasm) {
            const m = tuiHtml.match(/(?:\.\/)?pkg\/(mtg_engine_bg\.[0-9a-f]{16}\.wasm)/);
            if (m) hashedWasm = m[1];
        }
        const di = tuiHtml.match(/data\/sets\/(index\.[0-9a-f]{16}\.json)/);
        if (di) dataIndexHashed = di[1];
        const ni = tuiHtml.match(/(network\.[0-9a-f]{16}\.js)/);
        if (ni) networkJsHashed = ni[1];
    }
    let lobbyLauncherJsHashed = null;
    // lobby_launcher.js is imported by both game pages; check native_game.html.
    if (nativeHashedName) {
        const nativePath = path.join(stage, nativeHashedName);
        if (fs.existsSync(nativePath)) {
            const nativeHtml = fs.readFileSync(nativePath, 'utf8');
            const li = nativeHtml.match(/(lobby_launcher\.[0-9a-f]{16}\.js)/);
            if (li) lobbyLauncherJsHashed = li[1];
        }
    }
    check(!!hashedJs, 'pkg JS glue is hashed');
    check(!!hashedWasm, 'pkg wasm is hashed');
    check(!!dataIndexHashed, 'data/sets/index.json is hashed');
    check(!!networkJsHashed, 'network.js is hashed');
    check(!!lobbyLauncherJsHashed, 'lobby_launcher.js is hashed');

    // First hashed bin from the HASHED index.json (logical→hashed resolution for bins).
    // ALSO resolve the content-addressed tokens/decks bin names recorded in the
    // manifest (tokens+decks cache-skew fix): they must be hashed (so the server
    // serves them immutable) and there must be NO fixed-name `data/decks.bin` /
    // `data/tokens.bin` left in the staged tree.
    let firstBin = null;
    let tokensBin = null;
    let decksBin = null;
    if (dataIndexHashed) {
        const idxPath = path.join(stage, 'data', 'sets', dataIndexHashed);
        const idx = JSON.parse(fs.readFileSync(idxPath, 'utf8'));
        check(Array.isArray(idx.sets) && idx.sets.length > 0, 'hashed index.json has sets[]');
        firstBin = idx.sets[0].file;
        check(/^[0-9-]+[A-Z]+\.[0-9a-f]+\.bin$/.test(firstBin), `first bin is content-addressed: ${firstBin}`);

        tokensBin = idx.tokens;
        decksBin = idx.decks;
        check(/^tokens\.[0-9a-f]{16}\.bin$/.test(tokensBin || ''), `index.json tokens is content-addressed: ${tokensBin}`);
        check(/^decks\.[0-9a-f]{16}\.bin$/.test(decksBin || ''), `index.json decks is content-addressed: ${decksBin}`);
        check(fs.existsSync(path.join(stage, 'data', tokensBin)), `hashed tokens bin exists on disk: ${tokensBin}`);
        check(fs.existsSync(path.join(stage, 'data', decksBin)), `hashed decks bin exists on disk: ${decksBin}`);
        check(!fs.existsSync(path.join(stage, 'data', 'tokens.bin')), 'no fixed-name data/tokens.bin left in staged tree');
        check(!fs.existsSync(path.join(stage, 'data', 'decks.bin')), 'no fixed-name data/decks.bin left in staged tree');
    }

    // --- 2. Launch mtg server-web against the staged hashed tree ---
    let serverPort;
    {
        const ports = await getRandomPorts();
        serverPort = ports.serverPort;
        // Make sure it is free right now.
        if (!(await isPortAvailable(serverPort))) serverPort = ports.httpPort;
    }
    const base = `http://${LOCALHOST}:${serverPort}`;
    const server = await startServer(stage, serverPort);

    let exitCode = 1;
    try {
        const up = await waitForHttp(base);
        check(up, 'server-web came up and /health returned 200');
        if (!up) throw new Error('server never came up');

        // a. Landing page (/ → index.html): 200 + short-TTL (the sole stable URL).
        const landing = await httpGet(base + '/');
        check(landing.status === 200, `landing page / → 200 (got ${landing.status})`);
        check(landing.body.length > 100, 'landing page has a body');
        check(
            isShortTtl(landing.headers['cache-control']),
            `/ (index.html) is short-TTL not immutable (got "${landing.headers['cache-control']}")`,
        );
        const indexExplicit = await httpGet(base + '/index.html');
        check(indexExplicit.status === 200, `/index.html → 200`);
        check(
            isShortTtl(indexExplicit.headers['cache-control']),
            `/index.html is short-TTL not immutable (got "${indexExplicit.headers['cache-control']}")`,
        );

        // b. mtg-620 INVARIANT: every FIXED name of a now-hashed asset
        //    must 404 on the hashed tree (proves the rewrite renamed them).
        const fixed404s = [
            '/pkg/mtg_engine.js',
            '/pkg/mtg_engine_bg.wasm',
            '/server-config.js',
            '/network.js',
            '/bug_report.js',
            '/lobby_launcher.js',
            '/data/sets/index.json',
            '/native_game.html',
            '/tui_game.html',
            '/demo.html',
        ];
        for (const u of fixed404s) {
            const r = await httpGet(base + u);
            check(r.status === 404, `fixed-name ${u} → 404 on hashed tree (got ${r.status})`);
        }

        // c. hashed per-set bin: 200 + immutable.
        if (firstBin) {
            const bin = await httpGet(base + '/data/sets/' + firstBin);
            check(bin.status === 200, `/data/sets/${firstBin} → 200 (got ${bin.status})`);
            check(bin.body.length > 100, `hashed bin has bytes (${bin.body.length})`);
            check(
                isImmutable(bin.headers['cache-control']),
                `hashed bin is IMMUTABLE (got "${bin.headers['cache-control']}")`,
            );
        }

        // d. hashed data/sets/index.json: 200 + IMMUTABLE (NOT no-cache anymore).
        if (dataIndexHashed) {
            const di = await httpGet(base + '/data/sets/' + dataIndexHashed);
            check(di.status === 200, `/data/sets/${dataIndexHashed} → 200 (got ${di.status})`);
            check(
                isImmutable(di.headers['cache-control']),
                `hashed index.json is IMMUTABLE (got "${di.headers['cache-control']}")`,
            );
        }

        // mtg-33fmb: confirm no fixed-name cards.bin is served (it is dead —
        // superseded by the per-set split from mtg-6fsjb). The source HTML
        // files must not contain any `fetch(` calls to `cards.bin`; the smoke
        // test verifies this at the served-asset level by checking 404.
        const cardsBinFixed = await httpGet(base + '/data/cards.bin');
        check(cardsBinFixed.status === 404, `/data/cards.bin → 404 (should be dead / content-addressed)`);
        const cardsRootFixed = await httpGet(base + '/cards.bin');
        check(cardsRootFixed.status === 404, `/cards.bin → 404 (should be dead)`);

        // d2. hashed tokens/decks bins: 200 + IMMUTABLE, and the retired
        //     fixed names must 404 (tokens+decks cache-skew fix). This proves a
        //     content change → new hash → new URL → guaranteed cache-miss, so a
        //     stale browser copy can never be paired with new WASM.
        for (const [label, name] of [['tokens', tokensBin], ['decks', decksBin]]) {
            if (!name) continue;
            const r = await httpGet(base + '/data/' + name);
            check(r.status === 200, `/data/${name} → 200 (got ${r.status})`);
            check(r.body.length > 100, `hashed ${label} bin has bytes (${r.body.length})`);
            check(
                isImmutable(r.headers['cache-control']),
                `hashed ${label} bin is IMMUTABLE (got "${r.headers['cache-control']}")`,
            );
            const fixed = await httpGet(base + `/data/${label}.bin`);
            check(fixed.status === 404, `fixed-name /data/${label}.bin → 404 on hashed tree (got ${fixed.status})`);
        }

        // e. hashed wasm: 200 + immutable.
        if (hashedWasm) {
            const wasm = await httpGet(base + '/pkg/' + hashedWasm);
            check(wasm.status === 200, `/pkg/${hashedWasm} → 200 (got ${wasm.status})`);
            check(wasm.body.length > 1000000, `hashed wasm is reasonably large (${wasm.body.length})`);
            check(
                isImmutable(wasm.headers['cache-control']),
                `hashed wasm is IMMUTABLE (got "${wasm.headers['cache-control']}")`,
            );
        }

        // f. hashed JS glue: 200 + immutable.
        if (hashedJs) {
            const js = await httpGet(base + '/pkg/' + hashedJs);
            check(js.status === 200, `/pkg/${hashedJs} → 200 (got ${js.status})`);
            check(js.body.length > 10000, `hashed JS glue has bytes (${js.body.length})`);
            check(
                isImmutable(js.headers['cache-control']),
                `hashed JS glue is IMMUTABLE (got "${js.headers['cache-control']}")`,
            );
        }

        // g. hashed JS leaves + hashed game HTML: 200 + immutable.
        const moreImmutable = [
            serverCfgHashed && '/' + serverCfgHashed,
            networkJsHashed && '/' + networkJsHashed,
            lobbyLauncherJsHashed && '/' + lobbyLauncherJsHashed,
            tuiHashedName && '/' + tuiHashedName,
            nativeHashedName && '/' + nativeHashedName,
        ].filter(Boolean);
        for (const u of moreImmutable) {
            const r = await httpGet(base + u);
            check(r.status === 200, `${u} → 200 (got ${r.status})`);
            check(
                isImmutable(r.headers['cache-control']),
                `${u} is IMMUTABLE (got "${r.headers['cache-control']}")`,
            );
        }

        log('  → all hashed-tree assertions complete');
    } finally {
        killProc(server);
        // Give it a beat to release the port; then ensure dead.
        await new Promise((r) => setTimeout(r, 500));
        if (server && !server.killed) {
            try { process.kill(server.pid, 'SIGKILL'); } catch (_) { /* gone */ }
        }
    }

    // --- 3. Immutability INVARIANT against the SOURCE tree: a FIXED pkg name
    //        MUST NOT be served immutable. After mtg-620 the fallback for
    //        fixed-name assets is short-TTL (`max-age=60`), not `no-cache`,
    //        so the rule we assert is "NOT immutable" — the invariant
    //        "immutable iff content-addressed" remains intact. ---
    let serverPort2;
    {
        const ports = await getRandomPorts();
        serverPort2 = ports.serverPort;
    }
    const base2 = `http://${LOCALHOST}:${serverPort2}`;
    const server2 = await startServer(WEB_SRC, serverPort2);
    try {
        const up2 = await waitForHttp(base2);
        check(up2, 'second server-web (source tree) came up');
        if (up2) {
            const srcJs = await httpGet(base2 + '/pkg/mtg_engine.js');
            check(srcJs.status === 200, `source /pkg/mtg_engine.js → 200 (got ${srcJs.status})`);
            check(
                !isImmutable(srcJs.headers['cache-control']),
                `fixed-name pkg is NOT immutable (got "${srcJs.headers['cache-control']}")`,
            );
        }
    } finally {
        killProc(server2);
        await new Promise((r) => setTimeout(r, 500));
        if (server2 && !server2.killed) {
            try { process.kill(server2.pid, 'SIGKILL'); } catch (_) { /* gone */ }
        }
    }

    // --- cleanup staging dir ---
    try { fs.rmSync(stage, { recursive: true, force: true }); } catch (_) { /* best effort */ }

    log('');
    if (failures.length === 0) {
        log('=== PRE-DEPLOY LOCAL SMOKE TEST: PASS ===');
        exitCode = 0;
    } else {
        log(`=== PRE-DEPLOY LOCAL SMOKE TEST: FAIL (${failures.length}) ===`);
        for (const f of failures) log(`   - ${f}`);
        exitCode = 1;
    }
    process.exit(exitCode);
}

main().catch((e) => {
    console.error('UNCAUGHT', e);
    process.exit(1);
});
