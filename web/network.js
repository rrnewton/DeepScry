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
    }

    /**
     * Connect to game server
     * @param {string} serverUrl - WebSocket URL (e.g., "ws://localhost:17771")
     * @param {string} password - Server password
     * @param {string} playerName - Player's display name
     * @param {string} deckJson - Deck submission as JSON
     */
    connect(serverUrl, password, playerName, deckJson) {
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            console.warn('[Network] Already connected, disconnecting first');
            this.disconnect();
        }

        this.serverUrl = serverUrl;
        console.log(`[Network] Connecting to ${serverUrl}...`);

        // Initialize WASM network state
        this.wasm.network_init(serverUrl, password, playerName, deckJson);

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
            console.log('[Network] WebSocket connected');
            this.wasm.network_on_open();
            this._notifyStateChange();

            // Start polling for outbound messages
            this._startOutboundPoll();

            // Send any queued messages
            this._flushMessageQueue();
        };

        this.ws.onmessage = (event) => {
            const data = event.data;
            console.log('[Network] Received:', data.substring(0, 200) + (data.length > 200 ? '...' : ''));

            // Pass to WASM for processing
            this.wasm.network_on_message(data);
            this._notifyStateChange();

            // Check if game is now ready
            if (this.wasm.network_is_game_ready() && this.onGameReady) {
                this.onGameReady();
            }
        };

        this.ws.onclose = (event) => {
            console.log(`[Network] WebSocket closed: code=${event.code}, reason=${event.reason}`);
            this.wasm.network_on_close();
            this._notifyStateChange();
            this._stopOutboundPoll();

            // Attempt reconnect if game was in progress
            if (!event.wasClean && !this.reconnecting) {
                this._scheduleReconnect();
            }
        };

        this.ws.onerror = (event) => {
            const msg = 'WebSocket error occurred';
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
            console.log('[Network] Sending:', json.substring(0, 200) + (json.length > 200 ? '...' : ''));
            this.ws.send(json);
        } else {
            console.log('[Network] Queuing message (not connected)');
            this.messageQueue.push(json);
        }
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
            console.log('[Network] Sending queued:', msg.substring(0, 200) + (msg.length > 200 ? '...' : ''));
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
        console.log('[Network] Scheduling reconnect in 3 seconds...');

        setTimeout(() => {
            if (this.reconnecting && this.serverUrl) {
                console.log('[Network] Attempting reconnect...');
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
