#!/usr/bin/env node
/**
 * E2E test for web/deck_storage.js (R2 durable deck collection, mtg-742).
 *
 * Runs the REAL client module in a real headless browser, but mocks the
 * network so it stays hermetic (no live R2 — CLAUDE.md forbids validate
 * depending on a deployed env). A tiny in-test fake R2 (a JS Map keyed by
 * object key, holding bytes + a synthetic ETag) backs the presigned URLs,
 * so we exercise the genuine pack→PUT→GET→unpack path and the If-Match
 * conditional-write conflict (412) path through the actual module code.
 *
 * Verifies:
 *   1. packTgz/unpackTgz round-trips a deck collection byte-for-name-clean.
 *   2. .dck text serialization round-trips main + sideboard with counts.
 *   3. hydrate() on an empty store returns {} (404 → empty).
 *   4. save() then hydrate() returns the same collection (full R2 round-trip).
 *   5. The upload is Content-Type application/gzip with NO Content-Encoding,
 *      and really is gzip (magic bytes 0x1f 0x8b) — the opaque-bytes contract.
 *   6. A stale-ETag save() throws {conflict:true} (the 412 cross-device path).
 *   7. migrateLocalStorage() imports localStorage decks additively without
 *      clobbering existing remote decks.
 *
 * Usage: node web/test_deck_storage.js
 */

'use strict';

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');

const WEB_SRC = __dirname;

function log(msg) {
  const ts = new Date().toISOString().substring(11, 23);
  console.log(`[${ts}] ${msg}`);
}

const failures = [];
function check(cond, msg) {
  if (cond) log('  ✓ ' + msg);
  else {
    log('  ✗ FAIL: ' + msg);
    failures.push(msg);
  }
}

