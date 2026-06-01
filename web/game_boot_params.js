// game_boot_params.js — shared test helper for booting the PURE game-page
// renderers (native_game.html / tui_game.html) from URL params.
//
// mtg-35z3s page 3: the game pages no longer have a built-in launcher. They
// boot entirely from URL params (see lobby_launcher.consumeLocalGameParams /
// consumeLobbyParams). The e2e tests used to drive the launcher form
// (selectOption('#p1-deck', ...) + click('#btn-launch')); now they construct a
// local-game boot URL instead. This module centralises that URL contract + the
// WASM-free "first built-in deck" lookup so every renderer test shares ONE
// implementation (DRY) rather than each re-deriving the param names.
//
// The deck list comes from data/sets/index.json's `deck_names` field — the same
// WASM-free source launcher.html uses to populate its deck picker. That file is
// emitted by `mtg export-wasm` (part of the wasm build), so it is present
// whenever the game pages themselves are servable.

'use strict';

const http = require('http');

/**
 * Fetch the first built-in deck name from `<base>/data/sets/index.json`.
 * Returns a string deck name, or throws if the index / deck_names is missing.
 *
 * @param {string} base   e.g. "http://localhost:8766"
 * @returns {Promise<string>}
 */
function firstBuiltinDeck(base) {
    return new Promise((resolve, reject) => {
        const url = base.replace(/\/$/, '') + '/data/sets/index.json';
        http.get(url, (res) => {
            if (res.statusCode !== 200) {
                res.resume();
                reject(new Error(`index.json HTTP ${res.statusCode} (run 'mtg export-wasm')`));
                return;
            }
            let body = '';
            res.on('data', (c) => { body += c; });
            res.on('end', () => {
                try {
                    const idx = JSON.parse(body);
                    const names = Array.isArray(idx.deck_names) ? idx.deck_names.slice().sort() : [];
                    if (names.length === 0) {
                        reject(new Error('index.json has no deck_names'));
                        return;
                    }
                    resolve(names[0]);
                } catch (e) {
                    reject(e);
                }
            });
        }).on('error', reject);
    });
}

/**
 * Like firstBuiltinDeck, but prefer the first deck whose name matches `re`,
 * falling back to the first deck overall when none match. Mirrors the old
 * launcher tests' `opts.find(o => /pattern/.test(o)) || opts[0]` selection.
 *
 * @param {string} base
 * @param {RegExp} re
 * @returns {Promise<string>}
 */
async function pickBuiltinDeck(base, re) {
    const url = base.replace(/\/$/, '') + '/data/sets/index.json';
    const names = await new Promise((resolve, reject) => {
        http.get(url, (res) => {
            if (res.statusCode !== 200) {
                res.resume();
                reject(new Error(`index.json HTTP ${res.statusCode} (run 'mtg export-wasm')`));
                return;
            }
            let body = '';
            res.on('data', (c) => { body += c; });
            res.on('end', () => {
                try {
                    const idx = JSON.parse(body);
                    resolve(Array.isArray(idx.deck_names) ? idx.deck_names.slice().sort() : []);
                } catch (e) { reject(e); }
            });
        }).on('error', reject);
    });
    if (names.length === 0) throw new Error('index.json has no deck_names');
    return names.find((n) => re.test(n)) || names[0];
}

/**
 * Build a local-game boot URL for a PURE game-page renderer.
 *
 * @param {string} base   e.g. "http://localhost:8766"
 * @param {string} page   "native_game.html" | "tui_game.html"
 * @param {object} opts
 * @param {string}  opts.deck            - deck for BOTH seats (or use p1Deck/p2Deck)
 * @param {string} [opts.p1Deck]
 * @param {string} [opts.p2Deck]
 * @param {string} [opts.p1='heuristic'] - P1 controller
 * @param {string} [opts.p2='heuristic'] - P2 controller
 * @param {string|number} [opts.seed]
 * @param {boolean} [opts.debug]
 * @param {object} [opts.extra]          - extra raw query params (e.g. allow_local_img_load)
 * @returns {string}
 */
function localGameUrl(base, page, opts) {
    const o = opts || {};
    const qp = new URLSearchParams();
    qp.set('mode', 'local');
    qp.set('p1_deck', o.p1Deck || o.deck);
    qp.set('p2_deck', o.p2Deck || o.deck);
    qp.set('p1', o.p1 || 'heuristic');
    qp.set('p2', o.p2 || 'heuristic');
    if (o.seed !== undefined && o.seed !== null && o.seed !== '') qp.set('seed', String(o.seed));
    if (o.debug) qp.set('debug', 'true');
    if (o.extra) for (const [k, v] of Object.entries(o.extra)) qp.set(k, String(v));
    return base.replace(/\/$/, '') + '/' + page + '?' + qp.toString();
}

/**
 * Parse a .dck file's text into the custom-deck JSON shape the game pages
 * store under the shared `mtg-forge-custom-decks` localStorage key and load via
 * register_custom_deck: { main_deck: [[name,count],...], sideboard: [...] }.
 *
 * mtg-35z3s page 3: the game pages no longer ship a built-in .dck parser
 * (parseDckFormat lived in the deleted launcher), so network e2e tests that
 * give the browser an out-of-glob deck parse it here and seed localStorage.
 *
 * @param {string} content   raw .dck file text
 * @returns {{main_deck: Array<[string,number]>, sideboard: Array<[string,number]>}}
 */
function parseDckIntoCustomDeck(content) {
    const mainDeck = [];
    const sideboard = [];
    let section = null;
    for (const raw of String(content).split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line.startsWith('#')) continue;
        const sec = line.match(/^\[(\w+)\]$/);
        if (sec) { section = sec[1].toLowerCase(); continue; }
        if (section === 'main' || section === 'sideboard') {
            const m = line.match(/^(\d+)\s+(.+?)(?:\|.*)?$/);
            if (m) {
                const count = parseInt(m[1], 10);
                const name = m[2].trim();
                if (count > 0 && name) (section === 'main' ? mainDeck : sideboard).push([name, count]);
            }
        }
    }
    return { main_deck: mainDeck, sideboard };
}

module.exports = { firstBuiltinDeck, pickBuiltinDeck, localGameUrl, parseDckIntoCustomDeck };
