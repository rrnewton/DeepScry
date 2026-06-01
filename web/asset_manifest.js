/**
 * asset_manifest.js — the STABLE-NAMED runtime asset resolver (mtg-682).
 *
 * ──────────────────────────────────────────────────────────────────────────
 * WHY THIS FILE EXISTS (the content-addressing cycle problem)
 * ──────────────────────────────────────────────────────────────────────────
 * The deploy pipeline (`mtg hash-web-assets`, see mtg-engine/src/asset_hash.rs)
 * content-addresses every web asset: `foo.html` → `foo.<blake3>.html`, so the
 * server can serve them `immutable` forever. The ONE exception is the bootstrap
 * entry `index.html`, whose URL must stay stable so browsers can find it.
 *
 * Most references can be rewritten statically because the asset graph is a DAG:
 * a referrer is hashed AFTER its referents, so by the time we hash the referrer
 * its referents' hashed names are already known and we bake them in.
 *
 * But some pages reference each OTHER (a true cycle in the dependency graph):
 *   - `tui_game.html` ⇄ `native_game.html` (mutual renderer-switch nav links)
 *   - the game pages ⇄ `lobby_launcher.js` (pages import it; it names the pages)
 * A cycle cannot be resolved by static topological hashing — computing one
 * member's hash needs another member's (not-yet-computed) hash. We break the
 * cycle the GENERAL way: those cross-references resolve through THIS manifest
 * at RUNTIME instead of being baked in at hash time.
 *
 * ──────────────────────────────────────────────────────────────────────────
 * HOW IT WORKS
 * ──────────────────────────────────────────────────────────────────────────
 * `mtg hash-web-assets` overwrites the `MANIFEST` literal below with the real
 * `logical → hashed` mapping for the staged tree (a structured rewrite, NOT a
 * blind substring edit). This file keeps its STABLE name (never hashed), so any
 * page can import it. On the un-hashed source / dev tree the mapping is empty
 * and `resolveAsset()` is the identity (fixed names resolve as-is) — so dev and
 * deploy behave identically.
 *
 *   import { resolveAsset, installManifestHrefRewrite } from './asset_manifest.js';
 *   const page = resolveAsset('tui_game.html');   // → 'tui_game.<hash>.html' on deploy
 *
 * For plain `<a href="tui_game.html" data-asset-href>` navigation links the page
 * calls `installManifestHrefRewrite()` once on load; it rewrites every
 * `[data-asset-href]` anchor's `href` to the resolved (hashed) name.
 */

'use strict';

/**
 * logical-name → hashed-name map. Overwritten in place by
 * `mtg hash-web-assets` at deploy-staging time. Empty on the source tree
 * (identity resolution), so dev and deploy share one code path.
 *
 * The hasher locates this exact object literal by the marker comment on the
 * next line and replaces the `{ ... }` body. Do NOT remove the marker.
 */
/* @@ASSET_MANIFEST@@ */
export const MANIFEST = {};

/**
 * Resolve a logical asset name (e.g. `'tui_game.html'`) to the name actually
 * served. Returns the hashed name on the deploy tree, or the input unchanged
 * on the source tree / for any asset not in the manifest (identity).
 *
 * Preserves any `?query` / `#fragment` suffix: only the bare filename token is
 * looked up, the suffix is re-appended.
 *
 * @param {string} logical  e.g. 'tui_game.html' or 'tui_game.html?foo=1'
 * @returns {string}
 */
export function resolveAsset(logical) {
    if (typeof logical !== 'string' || logical.length === 0) return logical;
    // Split off ?query / #fragment so only the filename token is resolved.
    const m = logical.match(/^([^?#]*)([?#].*)?$/);
    const name = m[1];
    const suffix = m[2] || '';
    const hashed = Object.prototype.hasOwnProperty.call(MANIFEST, name) ? MANIFEST[name] : name;
    return hashed + suffix;
}

/**
 * Rewrite every `[data-asset-href]` anchor in the document so its `href`
 * points at the resolved (hashed) asset name. The fixed name is authored in
 * BOTH `href` (so the link works on the dev tree even before JS runs) and
 * `data-asset-href` (the logical name to resolve). Idempotent.
 *
 * Call once after DOM is ready.
 */
export function installManifestHrefRewrite() {
    const anchors = document.querySelectorAll('a[data-asset-href]');
    for (const a of anchors) {
        const logical = a.getAttribute('data-asset-href');
        if (logical) a.setAttribute('href', resolveAsset(logical));
    }
}
