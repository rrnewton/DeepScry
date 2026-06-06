/**
 * lobby_launcher.js — Shared lobby-redirect + param-plumbing module.
 *
 * Used by:
 *   - web/index.html   (the lobby: builds redirect URLs, offers TUI vs native choice)
 *   - web/tui_game.html    (consumer: reads query params and auto-launches)
 *   - web/native_game.html (consumer: reads query params and auto-launches)
 *
 * Design goals (native-network lobby overhaul, mtg-682):
 *   - DRY: param names, lobby-action semantics, and WS-URL derivation live here
 *     exactly once. Game pages import and call `consumeLobbyParams()` instead of
 *     duplicating the ?lobby_create / ?lobby_join detection logic.
 *   - Selectable launch target: the lobby's "Launch Game" button can send the
 *     user to either `tui_game.html` or `native_game.html`. The target is chosen
 *     via a `ui` query param and is encoded into every redirect URL this module
 *     produces so both pages share the same redirect path.
 *   - No game-state changes: this module is pure browser/JS plumbing. It reads
 *     and writes URL parameters and DOM fields. It does NOT touch WASM game state,
 *     controller decisions, or the network protocol — determinism is preserved.
 *
 * ──────────────────────────────────────────────────────────────────────────────
 * Redirect param contract (used by both `tui_game.html` and `native_game.html`):
 *   ?lobby_create=<game_name>     → game page sends CreateGame on connect
 *   ?lobby_join=<game_name>       → game page sends JoinGame on connect
 *   &lobby_pass=<passcode>        → optional per-game passcode
 *   &deck=<deck_name>             → pre-select deck in the game page launcher
 *   &name=<player_name>           → pre-fill player name field
 *   &ws=<ws_url>                  → override lobby WebSocket URL
 *   &advanced_options=true        → propagate the sticky advanced-options unlock (mtg-781)
 *   &allow_local_img_load=true    → legacy alias of advanced_options (local-image unlock)
 *   &ui=tui|native                → which game-page UI to land on (default: tui)
 *   &mode=local|network           → game mode hint (default: network when from lobby)
 *   &reconnect_token=<token>      → reconnect token from GameStarted (reattach still a stub, mtg-682)
 *   &images=true|false            → show card images on the game page (pre-game pref)
 *   &img_src=local,scryfall,gatherer → enabled image sources, in fallback order
 *   &debug=true                   → enable TRACE logging on the game page
 *   &auto_run=true                → auto-advance an AI game (default: start PAUSED, mtg-780)
 * ──────────────────────────────────────────────────────────────────────────────
 *
 * GAME-PREFS contract (mtg-695 launcher-parity restore):
 *   The lobby redo moved the launcher's per-game *preferences* (image display,
 *   image-source selection, debug logging) off the game pages. They now live on
 *   launcher.html and ride to the game page as the three params above. They are
 *   parsed once by `consumeGamePrefs()` so neither game page re-derives them.
 */

'use strict';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Default launch target when `?ui=` is absent. */
export const DEFAULT_UI = 'tui';

// ---------------------------------------------------------------------------
// buildRedirectQuery — used by launcher.html to construct the redirect params
// ---------------------------------------------------------------------------

