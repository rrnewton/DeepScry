/**
 * MTG Network Client - WebSocket wrapper for WASM network gameplay
 *
 * This module handles WebSocket connection to the game server and bridges
 * JavaScript events to WASM exports.
 *
 * Architecture:
 * - JavaScript owns the WebSocket connection
 * - WASM queues outbound messages; JavaScript polls and sends them
 * - Inbound messages are passed to WASM via network_on_message()
 * - WASM controllers use NeedInput pattern (non-blocking)
 */

export class MTGNetworkClient {
    constructor(wasmModule) {
        this.wasm = wasmModule;
        this.ws = null;
        this.serverUrl = null;
        this.reconnecting = false;
        this.messageQueue = [];  // Buffer messages if WS not ready
        this.pollInterval = null;
        this.onStateChange = null;  // Callback for UI updates
        this.onError = null;  // Callback for error display
        this.onGameReady = null;  // Callback when game starts
        this.onMessageProcessed = null;  // Callback after each message (for triggering game loop)
        // Phase 0 reconnect UX (mtg-z459a). Fired when the in-game socket drops
        // and we begin attempting to re-open it. The argument is a small status
        // object `{ attempt, maxAttempts, recovered }` so the page can render an
        // HONEST transient "reconnecting…" banner. IMPORTANT: Phase 0 only
        // re-opens the socket; it does NOT yet replay the action log to catch up
        // (that is the determinism-gated Phase 1+ resume in mtg-z459a). So a
        // successful socket re-open is reported with `recovered: false` — we must
        // NOT claim the game resumed when it cannot actually continue.
        this.onReconnecting = null;
        // Two-phase bug report callbacks (mtg-749). Phase 1 confirms the
        // server-side disk write; phase 2 reports the GitHub issue outcome.
        this.onBugReportStored = null;
        this.onBugReportIssueResult = null;
        this.gameReadyFired = false;  // Track if onGameReady was already called
        // mtg web-ui-fixes: gate the chatty "[Network] Received/Sending/State/
        // …" traffic logs behind the page's debug-tracing flag. Default OFF so a
        // normal user's console is quiet; the page sets `client.debug =
        // isDebugMode()`. Genuine errors (console.error) are NOT gated and are
        // additionally surfaced to the UI via onError.
        this.debug = false;
        // mtg-891: set true by the page when the game has CONCLUDED (view
        // model game_over). The server closes the socket with a non-clean 1006
        // close right after a game ends — that is NORMAL, not a lost connection,
        // so onclose/onerror must NOT escalate it to the red banner or attempt a
        // reconnect once the game is over.
        this.gameEnded = false;
        // Phase 0 keepalive (mtg-z459a). A periodic application-level
        // ClientMessage::Ping over the live in-game socket keeps idle
        // connections warm so Safari / energy-saver / proxies do not reap them
        // during long AI-vs-AI auto-runs (the user-reported "network connection
        // was lost"). The server answers every Ping with a Pong it then DROPS
        // (client.rs maps Pong → None); Ping/Pong is OUT-OF-BAND from the
        // deterministic fact/choice stream — it never touches action_count, the
        // undo log, or the view hash. 25s < the ~100s proxy idle cutoff (same
        // value the lobby waiting-room keepalive uses in launcher.html).
        this._keepaliveTimer = null;
        this.KEEPALIVE_INTERVAL_MS = 25000;
        // Exposed counter so e2e can assert pings are emitted (driving a true
        // ~100s idle in a test is impractical — mirrors launcher.html's gate).
        this.pingsSent = 0;
        // Phase 0 reconnect attempt bookkeeping (mtg-z459a). Bounded retries so
        // a permanently-dead server does not spin forever.
        this._reconnectAttempt = 0;
        this.MAX_RECONNECT_ATTEMPTS = 3;
        this.RECONNECT_DELAY_MS = 3000;
        // The connect() args, stashed so a Phase 0 reconnect can re-open the
        // SAME socket with the SAME parameters.
        this._connectArgs = null;
    }

    /** Gated informational log — only emitted when debug tracing is ON. */
    _log(...args) {
        if (this.debug) console.log(...args);
    }

