// deck_storage.js — durable deck collection on Cloudflare R2 (mtg-742).
//
// PLAIN LANGUAGE: this module lets the website save your custom decks to the
// cloud so they follow you between devices, instead of living only in this
// one browser. It packs all your decks into a single compressed file (a
// ".tgz"), uploads it straight to storage (the bytes never pass through our
// game server), and downloads it again on another device. A "Download my
// decks" button hands you that exact file so your data is never locked in.
//
// ── Architecture (see mtg-742) ────────────────────────────────────────────
//
//   • R2 is the STORE OF RECORD. Each user has ONE object:
//     decks/<identity>/collection.tgz  (a gzipped tar of plaintext .dck files).
//   • The Rust web server mints SHORT-TTL, PREFIX-SCOPED *presigned* URLs at
//     GET /api/deck-storage/credentials. The browser uses those URLs to talk
//     to R2 DIRECTLY (PUT/GET/HEAD). Deck bytes never transit our server.
//   • Cross-device clobber is prevented with an If-Match conditional PUT
//     keyed on the object's ETag (R2 returns 412 if someone else wrote first).
//   • IndexedDB is an offline read/edit CACHE, NOT the source of truth.
//
// This module is OAuth-INDEPENDENT: the server currently resolves every
// caller to a fixed `dev` identity. When login lands, the same endpoint
// returns a per-user prefix and nothing here changes.
//
// ── Opaque storage contract ───────────────────────────────────────────────
// We upload with Content-Type: application/gzip and NO Content-Encoding, so
// the stored bytes are byte-clean (no CDN auto-(de)compression). We gzip the
// tar ourselves; R2 stores and returns the exact bytes.
//
// ── Feature flag ──────────────────────────────────────────────────────────
// Everything here is ADDITIVE and OFF by default. The existing localStorage
// deck flow (mtg-forge-custom-decks) is untouched. Enable the cloud path by
// setting `localStorage['mtg-deck-cloud'] = '1'` (or window.MTG_DECK_CLOUD =
// true before this script loads). Production behaviour is unchanged until the
// user (and OAuth) flip it on.