/**
 * Build the QUERY PARAMS the lobby/launcher forwards to a game page on
 * "Launch Game" (creator) or "Join" (joiner). Returns ONLY the param set — the
 * CALLER owns the game-page filename (`tui_game.html` / `native_game.html`).
 *
 * This is the mtg-704 LEAF-IFICATION: this module no longer names the game
 * pages, so it has NO back-reference to them. That turns the old
 * `game pages ⇄ lobby_launcher.js` cycle into a one-way import (pages →
 * this leaf), so the deploy hasher statically bakes the leaf's hashed name into
 * each page and never needs a runtime manifest to resolve it. "Which page" now
 * lives UP in launcher.html (a forward DAG edge the hasher rewrites).
 *
 * @param {object} opts
 * @param {'create'|'join'} opts.action
 * @param {string}  opts.gameName
 * @param {string}  [opts.gamePass]         - optional per-game passcode
 * @param {string}  [opts.deckName]         - optional deck override
 * @param {string}  [opts.playerName]       - player's username
 * @param {string}  [opts.wsUrl]            - WebSocket URL override
 * @param {boolean} [opts.allowLocalImgLoad]
 * @param {'tui'|'native'} [opts.ui]        - target game UI (default: 'tui')
 * @param {'local'|'network'} [opts.mode]   - game mode hint (default: 'network')
 * @param {'human'|'random'|'heuristic'|'zero'} [opts.controller] - who drives
 *          THIS web client (default: 'human'). Only 'human'/'random' truly work
 *          in network games; heuristic/zero silently downgrade to Human (mtg-254).
 * @param {string}  [opts.reconnectToken]   - reconnect token from GameStarted
 * @param {boolean} [opts.showImages]       - card-image display pref (game-page)
 * @param {string[]} [opts.imageSources]    - enabled image sources, fallback order
 * @param {boolean} [opts.debug]            - enable TRACE logging on the game page
 * @returns {URLSearchParams}  the redirect query (no page name, no leading '?')
 */
export function buildRedirectQuery(opts) {
    const ui = opts.ui === 'native' ? 'native' : 'tui';
    const qp = new URLSearchParams();

    if (opts.action === 'create') {
        qp.set('lobby_create', opts.gameName);
    } else {
        qp.set('lobby_join', opts.gameName);
    }

    if (opts.gamePass)         qp.set('lobby_pass', opts.gamePass);
    if (opts.deckName)         qp.set('deck', opts.deckName);
    if (opts.playerName)       qp.set('name', opts.playerName);
    if (opts.wsUrl)            qp.set('ws', opts.wsUrl);
    // mtg-781: advanced_options is the new gate (unlocks the Local image
    // source + the advanced multiplayer-seed field). Emit allow_local_img_load
    // too as a backward-compatible alias so older consumers keep working.
    if (opts.advancedOptions || opts.allowLocalImgLoad) {
        qp.set('advanced_options', 'true');
        qp.set('allow_local_img_load', 'true');
    }
    if (opts.reconnectToken)   qp.set('reconnect_token', opts.reconnectToken);
    // Per-game prefs re-homed from the old built-in launcher (mtg-695). Only
    // emit when explicitly provided so a caller that does not set them lets the
    // game page fall back to its built-in default (images on, info-level logs).
    if (opts.showImages !== undefined) qp.set('images', opts.showImages ? 'true' : 'false');
    if (Array.isArray(opts.imageSources) && opts.imageSources.length > 0) {
        qp.set('img_src', opts.imageSources.join(','));
    }
    if (opts.debug) qp.set('debug', 'true');
    // Network controller for the web client (mtg-254). consumeLobbyParams() reads
    // this back; default to 'human' for any unknown/absent value.
    qp.set('controller', ['human', 'random', 'heuristic', 'zero'].includes(opts.controller)
        ? opts.controller : 'human');
    qp.set('ui', ui);
    // Default to 'network' mode when coming from the lobby redirect.
    qp.set('mode', opts.mode === 'local' ? 'local' : 'network');

    return qp;
}

// ---------------------------------------------------------------------------
// consumeLobbyParams — used by game pages (tui_game / native_game)
// ---------------------------------------------------------------------------

/**
 * Parse the lobby redirect params from `window.location.search` and return a
 * structured descriptor.  Returns `null` if no lobby params are present (the
 * page was opened standalone, not from the lobby redirect).
 *
 * @returns {LobbyParams|null}
 *
 * @typedef {object} LobbyParams
 * @property {'create'|'join'} action
 * @property {string}  gameName
 * @property {string}  gamePass           - may be ''
 * @property {string}  playerName         - may be ''
 * @property {string}  wsUrl              - may be ''
 * @property {string}  serverPass         - server-level password (rarely used)
 * @property {string}  deckName           - may be ''
 * @property {'tui'|'native'} ui
 * @property {'local'|'network'} mode     - game mode hint
 * @property {string}  reconnectToken     - may be '' (reattach still a stub, mtg-682)
 * @property {'human'|'heuristic'|'random'|'zero'} controller - our network
 *           controller. Defaults to 'human' (a person plays the web client).
 *           An explicit &controller= lets an AI drive the web client over the
 *           network (the spec's AI-driver acceptance-test strategy, mtg-35z3s).
 */