    /**
     * Connect to game server.
     *
     * The optional `lobbyAction` argument (mtg-474) selects the first
     * message sent on WS open:
     *   - omitted / null  → legacy `Authenticate` against DEFAULT_LOBBY_GAME
     *   - { kind: 'create', gameName, gamePassword } → `CreateGame`
     *   - { kind: 'join',   gameName, gamePassword } → `JoinGame`
     *
     * Used by the landing-page-lobby (web/index.html) redirect that lands on
     * tui_game.html with `?lobby_create=...` / `?lobby_join=...` query params.
     *
     * @param {string} serverUrl - WebSocket URL (e.g., "ws://localhost:17771")
     * @param {string} password - Server password
     * @param {string} playerName - Player's display name
     * @param {string} deckJson - Deck submission as JSON
     * @param {object} [lobbyAction] - Optional create/join descriptor
     */
    connect(serverUrl, password, playerName, deckJson, lobbyAction) {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this._log('[Network] Already connected, disconnecting first');
            this.disconnect();
        }

        this.serverUrl = serverUrl;
        // Stash for a Phase 0 reconnect (re-open the same socket, same params).
        this._connectArgs = { serverUrl, password, playerName, deckJson, lobbyAction };
        this._log(`[Network] Connecting to ${serverUrl}...`);

        // Initialize WASM network state
        this.wasm.network_init(serverUrl, password, playerName, deckJson);

        // Configure the WS-open dispatch (Authenticate vs CreateGame vs JoinGame).
        // The WASM client exposes setters for the CreateGame/JoinGame paths
        // (mtg-474); absence reverts to the legacy Authenticate behaviour.
        if (lobbyAction && lobbyAction.kind === 'create' && typeof this.wasm.network_set_lobby_create === 'function') {
            this.wasm.network_set_lobby_create(lobbyAction.gameName || '', lobbyAction.gamePassword || '');
        } else if (lobbyAction && lobbyAction.kind === 'join' && typeof this.wasm.network_set_lobby_join === 'function') {
            this.wasm.network_set_lobby_join(lobbyAction.gameName || '', lobbyAction.gamePassword || '');
        } else if (typeof this.wasm.network_clear_lobby_action === 'function') {
            this.wasm.network_clear_lobby_action();
        }

        try {
            this.ws = new WebSocket(serverUrl);
        } catch (e) {
            const msg = `Failed to create WebSocket: ${e.message}`;
            console.error(`[Network] ${msg}`);
            this.wasm.network_on_error(msg);
            if (this.onError) this.onError(msg);
            return;
        }

        this.ws.onopen = () => {
            this._log('[Network] WebSocket connected');
            this.wasm.network_on_open();
            this._notifyStateChange();

            // A successful (re)open clears reconnect bookkeeping.
            this.reconnecting = false;
            this._reconnectAttempt = 0;

            // Start polling for outbound messages
            this._startOutboundPoll();

            // Keep the idle in-game socket warm (Phase 0, mtg-z459a) so long
            // AI-vs-AI auto-runs survive Safari/proxy idle reaping.
            this._startKeepalive();

            // Send any queued messages
            this._flushMessageQueue();
        };

