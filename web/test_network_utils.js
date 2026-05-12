// Shared utilities for WASM network E2E tests
//
// Extracted from test_network_human_input.js for reuse across tests.

const WebSocket = require('ws');
const net = require('net');

// Timestamped logging
function log(message) {
    const timestamp = new Date().toISOString().substring(11, 23);
    console.log(`[${timestamp}] ${message}`);
}

// Check if a TCP port is available by attempting to bind to it, then releasing.
// Returns a promise that resolves to true if available, false if in use.
function isPortAvailable(port) {
    return new Promise((resolve) => {
        const srv = net.createServer();
        srv.once('error', () => resolve(false));
        srv.once('listening', () => {
            srv.close(() => resolve(true));
        });
        try {
            srv.listen(port, '127.0.0.1');
        } catch (_) {
            resolve(false);
        }
    });
}

// Allocate a pair of random available ports (one for the game server WebSocket,
// one for the HTTP static file server). Picks from range 10000-60000 to avoid
// collisions with well-known ports and other test processes.
// Returns { serverPort, httpPort }.
async function getRandomPorts() {
    const MIN_PORT = 10000;
    const MAX_PORT = 60000;
    const range = MAX_PORT - MIN_PORT;
    const MAX_ATTEMPTS = 20;

    async function findAvailablePort() {
        for (let i = 0; i < MAX_ATTEMPTS; i++) {
            const port = MIN_PORT + Math.floor(Math.random() * range);
            if (await isPortAvailable(port)) {
                return port;
            }
        }
        throw new Error(`Failed to find an available port after ${MAX_ATTEMPTS} attempts`);
    }

    const serverPort = await findAvailablePort();
    // Ensure httpPort is different from serverPort
    let httpPort;
    for (let i = 0; i < MAX_ATTEMPTS; i++) {
        httpPort = MIN_PORT + Math.floor(Math.random() * range);
        if (httpPort !== serverPort && await isPortAvailable(httpPort)) {
            return { serverPort, httpPort };
        }
    }
    throw new Error('Failed to find two distinct available ports');
}

// Wait for server to be ready by attempting WebSocket connection
async function waitForServer(port, maxAttempts = 30) {
    for (let i = 0; i < maxAttempts; i++) {
        try {
            const ws = new WebSocket(`ws://localhost:${port}`);
            await new Promise((resolve, reject) => {
                ws.on('open', () => { ws.close(); resolve(); });
                ws.on('error', reject);
                setTimeout(() => reject(new Error('timeout')), 1000);
            });
            return true;
        } catch (e) {
            await new Promise(r => setTimeout(r, 500));
        }
    }
    return false;
}

// Extract all text from the RatZilla terminal DOM
async function extractTerminalText(page) {
    return await page.evaluate(() => {
        const terminal = document.getElementById('ratzilla-terminal');
        if (!terminal) return 'NO TERMINAL';
        const rows = [];
        const rowElements = terminal.querySelectorAll('div');
        for (const row of rowElements) {
            const text = row.textContent || '';
            if (text.trim()) rows.push(text);
        }
        return rows.join('\n');
    });
}

// Check for fatal errors in browser console logs
// Per NETWORK_ARCHITECTURE.md: desync is ALWAYS fatal.
// REWIND/REPLAY FATAL is surfaced by the rewind/replay verifier in
// fancy_tui.rs (replay_verifier::ReplayCheckOutcome::fatal_message). When
// `enableReplayVerifier(page)` has been called for this test, ANY occurrence
// of that string means the post-replay state diverged from the pre-rewind
// snapshot — same severity as a network desync and absolutely a test failure.
function checkForFatalErrors(browserLogs) {
    const fatalPatterns = [
        'MONOTONICITY VIOLATION',
        'FATAL DESYNC',
        'DESYNC',
        'REWIND/REPLAY FATAL',
        'unreachable',
        'panic',
    ];
    for (const entry of browserLogs) {
        for (const pattern of fatalPatterns) {
            if (entry.text.toUpperCase().includes(pattern.toUpperCase())) {
                return entry.text;
            }
        }
    }
    return null;
}

// Enable the WASM rewind/replay verifier on the given Playwright page.
//
// The verifier is implemented in mtg-engine/src/wasm/replay_verifier.rs and
// wired into fancy_tui.rs via the wasm-bindgen export
// `tui_set_verify_rewind_replay(bool)`. When enabled, every rewind to a turn
// boundary captures a snapshot (state hash, action count, log tail) and the
// post-replay state is compared against it. Any divergence is logged at
// ERROR level with a "REWIND/REPLAY FATAL" prefix — which `checkForFatalErrors`
// above treats as a hard test failure.
//
// Call this AFTER `#launcher.show` is visible (i.e. WASM has finished
// initialising) but BEFORE the game launch button is clicked. The Rust side
// gracefully handles being called before the TUI state is initialised — it
// stashes the flag and applies it at next launch — but calling post-WASM-init
// avoids the warning log.
//
// Returns true if the export was available and called, false if the WASM
// build does not include it (e.g. very old build). Tests should treat false
// as a soft warning, NOT a failure: the verifier is a debug aid, not a
// runtime requirement.
async function enableReplayVerifier(page) {
    return await page.evaluate(() => {
        if (typeof tui_set_verify_rewind_replay === 'function') {
            tui_set_verify_rewind_replay(true);
            console.log('[Test] tui_set_verify_rewind_replay(true) called');
            return true;
        }
        console.log('[Test] tui_set_verify_rewind_replay not exported by this WASM build');
        return false;
    });
}

