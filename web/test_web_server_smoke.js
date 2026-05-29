// Hermetic PRE-DEPLOY smoke test for the content-addressed web-asset pipeline
// (mtg-571). Distinct from web/smoke_test_live.js: this is LOCAL ONLY (no
// deepscry.net / cloud VM), so it is safe to wire into `make validate` and CI.
//
// What it does:
//   1. Stage a hashed copy of web/ via `mtg hash-web-assets` (the Rust
//      replacement for the retired scripts/hash_web_assets.sh) into a temp
//      dir, so the pkg pair is content-addressed exactly like a real deploy.
//   2. Launch `mtg server-web` on a temp localhost port serving that staged,
//      hashed tree.
//   3. Assert, over plain HTTP against 127.0.0.1 (no TLS, no external host):
//        a. GET /              (landing page)        → 200
//        b. GET /data/sets/index.json                → 200 + no-cache
//        c. a per-set hashed .bin (logical→hashed via index.json)
//                                                     → 200 + IMMUTABLE
//        d. the hashed wasm (logical→hashed via the rewritten HTML import)
//                                                     → 200 + IMMUTABLE
//        e. the hashed JS glue                        → 200 + IMMUTABLE
//        f. a FIXED-name pkg path (mtg_engine.js)   → 404 (renamed away) and
//           the immutability INVARIANT: a fixed pkg name, were it present,
//           is served no-cache — verified against the SOURCE tree below.
//        g. asset resolution end-to-end: every name the client would fetch
//           (index.json sets[].file, the HTML import specifier) actually
//           resolves to 200 bytes on the server.
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
function isNoCache(cc) {
    return !!cc && /no-cache/.test(cc);
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
    // Copy only what we need (pkg + data + html); skip the huge images/ tree.
    fs.cpSync(path.join(WEB_SRC, 'pkg'), path.join(stage, 'pkg'), { recursive: true });
    fs.cpSync(path.join(WEB_SRC, 'data'), path.join(stage, 'data'), { recursive: true });
    for (const f of fs.readdirSync(WEB_SRC)) {
        if (f.endsWith('.html') || f === 'server-config.js' || f === 'network.js') {
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

    // Derive the hashed pkg names from the rewritten landing/game HTML import
    // specifier — the SAME resolution path a browser performs.
    const tuiHtml = fs.readFileSync(path.join(stage, 'tui_game.html'), 'utf8');
    const jsMatch = tuiHtml.match(/\/pkg\/(mtg_engine\.[0-9a-f]+\.js)/);
    const wasmMatch = tuiHtml.match(/\/pkg\/(mtg_engine_bg\.[0-9a-f]+\.wasm)/);
    check(!!jsMatch, 'HTML rewrite produced a hashed JS import specifier');
    check(!!wasmMatch, 'HTML rewrite produced a hashed wasm module_or_path');
    const hashedJs = jsMatch ? jsMatch[1] : null;
    const hashedWasm = wasmMatch ? wasmMatch[1] : null;

    // First hashed bin from index.json (logical→hashed resolution for bins).
    const idx = JSON.parse(fs.readFileSync(path.join(stage, 'data', 'sets', 'index.json'), 'utf8'));
    check(Array.isArray(idx.sets) && idx.sets.length > 0, 'index.json has sets[]');
    const firstBin = idx.sets[0].file;
    check(/^[0-9-]+[A-Z]+\.[0-9a-f]+\.bin$/.test(firstBin), `first bin is content-addressed: ${firstBin}`);

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

        // a. Landing page.
        const landing = await httpGet(base + '/');
        check(landing.status === 200, `landing page / → 200 (got ${landing.status})`);
        check(landing.body.length > 100, 'landing page has a body');

        // b. index.json: 200 + no-cache (the MUTABLE pointer).
        const indexJson = await httpGet(base + '/data/sets/index.json');
        check(indexJson.status === 200, `/data/sets/index.json → 200 (got ${indexJson.status})`);
        check(
            isNoCache(indexJson.headers['cache-control']),
            `/data/sets/index.json is no-cache (got "${indexJson.headers['cache-control']}")`,
        );

        // c. hashed per-set bin: 200 + immutable.
        const bin = await httpGet(base + '/data/sets/' + firstBin);
        check(bin.status === 200, `/data/sets/${firstBin} → 200 (got ${bin.status})`);
        check(bin.body.length > 100, `hashed bin has bytes (${bin.body.length})`);
        check(
            isImmutable(bin.headers['cache-control']),
            `hashed bin is IMMUTABLE (got "${bin.headers['cache-control']}")`,
        );

        // d. hashed wasm: 200 + immutable.
        if (hashedWasm) {
            const wasm = await httpGet(base + '/pkg/' + hashedWasm);
            check(wasm.status === 200, `/pkg/${hashedWasm} → 200 (got ${wasm.status})`);
            check(wasm.body.length > 1000000, `hashed wasm is reasonably large (${wasm.body.length})`);
            check(
                isImmutable(wasm.headers['cache-control']),
                `hashed wasm is IMMUTABLE (got "${wasm.headers['cache-control']}")`,
            );
        }

        // e. hashed JS glue: 200 + immutable.
        if (hashedJs) {
            const js = await httpGet(base + '/pkg/' + hashedJs);
            check(js.status === 200, `/pkg/${hashedJs} → 200 (got ${js.status})`);
            check(js.body.length > 10000, `hashed JS glue has bytes (${js.body.length})`);
            check(
                isImmutable(js.headers['cache-control']),
                `hashed JS glue is IMMUTABLE (got "${js.headers['cache-control']}")`,
            );
        }

        // f. The FIXED pkg name was renamed away on the staged tree → 404.
        const fixed = await httpGet(base + '/pkg/mtg_engine.js');
        check(fixed.status === 404, `fixed-name /pkg/mtg_engine.js → 404 on hashed tree (got ${fixed.status})`);

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
    //        MUST be served no-cache (NOT immutable). This guards the rule
    //        "immutable iff content-addressed" directly. ---
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
                isNoCache(srcJs.headers['cache-control']),
                `fixed-name pkg is NO-CACHE not immutable (got "${srcJs.headers['cache-control']}")`,
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