(async () => {
  let browser = null;
  let httpServer = null;
  const HTTP_PORT = 19000 + Math.floor(Math.random() * 1000);
  try {
    // IndexedDB requires a real (non about:blank) origin; serve web/ over HTTP.
    httpServer = spawn('python3', ['-m', 'http.server', String(HTTP_PORT)], {
      cwd: WEB_SRC,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    httpServer.stderr.on('data', () => {});
    await new Promise((r) => setTimeout(r, 1200));
    log('HTTP server started on ' + HTTP_PORT);

    browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
    const ctx = await browser.newContext();
    const page = await ctx.newPage();
    page.on('pageerror', (e) => {
      log('PAGE ERROR: ' + e.message);
      failures.push('pageerror: ' + e.message);
    });

    // ── Fake R2 + credentials endpoint via request interception ──────────
    // The presigned URLs we hand the client point at a sentinel host; we
    // intercept by URL substring and serve from an in-memory store.
    await page.route('**/api/deck-storage/credentials', (route) => {
      const base = 'https://fake-r2.example.com/deepscry-decks/decks/dev/collection.tgz';
      route.fulfill({
        status: 200,
        contentType: 'application/json',
        body: JSON.stringify({
          user_id: 'dev',
          object_key: 'decks/dev/collection.tgz',
          ttl_secs: 600,
          put_url: base + '?method=PUT',
          get_url: base + '?method=GET',
          head_url: base + '?method=HEAD',
          download_url: base + '?method=GET&dl=1',
          content_type: 'application/gzip',
        }),
      });
    });

    // In-memory fake R2 object store lives in page context (so route handler
    // for the R2 host can read/write it). We implement the R2 host handler in
    // Node and keep the store here.
    const store = { bytes: null, etag: null, ctype: null, cenc: null };
    let etagCounter = 0;
    // Real R2 CORS exposes ETag to cross-origin JS (see scripts/r2-cors.json);
    // the fake MUST mirror that or browser fetch hides response headers — which
    // is itself a useful assertion that our CORS expose-list is necessary.
    const cors = {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Expose-Headers': 'ETag',
    };

    await page.route('**/fake-r2.example.com/**', async (route) => {
      const req = route.request();
      const method = req.method();
      const headers = req.headers();
      if (method === 'PUT') {
        // Conditional-write checks.
        const ifMatch = headers['if-match'];
        const ifNoneMatch = headers['if-none-match'];
        if (ifNoneMatch === '*' && store.bytes !== null) {
          return route.fulfill({ status: 412, headers: cors, body: 'precondition failed' });
        }
        if (ifMatch && ifMatch !== store.etag) {
          return route.fulfill({ status: 412, headers: cors, body: 'precondition failed' });
        }
        const postData = req.postDataBuffer();
        store.bytes = Buffer.from(postData);
        store.ctype = headers['content-type'] || null;
        store.cenc = headers['content-encoding'] || null;
        store.etag = '"etag-' + ++etagCounter + '"';
        return route.fulfill({ status: 200, headers: { ...cors, ETag: store.etag }, body: '' });
      }
      if (method === 'GET' || method === 'HEAD') {
        if (store.bytes === null) return route.fulfill({ status: 404, headers: cors, body: 'not found' });
        return route.fulfill({
          status: 200,
          headers: { ...cors, ETag: store.etag, 'Content-Type': 'application/gzip' },
          body: method === 'HEAD' ? Buffer.alloc(0) : store.bytes,
        });
      }
      return route.fulfill({ status: 405, headers: cors, body: 'method not allowed' });
    });

    // Load the module into a real same-origin page (IndexedDB needs an
    // http(s) origin; site.webmanifest is a tiny always-present static file).
    await page.goto(`http://127.0.0.1:${HTTP_PORT}/site.webmanifest`);
    const moduleSrc = require('fs').readFileSync(path.join(WEB_SRC, 'deck_storage.js'), 'utf8');
    await page.addScriptTag({ content: moduleSrc });
    const hasApi = await page.evaluate(() => typeof window.DeckStorage === 'object');
    check(hasApi, 'DeckStorage module loaded');

    // ── 1+2. pack/unpack + dck round-trip (pure functions) ───────────────
    const sampleCollection = {
      'My Burn Deck': {
        main_deck: [['Lightning Bolt', 4], ['Mountain', 20]],
        sideboard: [['Pyroblast', 2]],
      },
      'Mono Blue': { main_deck: [['Island', 24], ['Counterspell', 4]], sideboard: [] },
    };
    const rt = await page.evaluate(async (coll) => {
      const files = window.DeckStorage.collectionToFiles(coll);
      const tgz = await window.DeckStorage.packTgz(files);
      const back = await window.DeckStorage.unpackTgz(tgz);
      const coll2 = window.DeckStorage.filesToCollection(back);
      return {
        gzipMagic: tgz[0] === 0x1f && tgz[1] === 0x8b,
        fileNames: Object.keys(files),
        roundTrip: coll2,
      };
    }, sampleCollection);
    check(rt.gzipMagic, 'packTgz output has gzip magic bytes (0x1f 0x8b)');
    check(rt.fileNames.includes('My Burn Deck.dck'), 'deck packed as <name>.dck plaintext member');
    check(
      JSON.stringify(rt.roundTrip['My Burn Deck']) === JSON.stringify(sampleCollection['My Burn Deck']),
      'pack→unpack round-trips main+sideboard with counts'
    );
    check(
      JSON.stringify(rt.roundTrip['Mono Blue']) === JSON.stringify(sampleCollection['Mono Blue']),
      'pack→unpack round-trips a sideboard-less deck'
    );

    // ── 3. hydrate empty ─────────────────────────────────────────────────
    const empty = await page.evaluate(async () => {
      const r = await window.DeckStorage.hydrate();
      return { source: r.source, count: Object.keys(r.collection).length };
    });
    check(empty.source === 'empty' && empty.count === 0, 'hydrate() on empty store returns {} (404→empty)');

    // ── 4+5. save then hydrate (full R2 round-trip) + opaque contract ─────
    const saved = await page.evaluate(async (coll) => {
      const res = await window.DeckStorage.save(coll, null);
      const h = await window.DeckStorage.hydrate();
      return { etag: res.etag, hydrated: h.collection, source: h.source };
    }, sampleCollection);
    check(!!saved.etag, 'save() returns an ETag');
    check(saved.source === 'remote', 'hydrate() after save reads from remote');
    check(
      JSON.stringify(saved.hydrated['My Burn Deck']) === JSON.stringify(sampleCollection['My Burn Deck']),
      'save()→hydrate() preserves the collection'
    );
    check(store.ctype === 'application/gzip', 'upload Content-Type is application/gzip');
    check(!store.cenc, 'upload has NO Content-Encoding (opaque byte-clean storage)');
    check(store.bytes[0] === 0x1f && store.bytes[1] === 0x8b, 'stored bytes really are gzip');

    // ── 6. If-Match conflict (stale ETag) ────────────────────────────────
    const conflict = await page.evaluate(async (coll) => {
      try {
        await window.DeckStorage.save(coll, '"etag-stale"');
        return { threw: false };
      } catch (e) {
        return { threw: true, conflict: !!e.conflict };
      }
    }, sampleCollection);
    check(conflict.threw && conflict.conflict, 'stale If-Match save() throws {conflict:true} (412 path)');

    // ── 7. migrate localStorage additively ───────────────────────────────
    const migrated = await page.evaluate(async () => {
      // Seed localStorage with one NEW deck and one that already exists remotely.
      const local = {
        'Local Only Deck': { main_deck: [['Forest', 17], ['Llanowar Elves', 4]], sideboard: [] },
        'My Burn Deck': { main_deck: [['DIFFERENT', 1]], sideboard: [] }, // should NOT clobber
      };
      localStorage.setItem(window.DeckStorage.CUSTOM_DECKS_KEY, JSON.stringify(local));
      const res = await window.DeckStorage.migrateLocalStorage();
      const h = await window.DeckStorage.hydrate();
      return {
        migrated: res.migrated,
        hasLocalOnly: 'Local Only Deck' in h.collection,
        burnUnchanged:
          JSON.stringify(h.collection['My Burn Deck'].main_deck) ===
          JSON.stringify([['Lightning Bolt', 4], ['Mountain', 20]]),
      };
    });
    check(migrated.migrated === 1, 'migrate imports exactly the 1 new local deck');
    check(migrated.hasLocalOnly, 'migrated deck appears in the remote collection');
    check(migrated.burnUnchanged, 'migration is ADDITIVE: existing remote deck not clobbered');

    await browser.close();
    browser = null;
    if (httpServer) httpServer.kill();

    if (failures.length) {
      log(`FAILED: ${failures.length} check(s) failed`);
      process.exit(1);
    }
    log('ALL DECK STORAGE CHECKS PASSED');
    process.exit(0);
  } catch (err) {
    console.error('TEST ERROR: ' + (err && err.stack ? err.stack : err));
    if (browser) await browser.close().catch(() => {});
    if (httpServer) httpServer.kill();
    process.exit(1);
  }
})();