export function consumeLobbyParams() {
    const qs = new URLSearchParams(window.location.search);
    const createName = qs.get('lobby_create');
    const joinName   = qs.get('lobby_join');
    if (!createName && !joinName) return null;

    const ui = qs.get('ui') === 'native' ? 'native' : 'tui';
    const mode = qs.get('mode') === 'local' ? 'local' : 'network';
    const ctrl = qs.get('controller');
    const controller = ['human', 'heuristic', 'random', 'zero'].includes(ctrl) ? ctrl : 'human';
    return {
        action:         createName ? 'create' : 'join',
        gameName:       createName || joinName || '',
        gamePass:       qs.get('lobby_pass')       || '',
        playerName:     qs.get('name')             || '',
        wsUrl:          qs.get('ws')               || '',
        serverPass:     qs.get('server_pass')      || '',
        deckName:       qs.get('deck')             || '',
        reconnectToken: qs.get('reconnect_token')  || '',
        ui,
        mode,
        controller,
    };
}

// ---------------------------------------------------------------------------
// consumeLocalGameParams — local (non-network) boot params for the game pages
// ---------------------------------------------------------------------------

/**
 * After the lobby-redo (mtg-35z3s page 3) the game pages are PURE renderers
 * with no built-in launcher: they boot entirely from URL params. The network
 * boot uses `consumeLobbyParams()` (lobby_create/lobby_join). For LOCAL
 * (AI-vs-AI / dev / renderer-test) boots there is no lobby, so the page
 * accepts an explicit local-game param contract instead:
 *
 *   ?mode=local                  → boot a local game (no network)
 *   &p1_deck=<deck_name>         → Player 1 (our) deck   (required)
 *   &p2_deck=<deck_name>         → Player 2 (opponent) deck (defaults to p1_deck)
 *   &p1=human|heuristic|random|zero   → P1 controller (default: heuristic)
 *   &p2=human|heuristic|random|zero   → P2 controller (default: heuristic)
 *   &seed=<u64>                  → RNG seed (default: time-based / random)
 *   &debug=true                  → enable TRACE logging
 *
 * Returns `null` when `mode` is not `local` OR no `p1_deck` is supplied, so the
 * caller can fall back to the network path / the "launch from the lobby"
 * degraded message. Kept here (not duplicated per page) so both game pages
 * share one parser (DRY).
 *
 * @returns {LocalGameParams|null}
 *
 * @typedef {object} LocalGameParams
 * @property {string} p1Deck
 * @property {string} p2Deck
 * @property {'human'|'heuristic'|'random'|'zero'} p1Controller
 * @property {'human'|'heuristic'|'random'|'zero'} p2Controller
 * @property {string} seed              - '' means "pick one"
 * @property {boolean} debug
 */
export function consumeLocalGameParams() {
    const qs = new URLSearchParams(window.location.search);
    if (qs.get('mode') !== 'local') return null;
    const p1Deck = qs.get('p1_deck') || '';
    if (!p1Deck) return null;
    const ctrl = (v, dflt) =>
        ['human', 'heuristic', 'random', 'zero'].includes(v) ? v : dflt;
    return {
        p1Deck,
        p2Deck:       qs.get('p2_deck') || p1Deck,
        p1Controller: ctrl(qs.get('p1'), 'heuristic'),
        p2Controller: ctrl(qs.get('p2'), 'heuristic'),
        seed:         qs.get('seed') || '',
        debug:        qs.get('debug') === 'true',
    };
}

/**
 * Network boot WITHOUT a lobby action (auto-match). The normal lobby flow
 * carries a `lobby_create=`/`lobby_join=` game name (handled by
 * consumeLobbyParams). But the server can also auto-match two connecting
 * clients into one game when neither names a game — this is what the network
 * AI-vs-AI e2e harness relies on (the native `mtg connect` client and the web
 * client just connect and get paired). Such a boot is requested with
 * `?mode=network` and NO lobby_create/lobby_join:
 *
 *   ?mode=network&ws=<ws>&name=<name>&deck=<deck>&controller=random[&server_pass=]
 *
 * Returns `null` unless `mode=network` AND there is no lobby_create/lobby_join
 * (so the lobby flow keeps priority). The resulting boot connects with a `null`
 * lobby action — i.e. server auto-match.
 *
 * @returns {NetworkParams|null}
 *
 * @typedef {object} NetworkParams
 * @property {string} deckName
 * @property {string} playerName
 * @property {string} wsUrl
 * @property {string} serverPass
 * @property {'human'|'heuristic'|'random'|'zero'} controller
 */