(function (global) {
  'use strict';

  // ── Constants ────────────────────────────────────────────────────────────
  const CUSTOM_DECKS_KEY = 'mtg-forge-custom-decks'; // shared localStorage key
  const CLOUD_FLAG_KEY = 'mtg-deck-cloud';
  const CREDENTIALS_ENDPOINT = '/api/deck-storage/credentials';
  const IDB_NAME = 'mtg-deck-storage';
  const IDB_STORE = 'collection';
  const IDB_KEY = 'current';
  const SAVE_DEBOUNCE_MS = 1100; // R2 allows ~1 write/sec to the same key.
  const TGZ_CONTENT_TYPE = 'application/gzip';

  // ── Feature flag ──────────────────────────────────────────────────────────
  // Cloud deck storage engages when the user is signed in via OAuth. The
  // AUTHORITATIVE login signal is the server's `/auth/status` (the HttpOnly
  // session cookie) — see authStatus() below — NOT the inherited
  // sessionStorage['mtg.cloudIdentity'] hint, which is absent in a fresh
  // browser that logged in directly (the Safari+Google bug: the user signed in
  // but the editor showed nothing because it trusted sessionStorage instead of
  // the cookie).
  //
  // cloudEnabled() is the *synchronous* flag check still used by the legacy
  // background-mirror path (maybeCloudSync). The editor's primary cloud path now
  // uses the async, authoritative authStatus()/isLoggedIn() and does NOT depend
  // on this returning true. The dev flag remains as a manual override.
  function cloudEnabled() {
    if (global.MTG_DECK_CLOUD === true) return true;
    try {
      if (localStorage.getItem(CLOUD_FLAG_KEY) === '1') return true;
    } catch (_) {
      /* ignore */
    }
    try {
      return !!sessionStorage.getItem('mtg.cloudIdentity');
    } catch (_) {
      return false;
    }
  }

  // Cache the authoritative status for the page lifetime so the two lists, the
  // save indicator, and the migrate button all agree without re-fetching.
  let _statusCache = null;

  /**
   * Authoritative login status straight from the server session cookie.
   * Returns the parsed `/auth/status` body:
   *   { logged_in, user_id, provider, display_name, suggested_name,
   *     oauth_enabled, providers:{github,google} }
   * On a network/parse error returns a logged-out shape so callers degrade to
   * the localStorage-only path. `force` re-fetches past the cache.
   */
  async function authStatus(force) {
    if (_statusCache && !force) return _statusCache;
    try {
      const r = await fetch('/auth/status', { cache: 'no-store' });
      if (!r.ok) {
        _statusCache = { logged_in: false, oauth_enabled: false, providers: {} };
        return _statusCache;
      }
      _statusCache = await r.json();
    } catch (_) {
      _statusCache = { logged_in: false, oauth_enabled: false, providers: {} };
    }
    return _statusCache;
  }

  /** True iff the server currently has a valid OAuth session for this browser. */
  async function isLoggedIn() {
    const s = await authStatus();
    return !!(s && s.logged_in);
  }

  /**
   * The R2 object identity for the logged-in account, for transparency UI:
   *   { user_id, object_key }
   *   e.g. "github-583231", "decks/github-583231/collection.tgz"
   * Returns null when not logged in / R2 not configured (the credentials
   * endpoint answers 401/503). Re-uses the SAME presign endpoint the save path
   * uses, so the displayed path is exactly where the bytes live.
   */
  async function credentialsInfo() {
    try {
      const creds = await fetchCredentials();
      return { user_id: creds.user_id || null, object_key: creds.object_key || null };
    } catch (_) {
      return null;
    }
  }

  // ── Tiny USTAR tar writer/reader (no dependency) ──────────────────────────
  //
  // We store each deck as a plaintext `<name>.dck` member. USTAR is dead
  // simple: a 512-byte header per file (name, octal size, checksum, type),
  // then the file body padded to a 512-byte boundary, then two zero blocks.

  const enc = new TextEncoder();
  const dec = new TextDecoder();

  function octal(n, width) {
    // Fixed-width, NUL-terminated octal field (USTAR convention).
    const s = n.toString(8).padStart(width - 1, '0');
    return s + '\0';
  }

  function writeTarHeader(name, size) {
    const h = new Uint8Array(512);
    const put = (str, off, len) => {
      const bytes = enc.encode(str);
      h.set(bytes.subarray(0, len), off);
    };
    put(name, 0, 100); // name
    put('0000644\0', 100, 8); // mode
    put('0000000\0', 108, 8); // uid
    put('0000000\0', 116, 8); // gid
    put(octal(size, 12), 124, 12); // size
    put(octal(0, 12), 136, 12); // mtime (0 = deterministic)
    // checksum field (8 bytes): fill with spaces first, compute below.
    for (let i = 148; i < 156; i++) h[i] = 0x20;
    h[156] = '0'.charCodeAt(0); // typeflag '0' = regular file
    put('ustar\0', 257, 6); // magic
    put('00', 263, 2); // version
    // Checksum = sum of all header bytes with the checksum field as spaces.
    let sum = 0;
    for (let i = 0; i < 512; i++) sum += h[i];
    put(octal(sum, 7).slice(0, 6) + '\0 ', 148, 8);
    return h;
  }

  /**
   * Pack a {name: dckText} map into gzipped-tar (.tgz) bytes.
   * Returns a Promise<Uint8Array>.
   */
  async function packTgz(files) {
    const chunks = [];
    const names = Object.keys(files).sort(); // deterministic ordering
    for (const name of names) {
      const body = enc.encode(files[name]);
      chunks.push(writeTarHeader(name, body.length));
      chunks.push(body);
      const pad = (512 - (body.length % 512)) % 512;
      if (pad) chunks.push(new Uint8Array(pad));
    }
    chunks.push(new Uint8Array(1024)); // two zero blocks = end of archive
    const tar = concatChunks(chunks);
    return gzip(tar);
  }

  /**
   * Unpack .tgz bytes into a {name: dckText} map. Returns Promise<object>.
   */
  async function unpackTgz(tgzBytes) {
    const tar = await gunzip(tgzBytes);
    const files = {};
    let off = 0;
    while (off + 512 <= tar.length) {
      const header = tar.subarray(off, off + 512);
      // An all-zero block marks the end of the archive.
      if (header.every((b) => b === 0)) break;
      const name = readStr(header, 0, 100);
      const size = parseInt(readStr(header, 124, 12).trim() || '0', 8);
      off += 512;
      if (name) {
        files[name] = dec.decode(tar.subarray(off, off + size));
      }
      off += Math.ceil(size / 512) * 512;
    }
    return files;
  }

  function readStr(buf, off, len) {
    let end = off;
    const limit = off + len;
    while (end < limit && buf[end] !== 0) end++;
    return dec.decode(buf.subarray(off, end));
  }

  function concatChunks(chunks) {
    const total = chunks.reduce((n, c) => n + c.length, 0);
    const out = new Uint8Array(total);
    let o = 0;
    for (const c of chunks) {
      out.set(c, o);
      o += c.length;
    }
    return out;
  }

  // ── gzip / gunzip via native streams ─────────────────────────────────────
  async function gzip(bytes) {
    if (typeof CompressionStream === 'undefined') {
      throw new Error('CompressionStream unavailable; cannot gzip deck collection');
    }
    const cs = new CompressionStream('gzip');
    const stream = new Blob([bytes]).stream().pipeThrough(cs);
    return new Uint8Array(await new Response(stream).arrayBuffer());
  }

  async function gunzip(bytes) {
    if (typeof DecompressionStream === 'undefined') {
      throw new Error('DecompressionStream unavailable; cannot gunzip deck collection');
    }
    const ds = new DecompressionStream('gzip');
    const stream = new Blob([bytes]).stream().pipeThrough(ds);
    return new Uint8Array(await new Response(stream).arrayBuffer());
  }

  // ── Deck collection <-> .dck text conversion ─────────────────────────────
  //
  // The localStorage collection shape (shared with deck_editor.html /
  // launcher.html) is:
  //   { [name]: { main_deck: [[card,count],...], sideboard: [[card,count],...] } }
  // We serialize each deck to a plaintext `.dck` member so the .tgz is
  // human-readable and tool-friendly (data-liberation), then parse it back.

  function deckToDck(name, deck) {
    const lines = [`[metadata]`, `Name=${name}`, ``, `[Main]`];
    for (const [card, count] of deck.main_deck || []) {
      lines.push(`${count} ${card}`);
    }
    const side = deck.sideboard || [];
    if (side.length) {
      lines.push(``, `[Sideboard]`);
      for (const [card, count] of side) lines.push(`${count} ${card}`);
    }
    return lines.join('\n') + '\n';
  }

  function dckToDeck(text) {
    const main = [];
    const sideboard = [];
    let section = 'main';
    let name = '';
    for (let raw of text.split(/\r?\n/)) {
      const line = raw.trim();
      if (!line) continue;
      const lower = line.toLowerCase();
      if (lower.startsWith('name=')) {
        name = line.slice(5).trim();
        continue;
      }
      if (line.startsWith('[')) {
        if (lower.includes('sideboard')) section = 'sideboard';
        else if (lower.includes('main')) section = 'main';
        else section = 'meta';
        continue;
      }
      const m = line.match(/^(\d+)\s+(.+)$/);
      if (!m) continue;
      const entry = [m[2].trim(), parseInt(m[1], 10)];
      (section === 'sideboard' ? sideboard : main).push(entry);
    }
    return { name, deck: { main_deck: main, sideboard } };
  }

  /** {name: deckObj} -> {filename: dckText} for tar packing. */
  function collectionToFiles(collection) {
    const files = {};
    for (const [name, deck] of Object.entries(collection)) {
      files[`${sanitizeFilename(name)}.dck`] = deckToDck(name, deck);
    }
    return files;
  }

  /** {filename: dckText} -> {name: deckObj}. */
  function filesToCollection(files) {
    const collection = {};
    for (const text of Object.values(files)) {
      const { name, deck } = dckToDeck(text);
      if (name) collection[name] = deck;
    }
    return collection;
  }

  function sanitizeFilename(name) {
    return name.replace(/[^A-Za-z0-9 _.-]/g, '_');
  }

  // ── IndexedDB cache (offline read/edit, NOT source of truth) ─────────────
  function idbOpen() {
    return new Promise((resolve, reject) => {
      const req = indexedDB.open(IDB_NAME, 1);
      req.onupgradeneeded = () => req.result.createObjectStore(IDB_STORE);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
  }

  async function idbPut(record) {
    const db = await idbOpen();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(IDB_STORE, 'readwrite');
      tx.objectStore(IDB_STORE).put(record, IDB_KEY);
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error);
    });
  }

  async function idbGet() {
    const db = await idbOpen();
    return new Promise((resolve, reject) => {
      const tx = db.transaction(IDB_STORE, 'readonly');
      const req = tx.objectStore(IDB_STORE).get(IDB_KEY);
      req.onsuccess = () => resolve(req.result || null);
      req.onerror = () => reject(req.error);
    });
  }

  // ── Credential minting ───────────────────────────────────────────────────
  async function fetchCredentials() {
    const resp = await fetch(CREDENTIALS_ENDPOINT, { cache: 'no-store' });
    if (resp.status === 503) {
      throw new Error('cloud deck storage is not configured on this server');
    }
    if (!resp.ok) {
      throw new Error(`credentials endpoint returned ${resp.status}`);
    }
    return resp.json();
  }

  // ── Public API ────────────────────────────────────────────────────────────

  /**
   * Hydrate the collection from R2. Returns
   *   { collection: {name: deckObj}, etag: string|null, source: 'remote'|'cache'|'empty' }.
   * Falls back to the IndexedDB cache if the network/R2 is unavailable.
   */
  async function hydrate() {
    let creds;
    try {
      creds = await fetchCredentials();
    } catch (e) {
      const cached = await idbGet();
      if (cached) return { collection: cached.collection, etag: cached.etag, source: 'cache' };
      throw e;
    }
    const resp = await fetch(creds.get_url, { cache: 'no-store' });
    if (resp.status === 404) {
      return { collection: {}, etag: null, source: 'empty' };
    }
    if (!resp.ok) {
      const cached = await idbGet();
      if (cached) return { collection: cached.collection, etag: cached.etag, source: 'cache' };
      throw new Error(`R2 GET failed: ${resp.status}`);
    }
    const etag = resp.headers.get('ETag');
    const bytes = new Uint8Array(await resp.arrayBuffer());
    const files = await unpackTgz(bytes);
    const collection = filesToCollection(files);
    await idbPut({ collection, etag });
    return { collection, etag, source: 'remote' };
  }

  /**
   * Save the collection to R2 with an If-Match conditional write.
   *   - etag === null  → If-None-Match: * (create only; fails if it now exists)
   *   - etag === "..." → If-Match: <etag> (update only if unchanged)
   * Returns { etag } on success. Throws {conflict:true} on 412 so the caller
   * can re-hydrate and merge.
   */
  async function save(collection, etag) {
    const creds = await fetchCredentials();
    const files = collectionToFiles(collection);
    const tgz = await packTgz(files);
    const headers = { 'Content-Type': creds.content_type || TGZ_CONTENT_TYPE };
    if (etag) headers['If-Match'] = etag;
    else headers['If-None-Match'] = '*';
    const resp = await fetch(creds.put_url, { method: 'PUT', headers, body: tgz });
    if (resp.status === 412 || resp.status === 409) {
      const err = new Error('deck collection changed on another device');
      err.conflict = true;
      throw err;
    }
    if (!resp.ok) throw new Error(`R2 PUT failed: ${resp.status}`);
    const newEtag = resp.headers.get('ETag');
    await idbPut({ collection, etag: newEtag });
    return { etag: newEtag };
  }

  // Debounced save: collapses bursts of edits into ≤1 R2 PUT/sec.
  let _saveTimer = null;
  let _pending = null;
  function saveDebounced(collection, etag, onResult) {
    _pending = { collection, etag, onResult };
    if (_saveTimer) clearTimeout(_saveTimer);
    _saveTimer = setTimeout(async () => {
      const p = _pending;
      _pending = null;
      _saveTimer = null;
      try {
        const res = await save(p.collection, p.etag);
        if (p.onResult) p.onResult(null, res);
      } catch (e) {
        if (p.onResult) p.onResult(e, null);
      }
    }, SAVE_DEBOUNCE_MS);
  }

  /**
   * "Download my decks": mint a fresh presigned attachment-GET and navigate
   * to it so the browser saves the real collection.tgz. This is the
   * data-liberation property — the user always gets their actual bytes.
   */
  async function downloadMyDecks() {
    const creds = await fetchCredentials();
    const a = document.createElement('a');
    a.href = creds.download_url;
    a.download = 'deepscry-decks.tgz';
    document.body.appendChild(a);
    a.click();
    a.remove();
  }

  /**
   * Return the best available direct link to the user's cloud collection object
   * (the "Direct link" button, mtg-742 batch2 item 11).
   *
   * Prefers `public_url` — a permanent, non-expiring URL that works when the R2
   * bucket has public access enabled (R2_PUBLIC_BASE_URL set on the server).
   * Falls back to `get_url` (a short-TTL presigned URL) when public access is
   * not configured, then to `download_url` as a last resort.
   *
   * Returns { url, permanent } where `permanent` is true when the public URL is
   * used (no expiry), or false when the presigned URL is used (expires after the
   * server's presign TTL). Returns null when cloud storage is not configured.
   */
  async function directLink() {
    try {
      const creds = await fetchCredentials();
      if (creds.public_url) {
        return { url: creds.public_url, permanent: true };
      }
      const url = creds.get_url || creds.download_url || null;
      return url ? { url, permanent: false } : null;
    } catch (_) {
      return null;
    }
  }

  /**
   * One-time migration: import the existing localStorage decks
   * (mtg-forge-custom-decks) into the R2 collection. ADDITIVE — local decks
   * win on name collision only if the remote slot is empty; existing remote
   * decks are preserved. Does NOT touch/remove the localStorage copy.
   * Gated by the cloud feature flag at the call site.
   */
  async function migrateLocalStorage() {
    let local = {};
    try {
      const raw = localStorage.getItem(CUSTOM_DECKS_KEY);
      local = raw ? JSON.parse(raw) : {};
    } catch (_) {
      local = {};
    }
    const localNames = Object.keys(local);
    if (localNames.length === 0) return { migrated: 0, etag: null };

    const { collection, etag } = await hydrate();
    let migrated = 0;
    for (const name of localNames) {
      if (!(name in collection)) {
        collection[name] = local[name];
        migrated++;
      }
    }
    if (migrated === 0) return { migrated: 0, etag };
    const res = await save(collection, etag);
    return { migrated, etag: res.etag };
  }

  /**
   * Explicit "Migrate All →" (mtg-742, manual migration). Pushes EVERY
   * localStorage deck into the cloud collection, OVERWRITING the cloud slot of
   * the same name (the user clicked the button intending these local decks to
   * become the cloud copy). Distinct from migrateLocalStorage (additive,
   * collision-shy) so the button has predictable "make cloud match local"
   * semantics. Returns { migrated, total, etag }.
   *
   * Does NOT itself delete local copies — the caller decides that based on the
   * "Delete local copies after migrating" checkbox, only after this resolves
   * successfully (so a failed cloud write never loses the local decks).
   */
  async function migrateAll() {
    let local = {};
    try {
      const raw = localStorage.getItem(CUSTOM_DECKS_KEY);
      local = raw ? JSON.parse(raw) : {};
    } catch (_) {
      local = {};
    }
    const localNames = Object.keys(local);
    if (localNames.length === 0) return { migrated: 0, total: 0, etag: null };

    const { collection, etag } = await hydrate();
    for (const name of localNames) collection[name] = local[name];
    const res = await save(collection, etag);
    return { migrated: localNames.length, total: localNames.length, etag: res.etag };
  }

  global.DeckStorage = {
    cloudEnabled,
    authStatus,
    isLoggedIn,
    credentialsInfo,
    hydrate,
    save,
    saveDebounced,
    downloadMyDecks,
    directLink,
    migrateLocalStorage,
    migrateAll,
    // Exposed for tests + reuse:
    packTgz,
    unpackTgz,
    collectionToFiles,
    filesToCollection,
    deckToDck,
    dckToDeck,
    CUSTOM_DECKS_KEY,
    CLOUD_FLAG_KEY,
  };
})(typeof window !== 'undefined' ? window : globalThis);
