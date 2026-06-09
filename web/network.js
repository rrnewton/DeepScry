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

            // Start polling for outbound messages
            this._startOutboundPoll();

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

            // A NON-clean close is a real disconnect ("connection lost"): the
            // user must see it, not just the console (mtg web-ui-fixes fix #4).
            // A clean close (code 1000, client-initiated disconnect) is normal
            // and stays quiet.
            if (!event.wasClean) {
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
            // give an actionable one.
            const msg = 'Network error connecting to the game server.';
            console.error('[Network] WebSocket error:', event);
            this.wasm.network_on_error(msg);
            if (this.onError) this.onError(msg);
        };
    }

    /**
     * Disconnect from server
     */
    disconnect() {
        this.reconnecting = false;
        this._stopOutboundPoll();

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

    _scheduleReconnect() {
        if (this.reconnecting) return;

        this.reconnecting = true;
        this._log('[Network] Scheduling reconnect in 3 seconds...');

        setTimeout(() => {
            if (this.reconnecting && this.serverUrl) {
                this._log('[Network] Attempting reconnect...');
                // Re-fetch connection params from WASM or stored values
                // For now, just notify error - user should manually reconnect
                if (this.onError) {
                    this.onError('Connection lost. Please reconnect.');
                }
                this.reconnecting = false;
            }
        }, 3000);
    }
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