export function consumeNetworkParams() {
    const qs = new URLSearchParams(window.location.search);
    if (qs.get('mode') !== 'network') return null;
    if (qs.get('lobby_create') || qs.get('lobby_join')) return null; // lobby flow owns these
    const ctrl = qs.get('controller');
    const controller = ['human', 'heuristic', 'random', 'zero'].includes(ctrl) ? ctrl : 'human';
    return {
        deckName:   qs.get('deck')        || '',
        playerName: qs.get('name')        || '',
        wsUrl:      qs.get('ws')          || '',
        serverPass: qs.get('server_pass') || '',
        controller,
    };
}

/**
 * True when the game page was opened with `?auto_run=true` (mtg-780).
 *
 * Web games now start PAUSED by default: an AI-vs-AI (e.g. random/random) game
 * no longer auto-runs to completion the instant both clients load — the player
 * advances it with Space / the "Run 1 Turn" button / the "Auto Run" toggle.
 * `?auto_run=true` opts back into the old auto-advancing behaviour (so the
 * benchmark / e2e harnesses that WANT an unattended run can request it). Parsed
 * here so both game pages share ONE definition of the param (DRY).
 *
 * @returns {boolean}
 */
export function isAutoRunRequested() {
    return new URLSearchParams(window.location.search).get('auto_run') === 'true';
}

// ---------------------------------------------------------------------------
// consumeGamePrefs — per-game display/debug prefs re-homed from the launcher
// ---------------------------------------------------------------------------

/** All recognised image-source ids, in the fixed fallback order. */
export const IMAGE_SOURCE_IDS = ['local', 'scryfall', 'gatherer'];

/**
 * Parse the per-game preference params (`images`, `img_src`, `debug`) that the
 * launcher forwards to a game page. These were controls on the old built-in
 * launcher (Show-Card-Images, the image-source checkboxes, Debug-Mode); after
 * the lobby redo they live on launcher.html and ride here as URL params. Parsed
 * in ONE place so neither game page re-derives the contract (DRY, mtg-695).
 *
 * `imageSources` is filtered to the known ids in canonical fallback order; an
 * absent/empty `img_src` falls back to `defaults.imageSources` (all sources).
 * `showImages`/`debug` default to `defaults.*` when the param is absent, so a
 * caller can keep each game page's own historical default (native: images on;
 * tui: images off).
 *
 * @param {object} [defaults]
 * @param {boolean} [defaults.showImages=true]
 * @param {boolean} [defaults.debug=false]
 * @param {string[]} [defaults.imageSources=IMAGE_SOURCE_IDS]
 * @returns {{showImages: boolean, imageSources: string[], debug: boolean}}
 */
export function consumeGamePrefs(defaults) {
    const d = Object.assign(
        { showImages: true, debug: false, imageSources: IMAGE_SOURCE_IDS.slice() },
        defaults || {},
    );
    const qs = new URLSearchParams(window.location.search);

    let showImages = d.showImages;
    if (qs.has('images')) showImages = qs.get('images') === 'true';

    let debug = d.debug;
    if (qs.has('debug')) debug = qs.get('debug') === 'true';

    let imageSources = d.imageSources;
    const raw = qs.get('img_src');
    if (raw !== null) {
        const picked = raw.split(',').map((s) => s.trim().toLowerCase())
            .filter((s) => IMAGE_SOURCE_IDS.includes(s));
        // Re-impose canonical fallback order; dedupe.
        imageSources = IMAGE_SOURCE_IDS.filter((id) => picked.includes(id));
    }
    return { showImages, imageSources, debug };
}

// ---------------------------------------------------------------------------
// Sticky-param propagation across inter-page navigation
// ---------------------------------------------------------------------------

