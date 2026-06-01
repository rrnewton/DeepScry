/**
 * lobby_launcher.js — Shared lobby-redirect + param-plumbing module.
 *
 * Used by:
 *   - web/index.html   (the lobby: builds redirect URLs, offers TUI vs native choice)
 *   - web/tui_game.html    (consumer: reads query params and auto-launches)
 *   - web/native_game.html (consumer: reads query params and auto-launches)
 *
 * Design goals (Phase 2 / mtg-phase2-native-network):
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
 *   &allow_local_img_load=true    → propagate the sticky local-image unlock
 *   &ui=tui|native                → which game-page UI to land on (default: tui)
 *   &mode=local|network           → game mode hint (default: network when from lobby)
 *   &reconnect_token=<token>      → reconnect token from GameStarted (Phase 1 stub)
 * ──────────────────────────────────────────────────────────────────────────────
 */

'use strict';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/** Default launch target when `?ui=` is absent. */
export const DEFAULT_UI = 'tui';

/** File names for the two game UIs (no path prefix — same origin). */
export const GAME_PAGE = {
    tui:    'tui_game.html',
    native: 'native_game.html',
};

// ---------------------------------------------------------------------------
// buildRedirectUrl — used by index.html to construct the redirect URL
// ---------------------------------------------------------------------------

/**
 * Build the redirect URL that the lobby sends the user to when they click
 * "Launch Game" (creator) or "Join" (joiner).
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
 * @param {string}  [opts.reconnectToken]   - reconnect token from GameStarted
 * @returns {string}  Full relative URL (e.g. "tui_game.html?lobby_create=...")
 */
export function buildRedirectUrl(opts) {
    const ui = opts.ui === 'native' ? 'native' : 'tui';
    const page = GAME_PAGE[ui];
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
    if (opts.allowLocalImgLoad) qp.set('allow_local_img_load', 'true');
    if (opts.reconnectToken)   qp.set('reconnect_token', opts.reconnectToken);
    qp.set('ui', ui);
    // Default to 'network' mode when coming from the lobby redirect.
    qp.set('mode', opts.mode === 'local' ? 'local' : 'network');

    return page + '?' + qp.toString();
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
 * @property {string}  reconnectToken     - may be '' (Phase 1 stub)
 */
export function consumeLobbyParams() {
    const qs = new URLSearchParams(window.location.search);
    const createName = qs.get('lobby_create');
    const joinName   = qs.get('lobby_join');
    if (!createName && !joinName) return null;

    const ui = qs.get('ui') === 'native' ? 'native' : 'tui';
    const mode = qs.get('mode') === 'local' ? 'local' : 'network';
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
    };
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