        this.ws.onmessage = (event) => {
            const data = event.data;
            this._log('[Network] Received:', data.substring(0, 200) + (data.length > 200 ? '...' : ''));

            let msg = null;
            try {
                msg = JSON.parse(data);
            } catch (e) {
                // Non-JSON messages still flow into WASM for normal handling/error reporting
            }

            // mtg-891: when the game CONCLUDES legitimately, the server sends a
            // `game_ended` and then closes the socket (non-clean 1006); we mark
            // the game ended so that ensuing close is not mis-escalated as a lost
            // connection. (The page also sets this from its view-model game_over.)
            //
            // mtg-redo-fix: but the server ALSO sends a `game_ended` when it
            // ABORTS a game on a peer DISCONNECT or fatal error mid-game — and in
            // that case the survivor's connection IS being lost and MUST get the
            // normal connection-lost / reconnect handling, NOT suppression. The
            // two are distinguishable by the message's structured fields: a
            // legitimate conclusion carries a real outcome (a `winner`, or a
            // genuine draw that actually played out → `action_count > 0`), while
            // the abort-teardown is synthesized by the server's error path
            // (server.rs: the `Err(_)` branch) as `winner: null` AND
            // `action_count: 0` — no winner and zero actions taken, i.e. NOT a
            // real game outcome. Only a legitimate conclusion suppresses the
            // disconnect handling.
            if (msg?.type === 'game_ended' || msg?.type === 'game_over') {
                if (isLegitimateGameEnd(msg)) {
                    this.gameEnded = true;
                    console.log('[Network] Game ended');
                }
                // else: an abort/disconnect teardown — fall through so the
                // ensuing close runs the normal connection-lost + reconnect path.
            }

            // Two-phase bug report (mtg-749): phase 1 = disk-write
            // confirmation (immediate), phase 2 = GitHub issue outcome.
            if (msg?.type === 'bug_report_stored') {
                if (this.onBugReportStored) {
                    this.onBugReportStored(msg);
                }
                return;
            }
            if (msg?.type === 'bug_report_issue_result') {
                if (this.onBugReportIssueResult) {
                    this.onBugReportIssueResult(msg);
                }
                return;
            }

            // Pass to WASM for processing
            this.wasm.network_on_message(data);
            this._notifyStateChange();

            // Check if game is now ready (only fire once)
            if (!this.gameReadyFired && this.wasm.network_is_game_ready() && this.onGameReady) {
                this.gameReadyFired = true;
                this.onGameReady();
            }

            // Notify that a message was processed (triggers game loop for Human controller)
            if (msg) {
                if (msg.type === 'choice_request' ||
                    msg.type === 'choice_accepted' ||
                    msg.type === 'opponent_choice' ||
                    msg.type === 'game_started') {
                    if (this.onMessageProcessed) {
                        this.onMessageProcessed(msg.type);
                    }
                }
            }
        };

        this.ws.onclose = (event) => {
            this._log(`[Network] WebSocket closed: code=${event.code}, reason=${event.reason}`);
            this.wasm.network_on_close();
            this._notifyStateChange();
            this._stopOutboundPoll();
            this._stopKeepalive();

            // A NON-clean close is a real disconnect ("connection lost"): the
            // user must see it, not just the console (mtg web-ui-fixes fix #4).
            // A clean close (code 1000, client-initiated disconnect) is normal
            // and stays quiet — and so is the non-clean 1006 close the server
            // does right AFTER a game ends (this.gameEnded), which is NOT a lost
            // connection and must not raise the red banner or reconnect.
            if (!event.wasClean && !this.gameEnded) {
                const detail = event.reason ? `: ${event.reason}` : (event.code ? ` (code ${event.code})` : '');
                const msg = `Connection to the game server was lost${detail}.`;
                console.error(`[Network] ${msg}`);
                if (this.onError) this.onError(msg);
                if (!this.reconnecting) {
                    this._scheduleReconnect();
                }
            }
        };