/**
 * The params that must SURVIVE every hop across the lobby↔launcher↔game↔
 * deck-editor pages. The local-image unlock is the load-bearing one (the
 * gate is meaningless if a back-to-lobby click silently drops it), but the
 * player identity, server URL, and debug/image prefs are equally session-wide.
 *
 * `release` is the CAS release token (mtg-704): the content-hashed manifest
 * hash that identifies which deployment a page belongs to. The UNHASHED
 * index.html seeds it (the deploy bakes the current token); every hashed page
 * then RELAYS `release=` from its own URL onto BOTH its forward links and its
 * back-edges so navigation stays pinned to one release. Because it rides the
 * sticky-param plumbing, it is MERGED into each destination's query string
 * WITHOUT clobbering the other params — never dropping deck/name/seed/ws/etc.
 */
export const STICKY_PARAM_KEYS = ['advanced_options', 'allow_local_img_load', 'debug', 'images', 'img_src', 'name', 'release', 'ws'];

/**
 * Copy the sticky params (those present in `source`, default
 * `window.location.search`) onto `url`'s query string WITHOUT clobbering params
 * already set on `url`. Returns the same URL object for chaining. Centralised
 * here so every inter-page link forwards the same set (DRY, mtg-695) — the
 * back-to-lobby / deck-editor links were dropping `allow_local_img_load` and
 * friends, defeating the sticky gate.
 *
 * @param {URL} url                      - destination URL to augment (mutated)
 * @param {URLSearchParams|string} [source] - source params (default current URL)
 * @returns {URL}
 */
export function forwardStickyParams(url, source) {
    const src = source instanceof URLSearchParams
        ? source
        : new URLSearchParams(source !== undefined ? source : window.location.search);
    for (const key of STICKY_PARAM_KEYS) {
        if (src.has(key) && !url.searchParams.has(key)) {
            url.searchParams.set(key, src.get(key));
        }
    }
    return url;
}

/**
 * Build a relative URL string for `page` carrying the session's sticky params
 * (and any explicit `extra` params). Used for PROGRAMMATIC navigation (e.g. the
 * game page's exit-to-lobby), the counterpart of forwardStickyParamsOnAnchor
 * for static links. DRY: same STICKY_PARAM_KEYS set (mtg-695).
 *
 * @param {string} page                  - e.g. "index.html"
 * @param {object} [extra]               - extra query params to set first
 * @param {URLSearchParams|string} [source] - source params (default current URL)
 * @returns {string}  relative URL (e.g. "index.html?allow_local_img_load=true")
 */
export function stickyUrl(page, extra, source) {
    const url = new URL(page, window.location.href);
    if (extra) for (const [k, v] of Object.entries(extra)) {
        if (v !== undefined && v !== null && v !== '') url.searchParams.set(k, String(v));
    }
    forwardStickyParams(url, source);
    return url.pathname.split('/').pop() + url.search + url.hash;
}

/**
 * Apply `forwardStickyParams` to an anchor element's `href` in place. Skips
 * disabled / hrefless anchors. Convenience wrapper for the common "rewrite a
 * static <a href> so it carries the session's sticky params" case.
 *
 * @param {HTMLAnchorElement|null} anchor
 * @param {URLSearchParams|string} [source]
 */
export function forwardStickyParamsOnAnchor(anchor, source) {
    if (!anchor) return;
    const href = anchor.getAttribute('href');
    if (!href) return;
    const url = new URL(href, window.location.href);
    forwardStickyParams(url, source);
    // Preserve a relative href (same-origin pages are all relative siblings).
    anchor.setAttribute('href', url.pathname.split('/').pop() + url.search + url.hash);
}

/**
 * True when the page was opened with NEITHER a lobby (network) boot param NOR a
 * local-game boot param NOR a no-lobby network boot — i.e. a bare direct visit.
 * The game pages use this to decide whether to show the "launch from the lobby"
 * degraded message instead of a (now-deleted) built-in launcher.
 *
 * @returns {boolean}
 */
export function hasNoLaunchParams() {
    return consumeLobbyParams() === null
        && consumeLocalGameParams() === null
        && consumeNetworkParams() === null;
}

