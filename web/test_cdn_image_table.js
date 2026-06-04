#!/usr/bin/env node
/**
 * Hermetic test for the task #7 (mtg-722) Scryfall-CDN card-lookup table.
 *
 * Builds a TINY in-memory SCDT table (3 entries: a normal card + a token's
 * composite key + the token's bare-name alias), loads it into the WASM client
 * via tui_load_card_lookup_table(), and asserts that tui_card_cdn_url() resolves
 * cards (and tokens, via the composite key AND the bare-name fallback) to their
 * exact immutable cards.scryfall.io URLs — and returns "" on a genuine miss.
 *
 * No network, no real table: the fixture bytes are constructed here so this
 * runs in CI where web/data/card-lookup.bin (a deploy-time artifact) is absent.
 * This is the client-cascade hermetic gate: it proves table→CDN resolution
 * (the heart of the migration off api.scryfall) end-to-end through the WASM.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const projectRoot = path.join(__dirname, '..');

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

// Build a minimal SCDT (encoding-D) table blob. MUST match the Rust
// encode_card_lookup layout: MAGIC "SCDT" | u8 ver=1 | u8 reserved=0 |
// u32 LE count | u32 LE names_len | names('\n'-joined, SORTED) | N×16B uuid |
// N×u32 LE version (bit31 = DFC flag). `entries` MUST be pre-sorted by key.
function buildFixtureTable(entries) {
    const enc = new TextEncoder();
    const names = entries.map((e) => e.key).join('\n');
    const namesBytes = enc.encode(names);
    const n = entries.length;
    const buf = new Uint8Array(14 + namesBytes.length + n * 16 + n * 4);
    const dv = new DataView(buf.buffer);
    buf.set([0x53, 0x43, 0x44, 0x54], 0); // "SCDT"
    buf[4] = 1; // format_version
    buf[5] = 0; // reserved
    dv.setUint32(6, n, true);
    dv.setUint32(10, namesBytes.length, true);
    buf.set(namesBytes, 14);
    let off = 14 + namesBytes.length;
    for (const e of entries) {
        const hex = e.uuid.replace(/-/g, '');
        for (let i = 0; i < 16; i++) buf[off + i] = parseInt(hex.substr(i * 2, 2), 16);
        off += 16;
    }
    for (const e of entries) {
        dv.setUint32(off, (e.version >>> 0) | (e.dfc ? 0x80000000 : 0), true);
        off += 4;
    }
    return Array.from(buf); // serializable for page.evaluate
}

(async () => {
    let httpServer, browser;
    const { httpPort: HTTP_PORT } = await getRandomPorts();
    const failures = [];
    function check(name, ok, detail) {
        if (ok) log(`PASS: ${name} — ${detail}`);
        else { log(`FAIL: ${name} — ${detail}`); failures.push(`${name}: ${detail}`); }
    }

    try {
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: path.join(projectRoot, 'web'),
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        await new Promise((r) => setTimeout(r, 1500));

        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });
        const browserErrors = [];
        page.on('pageerror', (err) => browserErrors.push(err.message));

        // Boot native_game.html (so WASM inits + window.__mtg is exposed).
        const base = `http://localhost:${HTTP_PORT}`;
        const deck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', {
            deck, p1: 'heuristic', p2: 'heuristic', seed: 42,
        }), { waitUntil: 'networkidle', timeout: 30000 });
        await page.waitForFunction(
            () => window.__mtg && typeof window.__mtg.tui_card_cdn_url === 'function',
            { timeout: 30000 },
        );

        // Fixture: a normal card + the Clue token (composite key + bare alias).
        // Verified live uuids/versions. Keys sorted ascending ('Clue' < 'Clue␟␟␟'
        // < 'Lightning Bolt'). The Lightning Bolt entry sets the DFC flag to
        // exercise the bit31 round-trip through the WASM decode.
        const BOLT = '77c6fa74-5543-42ac-9ead-0e890b188e99';
        const CLUE = 'c321b9e4-ab7e-4e8a-988f-5463c776d685';
        const fixture = buildFixtureTable([
            { key: 'Clue', uuid: CLUE, version: 1771590258, dfc: false },
            { key: 'Clue', uuid: CLUE, version: 1771590258, dfc: false },
            { key: 'Lightning Bolt', uuid: BOLT, version: 1706239968, dfc: true },
        ]);

        const r = await page.evaluate((arr) => {
            const m = window.__mtg;
            const n = m.tui_load_card_lookup_table(new Uint8Array(arr));
            return {
                n,
                bolt: m.tui_card_cdn_url('Lightning Bolt', '', '', '', false, 'normal'),
                clueComposite: m.tui_card_cdn_url('Clue', '', '', '', true, 'small'),
                clueBareFallback: m.tui_card_cdn_url('Clue', '7', '7', 'R', true, 'small'),
                miss: m.tui_card_cdn_url('Nonexistent Card', '', '', '', false, 'small'),
            };
        }, fixture);

        check('table loaded (3 keys)', r.n === 3, `got ${r.n}`);
        check(
            'normal card → CDN URL',
            r.bolt === `https://cards.scryfall.io/normal/front/7/7/${BOLT}.jpg?1706239968`,
            r.bolt,
        );
        check(
            'token composite key → CDN URL',
            r.clueComposite === `https://cards.scryfall.io/small/front/c/3/${CLUE}.jpg?1771590258`,
            r.clueComposite,
        );
        check(
            'token composite MISS → bare-name fallback (still correct art)',
            r.clueBareFallback === `https://cards.scryfall.io/small/front/c/3/${CLUE}.jpg?1771590258`,
            r.clueBareFallback,
        );
        check('genuine miss → "" (cascade falls to gatherer)', r.miss === '', `"${r.miss}"`);
        check('no page errors', browserErrors.length === 0, browserErrors.join('; ') || 'clean');

        if (failures.length === 0) {
            log('=== CDN IMAGE TABLE TEST: PASS ===');
        } else {
            log(`=== CDN IMAGE TABLE TEST: FAIL (${failures.length}) ===`);
        }
    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill('SIGKILL');
    }
    process.exit(failures.length === 0 ? 0 : 1);
})().catch((e) => { console.error(e); process.exit(1); });