        this.ws.onerror = (event) => {
            // The browser fires onerror on connection failure (e.g.
            // "WebSocket connection to wss://…/lobby failed: The network
            // connection was lost"). Surface it to the UI, not just the console
            // (mtg web-ui-fixes fix #4). The Event itself carries no message, so
            // give an actionable one. Suppress once the game has ended — a
            // post-game socket error is not actionable for the user.
            console.error('[Network] WebSocket error:', event);
            const msg = 'Network error connecting to the game server.';
            this.wasm.network_on_error(msg);
            if (this.onError && !this.gameEnded) this.onError(msg);
        };
    }

    /**
     * Disconnect from server
     */
    disconnect() {
        this.reconnecting = false;
        this._reconnectAttempt = 0;
        this._stopOutboundPoll();
        this._stopKeepalive();

        if (this.ws) {
            this.ws.close(1000, 'Client disconnected');
            this.ws = null;
        }
    }

    /**
     * Get current connection state
     * @returns {string} State name from WASM
     */
    getState() {
        return this.wasm.network_get_state();
    }

    /**
     * Check if game is ready to play
     * @returns {boolean}
     */
    isGameReady() {
        return this.wasm.network_is_game_ready();
    }

    /**
     * Get our player ID (after game starts)
     * @returns {number|null}
     */
    getOurPlayerId() {
        return this.wasm.network_get_our_player_id();
    }

    /**
     * Send a message immediately if connected, otherwise queue it
     * @param {string} json - JSON message to send
     */
    send(json) {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this._log('[Network] Sending:', json.substring(0, 200) + (json.length > 200 ? '...' : ''));
            this.ws.send(json);
        } else {
            this._log('[Network] Queuing message (not connected)');
            this.messageQueue.push(json);
        }
    }

    /**
     * Check whether the underlying WebSocket is currently open.
     * @returns {boolean}
     */
    isConnected() {
        return !!(this.ws && this.ws.readyState === WebSocket.OPEN);
    }

    // --- Private methods ---

    _startOutboundPoll() {
        if (this.pollInterval) return;

        // Poll every 50ms for outbound messages from WASM
        this.pollInterval = setInterval(() => {
            this._sendPendingMessages();
        }, 50);
    }

    _stopOutboundPoll() {
        if (this.pollInterval) {
            clearInterval(this.pollInterval);
            this.pollInterval = null;
        }
    }

    // --- Phase 0 keepalive (mtg-z459a) ---
    //
    // A periodic application-level Ping over the live in-game socket. This is
    // the in-game counterpart of the lobby waiting-room keepalive in
    // launcher.html. The Ping is queued by the WASM client
    // (`network_ping()` → `ClientMessage::Ping { timestamp_ms }`); the server's
    // in-game loop answers it with a `Pong` (server.rs) which the client then
    // DROPS (client.rs: `ServerMessage::Pong => None`). It is OUT-OF-BAND from
    // the deterministic game stream — no action_count, undo-log, or view-hash
    // effect — so determinism is unaffected.

    _startKeepalive() {
        this._stopKeepalive();
        if (typeof this.wasm.network_ping !== 'function') {
            // Older WASM without the ping export: skip silently (no keepalive,
            // but never crash). The server tolerates the absence.
            return;
        }
        this._keepaliveTimer = setInterval(() => {
            this._sendKeepalivePing();
        }, this.KEEPALIVE_INTERVAL_MS);
    }

    _stopKeepalive() {
        if (this._keepaliveTimer !== null) {
            clearInterval(this._keepaliveTimer);
            this._keepaliveTimer = null;
        }
    }

    /**
     * Emit one keepalive ping over the live socket. Exposed (via the timer and
     * directly) so an e2e can drive a single ping and assert it is emitted —
     * a real ~100s idle is impractical to drive in a test (mirrors the
     * launcher.html lobby gate). Returns true if a ping was queued+flushed.
     */
    _sendKeepalivePing() {
        if (!this.isConnected()) return false;
        if (typeof this.wasm.network_ping !== 'function') return false;
        // Queue the Ping in WASM, then flush WASM's outbound queue immediately
        // so it goes out on this tick rather than waiting for the 50ms poll.
        this.wasm.network_ping();
        this._sendPendingMessages();
        this.pingsSent += 1;
        return true;
    }

    _sendPendingMessages() {
        if (!this.ws || this.ws.readyState !== WebSocket.OPEN) return;

        // Poll WASM for outbound messages
        let msg;
        while ((msg = this.wasm.network_get_outbound_message()) !== undefined && msg !== null) {
            this._log('[Network] Sending queued:', msg.substring(0, 200) + (msg.length > 200 ? '...' : ''));
            this.ws.send(msg);
        }
    }

    _flushMessageQueue() {
        while (this.messageQueue.length > 0) {
            const msg = this.messageQueue.shift();
            this.send(msg);
        }
    }

    _notifyStateChange() {
        if (this.onStateChange) {
            this.onStateChange(this.getState());
        }
    }

    /**
     * Phase 0 reconnect (mtg-z459a). Honestly attempt to RE-OPEN the dropped
     * in-game socket, with bounded retries. This deliberately does NOT yet
     * resume the in-progress game: Phase 0 has no action-log catch-up, so even
     * if the socket re-opens we are a FRESH connection that cannot continue the
     * existing game. We therefore report progress with `recovered: false` and
     * never claim the game resumed (a false "resumed" would be worse than the
     * old freeze). True state recovery is the determinism-gated Phase 1+ resume
     * tracked in mtg-z459a.
     *
     * The honest UX contract:
     *   - while retrying:        onReconnecting({ attempt, maxAttempts, recovered:false })
     *   - socket re-opened:      onReconnecting({ attempt, maxAttempts, recovered:false })
     *                            + onError("Reconnected to the server, but this
     *                              game cannot be resumed yet — start a new game.")
     *   - all attempts failed:   onError("Connection lost and could not
     *                              reconnect. Please reload to start a new game.")
     */
    _scheduleReconnect() {
        if (this.reconnecting) return;
        if (!this._connectArgs) {
            // No params to reconnect with — be honest, don't pretend.
            if (this.onError) this.onError('Connection lost. Please reload to reconnect.');
            return;
        }

        this.reconnecting = true;
        this._reconnectAttempt = 0;
        this._attemptReconnect();
    }

    _attemptReconnect() {
        // Stop if the game ended or someone cleanly disconnected meanwhile.
        if (!this.reconnecting || this.gameEnded) {
            this.reconnecting = false;
            return;
        }

        this._reconnectAttempt += 1;
        const attempt = this._reconnectAttempt;
        const maxAttempts = this.MAX_RECONNECT_ATTEMPTS;

        if (this.onReconnecting) {
            this.onReconnecting({ attempt, maxAttempts, recovered: false });
        }
        this._log(`[Network] Reconnect attempt ${attempt}/${maxAttempts} in ${this.RECONNECT_DELAY_MS}ms...`);

        setTimeout(() => {
            if (!this.reconnecting || this.gameEnded) {
                this.reconnecting = false;
                return;
            }

            // Probe a fresh socket WITHOUT tearing down WASM game state via the
            // full connect() init (which would also reset the queue). We just
            // open a raw socket to confirm reachability; on success we report an
            // HONEST "reconnected but cannot resume" state.
            let probe;
            try {
                probe = new WebSocket(this._connectArgs.serverUrl);
            } catch (e) {
                this._afterReconnectFailure();
                return;
            }

            probe.onopen = () => {
                // Reachable again. Phase 0 cannot catch the new socket up to the
                // in-progress game, so close the probe and tell the user the
                // honest truth: the server is back but THIS game can't continue.
                try { probe.close(1000, 'phase0-probe'); } catch (_) { /* ignore */ }
                this.reconnecting = false;
                this._reconnectAttempt = 0;
                if (this.onReconnecting) {
                    this.onReconnecting({ attempt, maxAttempts, recovered: false });
                }
                if (this.onError) {
                    this.onError('Reconnected to the server, but this game cannot be resumed yet — start a new game.');
                }
            };

            probe.onerror = () => {
                try { probe.close(); } catch (_) { /* ignore */ }
                this._afterReconnectFailure();
            };
        }, this.RECONNECT_DELAY_MS);
    }

    _afterReconnectFailure() {
        if (!this.reconnecting) return;
        if (this._reconnectAttempt < this.MAX_RECONNECT_ATTEMPTS) {
            this._attemptReconnect();
        } else {
            this.reconnecting = false;
            this._reconnectAttempt = 0;
            if (this.onError) {
                this.onError('Connection lost and could not reconnect. Please reload to start a new game.');
            }
        }
    }
}