// ---------------------------------------------------------------------------
// applyLobbyParamsToForm — populate a game-page launcher form from LobbyParams
// ---------------------------------------------------------------------------

/**
 * Pre-fill the game-page launcher form fields from the parsed LobbyParams.
 * Game pages call this after WASM init so the UI already shows the
 * right values before the auto-launch fires.
 *
 * Shared between tui_game.html and native_game.html.  Each page has
 * slightly different element IDs for their network-mode fields, so the
 * caller passes the element IDs to use.
 *
 * @param {LobbyParams} params
 * @param {object} [fieldIds]  - overridable element IDs (defaults match tui_game.html)
 * @param {string} [fieldIds.gameModeSelectId]   - id of the game-mode <select>
 * @param {string} [fieldIds.serverUrlInputId]   - id of the server-url <input>
 * @param {string} [fieldIds.serverPassInputId]  - id of the server-password <input>
 * @param {string} [fieldIds.playerNameInputId]  - id of the player-name <input>
 * @param {string} [fieldIds.p1DeckSelectId]     - id of the p1-deck <select>
 * @param {string} [fieldIds.p1ControllerSelectId] - id of the p1-controller <select>
 * @param {string} [fieldIds.networkStatusId]    - id of the network-status <span>
 * @param {function} [updateGameModeUI]          - optional callback to refresh mode-dependent display
 */
export function applyLobbyParamsToForm(params, fieldIds, updateGameModeUI) {
    const ids = Object.assign({
        gameModeSelectId:     'game-mode',
        serverUrlInputId:     'server-url',
        serverPassInputId:    'server-password',
        playerNameInputId:    'player-name',
        p1DeckSelectId:       'p1-deck',
        p1ControllerSelectId: 'p1-controller',
        networkStatusId:      'network-status',
    }, fieldIds || {});

    // Switch to network mode.
    const gameMode = document.getElementById(ids.gameModeSelectId);
    if (gameMode) {
        gameMode.value = 'network';
        if (typeof updateGameModeUI === 'function') updateGameModeUI();
    }

    // Fill connection fields.
    const wsField = document.getElementById(ids.serverUrlInputId);
    if (wsField && params.wsUrl) wsField.value = params.wsUrl;

    const pwField = document.getElementById(ids.serverPassInputId);
    if (pwField) pwField.value = params.serverPass;

    const nameField = document.getElementById(ids.playerNameInputId);
    if (nameField && params.playerName) nameField.value = params.playerName;

    // Pick a deck.
    const p1Deck = document.getElementById(ids.p1DeckSelectId);
    if (p1Deck) {
        const deckOverride = params.deckName;
        if (deckOverride && Array.from(p1Deck.options).some(o => o.value === deckOverride)) {
            p1Deck.value = deckOverride;
        } else if (!p1Deck.value && p1Deck.options.length > 0) {
            p1Deck.value = p1Deck.options[0].value;
        }
    }

    // Ensure the controller is Human.
    const p1Ctrl = document.getElementById(ids.p1ControllerSelectId);
    if (p1Ctrl) {
        const hasHuman = Array.from(p1Ctrl.options).some(o => o.value === 'human');
        if (hasHuman) p1Ctrl.value = 'human';
    }

    // Show a status hint.
    const statusEl = document.getElementById(ids.networkStatusId);
    if (statusEl) {
        statusEl.textContent = params.action === 'create'
            ? `Auto-creating "${params.gameName}"…`
            : `Auto-joining "${params.gameName}"…`;
        statusEl.style.color = '#4cc9f0';
    }
}

// ---------------------------------------------------------------------------
// buildLobbyAction — convert LobbyParams to the object network.js expects
// ---------------------------------------------------------------------------

/**
 * Convert a LobbyParams descriptor into the `lobbyAction` object that
 * `MTGNetworkClient.connect()` (web/network.js) accepts.
 *
 * @param {LobbyParams} params
 * @returns {{ kind: 'create'|'join', gameName: string, gamePassword: string }}
 */
export function buildLobbyAction(params) {
    return {
        kind:         params.action,
        gameName:     params.gameName,
        gamePassword: params.gamePass,
    };
}