// Classify what kind of choice prompt is on screen
// Returns: { type, text, numChoices } or null if no prompt
function classifyPrompt(terminalText) {
    if (!terminalText) return null;

    // Count how many choices are visible (look for [N] patterns)
    const choiceMatches = terminalText.match(/\[\d+\]/g);
    const numChoices = choiceMatches ? choiceMatches.length : 0;

    // Check for game over first
    if (terminalText.includes('Game Over') || terminalText.includes('wins')) {
        return { type: 'game_over', text: terminalText, numChoices };
    }

    // Check for various prompt types
    if (terminalText.includes('Declare Attackers')) {
        return { type: 'attackers', text: terminalText, numChoices };
    }
    if (terminalText.includes('Declare Blockers')) {
        return { type: 'blockers', text: terminalText, numChoices };
    }
    if (terminalText.includes('Discard') && terminalText.includes('card(s)')) {
        return { type: 'discard', text: terminalText, numChoices };
    }
    if (terminalText.includes('Choose target for')) {
        return { type: 'targets', text: terminalText, numChoices };
    }
    if (terminalText.includes('Choose mana source')) {
        return { type: 'mana_source', text: terminalText, numChoices };
    }
    if (terminalText.includes('Search library')) {
        return { type: 'library_search', text: terminalText, numChoices };
    }
    if (terminalText.includes('Choose damage order')) {
        return { type: 'damage_order', text: terminalText, numChoices };
    }
    if (terminalText.includes('sacrifice')) {
        return { type: 'sacrifice', text: terminalText, numChoices };
    }
    if (terminalText.includes('Choose') && terminalText.includes('mode')) {
        return { type: 'modes', text: terminalText, numChoices };
    }
    if (terminalText.includes('Priority') && terminalText.includes('Choose action')) {
        return { type: 'spell_ability', text: terminalText, numChoices };
    }
    // Fallback: any visible choice list
    if (terminalText.match(/\[\d+\]/)) {
        if (terminalText.match(/\[0\] pass/)) {
            return { type: 'spell_ability', text: terminalText, numChoices };
        }
        return { type: 'unknown_choice', text: terminalText, numChoices };
    }
    return null;
}

// Decide which key to press based on prompt type and available choices
// Returns { key, reason } where key is the choice index as string
function decideKey(prompt) {
    const text = prompt.text;

    switch (prompt.type) {
        case 'spell_ability': {
            const landMatch = text.match(/\[(\d+)\] play /);
            if (landMatch) return { key: landMatch[1], reason: 'play land' };
            const castMatch = text.match(/\[(\d+)\] cast /);
            if (castMatch) return { key: castMatch[1], reason: 'cast spell' };
            return { key: '0', reason: 'pass priority' };
        }
        case 'attackers': {
            const creatureMatch = text.match(/\[(\d+)\] (?!Done)/);
            if (creatureMatch) return { key: creatureMatch[1], reason: 'attack with creature' };
            return { key: '0', reason: 'done (no attackers)' };
        }
        case 'blockers': {
            const blockMatch = text.match(/\[(\d+)\] (?!Done)/);
            if (blockMatch) return { key: blockMatch[1], reason: 'block with creature' };
            return { key: '0', reason: 'done (no blockers)' };
        }
        case 'discard':
            return { key: '0', reason: 'discard first card' };
        case 'targets': {
            const targetMatch = text.match(/\[(\d+)\] (?!No target)/);
            if (targetMatch) return { key: targetMatch[1], reason: 'select target' };
            return { key: '0', reason: 'no target' };
        }
        case 'mana_source':
            return { key: '0', reason: 'first mana source' };
        case 'library_search': {
            const cardMatch = text.match(/\[(\d+)\] (?!Fail to find)/);
            if (cardMatch) return { key: cardMatch[1], reason: 'search: select card' };
            return { key: '0', reason: 'fail to find' };
        }
        case 'damage_order':
            return { key: '0', reason: 'first damage order' };
        case 'sacrifice': {
            const sacMatch = text.match(/\[(\d+)\] (?!Done)/);
            if (sacMatch) return { key: sacMatch[1], reason: 'sacrifice permanent' };
            return { key: '0', reason: 'done (sacrifice)' };
        }
        case 'modes':
            return { key: '0', reason: 'first mode' };
        default:
            return { key: '0', reason: 'unknown prompt (first option)' };
    }
}

// Submit a choice via keyboard. Uses multi-digit input when needed.
// For <=10 choices: single keypress (0-based index)
// For >10 choices: type digits + Enter
async function submitChoice(page, key, numChoices) {
    if (numChoices > 10) {
        // Multi-digit mode: type the number then press Enter
        for (const ch of key) {
            await page.keyboard.press(ch);
            await page.waitForTimeout(50);
        }
        await page.keyboard.press('Enter');
    } else {
        // Single-digit mode: just press the key
        await page.keyboard.press(key);
    }
}

// Wait for a choice prompt to appear in the terminal
// If previousText is provided, waits for the terminal text to CHANGE first
async function waitForChoicePrompt(page, timeout = 20000, previousText = null) {
    const startTime = Date.now();
    let textChanged = (previousText === null);

    while (Date.now() - startTime < timeout) {
        const text = await extractTerminalText(page);

        if (!textChanged) {
            if (text !== previousText) {
                textChanged = true;
                await page.waitForTimeout(200);
                continue;
            }
            await page.waitForTimeout(100);
            continue;
        }

        const prompt = classifyPrompt(text);
        if (prompt) return prompt;
        await page.waitForTimeout(200);
    }
    return null;
}

module.exports = {
    log,
    isPortAvailable,
    getRandomPorts,
    waitForServer,
    extractTerminalText,
    checkForFatalErrors,
    enableReplayVerifier,
    classifyPrompt,
    decideKey,
    submitChoice,
    waitForChoicePrompt,
};