/**
 * Does a `game_ended` server message represent a LEGITIMATE game conclusion
 * (a real win/decking, or a genuine draw that actually played out) — as opposed
 * to the synthetic teardown the server emits when it ABORTS a game on a peer
 * disconnect or fatal error mid-game?
 *
 * Server contract (mtg-engine/src/network/server.rs, the GameEnded send sites):
 *   - Ok(result)  →  real `winner` + real `action_count` (the undo-log length,
 *                    always > 0 for a game that actually played).
 *   - Err(_)      →  `winner: null`, `reason: "draw"`, `action_count: 0`
 *                    (no winner AND zero actions = not a real outcome).
 *
 * So a legitimate conclusion is one that has a winner OR took at least one
 * action. The abort-teardown (winner null AND action_count 0) is NOT legitimate
 * and must be handled as a connection loss, not a normal game end.
 */
function isLegitimateGameEnd(msg) {
    if (!msg) return false;
    const hasWinner = msg.winner !== null && msg.winner !== undefined;
    const tookActions = typeof msg.action_count === 'number' && msg.action_count > 0;
    return hasWinner || tookActions;
}

// Singleton instance (created when needed)
let networkClient = null;

/**
 * Get or create the network client singleton
 * @param {Object} wasmModule - WASM module with network exports
 * @returns {MTGNetworkClient}
 */
export function getNetworkClient(wasmModule) {
    if (!networkClient) {
        networkClient = new MTGNetworkClient(wasmModule);
    }
    return networkClient;
}

/**
 * Reset the network client (for new games)
 */
export function resetNetworkClient() {
    if (networkClient) {
        networkClient.disconnect();
        networkClient = null;
    }
}
