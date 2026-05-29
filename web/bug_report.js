/**
 * Shared bug-report dialog (mtg-587 / mtg-596 / mtg-597).
 *
 * This module owns the ENTIRE bug-report UX so both the Web TUI
 * (web/tui_game.html) and the native GUI (web/native_game.html) reuse one
 * implementation instead of duplicating ~250 lines of dialog logic per page
 * (DRY — see CLAUDE.md). It:
 *
 *   - installs a rolling console-log capture buffer (window.getRecentConsoleLogs),
 *   - injects the dialog CSS + DOM on demand,
 *   - implements the WS-connection precheck (mtg-596): a persistent
 *     "not connected" banner + disabled Submit shown up front on dialog open,
 *   - assembles + sends the `bug_report` WS message and renders the issue
 *     hyperlink returned by the server (mtg-587).
 *
 * Pages call `initBugReport(config)` with hooks for the bits that differ:
 *   config = {
 *     triggerButtonId: string,         // existing button that opens the dialog
 *     getNetworkClient: () => client,  // current MTGNetworkClient (or null)
 *     getGameLogs: () => string[],     // current game-log lines
 *     getConsoleLogs?: () => string[], // defaults to window.getRecentConsoleLogs
 *     getTurnInfo?: () => string,
 *     getMode?: () => string,
 *   }
 *
 * The network client is expected to expose `isConnected()`, `send(json)`, and a
 * settable `onBugReportResult` callback (see web/network.js).
 */

const MAX_CONSOLE_LINES = 500;

// ---- Rolling console capture (idempotent across pages/modules) --------------
export function installConsoleCapture() {
    if (window.__bugReportConsoleCaptureInitialized) {
        return;
    }
    window.__bugReportConsoleCaptureInitialized = true;

    const originalConsole = {
        log: console.log.bind(console),
        warn: console.warn.bind(console),
        error: console.error.bind(console),
    };
    const consoleBuffer = [];

    function stringifyConsoleArg(arg) {
        if (arg instanceof Error) {
            return arg.stack || `${arg.name}: ${arg.message}`;
        }
        if (typeof arg === 'string') {
            return arg;
        }
        if (typeof arg === 'undefined') {
            return 'undefined';
        }
        try {
            return JSON.stringify(arg);
        } catch (err) {
            return String(arg);
        }
    }

    function recordConsoleLine(level, args) {
        const rendered = args.map(stringifyConsoleArg).join(' ');
        consoleBuffer.push(`[${new Date().toISOString()}] [${level.toUpperCase()}] ${rendered}`);
        if (consoleBuffer.length > MAX_CONSOLE_LINES) {
            consoleBuffer.splice(0, consoleBuffer.length - MAX_CONSOLE_LINES);
        }
    }

    ['log', 'warn', 'error'].forEach((level) => {
        console[level] = (...args) => {
            recordConsoleLine(level, args);
            originalConsole[level](...args);
        };
    });

    window.getRecentConsoleLogs = function() {
        return consoleBuffer.slice();
    };
    window.clearRecentConsoleLogs = function() {
        consoleBuffer.length = 0;
    };
    window.__originalConsole = originalConsole;
}

// ---- Dialog CSS + DOM injection ---------------------------------------------
const BUG_REPORT_CSS = `
        #bug-report-overlay {
            display: none;
            position: fixed;
            inset: 0;
            background: rgba(0, 0, 0, 0.78);
            z-index: 1099;
        }
        #bug-report-overlay.show { display: block; }
        #bug-report-modal {
            display: none;
            position: fixed;
            top: 50%;
            left: 50%;
            transform: translate(-50%, -50%);
            width: min(680px, calc(100vw - 32px));
            max-height: min(720px, calc(100vh - 32px));
            overflow-y: auto;
            background: linear-gradient(180deg, rgba(15, 26, 51, 0.98) 0%, rgba(10, 17, 33, 0.98) 100%);
            border: 1px solid #4361ee;
            border-radius: 12px;
            box-shadow: 0 22px 60px rgba(0, 0, 0, 0.7);
            padding: 20px;
            z-index: 1100;
        }
        #bug-report-modal.show { display: block; }
        .bug-report-title { color: #ffd700; font-size: 20px; font-weight: 700; margin-bottom: 8px; }
        .bug-report-copy { color: #9fb3d1; font-size: 13px; line-height: 1.5; margin-bottom: 16px; }
        .bug-report-field { margin-bottom: 14px; }
        .bug-report-label {
            display: block; color: #4cc9f0; font-size: 12px; margin-bottom: 6px;
            text-transform: uppercase; letter-spacing: 0.05em;
        }
        .bug-report-input {
            width: 100%; background: rgba(6, 11, 23, 0.96); color: #f1f5ff;
            border: 1px solid #29447c; border-radius: 8px; padding: 12px 14px; font-size: 14px;
        }
        .bug-report-input:focus {
            outline: none; border-color: #4cc9f0; box-shadow: 0 0 0 2px rgba(76, 201, 240, 0.18);
        }
        #bug-report-description {
            min-height: 170px; resize: vertical;
            font-family: 'Consolas', 'Monaco', 'Courier New', monospace; line-height: 1.45;
        }
        .bug-report-meta {
            display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
            gap: 10px; margin: 14px 0 10px;
        }
        .bug-report-meta-card {
            background: rgba(18, 31, 58, 0.85); border: 1px solid rgba(67, 97, 238, 0.45);
            border-radius: 8px; padding: 10px 12px;
        }
        .bug-report-meta-card strong {
            display: block; color: #ffd700; font-size: 11px; margin-bottom: 4px;
            text-transform: uppercase; letter-spacing: 0.05em;
        }
        .bug-report-meta-card span { color: #dfe7f5; font-size: 13px; }
        #bug-report-status { min-height: 20px; color: #9fb3d1; font-size: 13px; margin: 8px 0 14px; }
        #bug-report-status.error {
            background: none; margin: 8px 0 14px; padding: 0; text-align: left; color: #ff8f8f;
        }
        #bug-report-status a { color: #4cc9f0; text-decoration: underline; }
        #bug-report-actions { display: flex; justify-content: flex-end; gap: 10px; flex-wrap: wrap; }
        .bug-report-button {
            border: none; border-radius: 8px; padding: 10px 16px; cursor: pointer;
            font-size: 14px; font-weight: 600;
        }
        #btn-bug-report-cancel { background: #2b3550; color: #dfe7f5; }
        #btn-bug-report-submit {
            background: linear-gradient(135deg, #4361ee 0%, #2a9d8f 100%);
            color: white; min-width: 160px;
        }
        #btn-bug-report-submit:disabled,
        #btn-bug-report-cancel:disabled { opacity: 0.65; cursor: wait; }
`;

const BUG_REPORT_HTML = `
    <div id="bug-report-overlay"></div>
    <div id="bug-report-modal" role="dialog" aria-modal="true" aria-labelledby="bug-report-title">
        <div class="bug-report-title" id="bug-report-title">Report Bug</div>
        <div class="bug-report-copy">
            Capture what happened in the current game. When a remote game WebSocket is connected, the form sends recent browser console output and current game logs to the server with your report.
        </div>
        <div class="bug-report-field">
            <label class="bug-report-label" for="bug-report-description">Describe the expected behavior and the deviant behavior</label>
            <textarea id="bug-report-description" class="bug-report-input" placeholder="Describe the expected behavior and the deviant behavior"></textarea>
        </div>
        <div class="bug-report-field">
            <label class="bug-report-label" for="bug-report-password">Trusted bug-report password</label>
            <input id="bug-report-password" class="bug-report-input" type="password" placeholder="Optional">
        </div>
        <div class="bug-report-meta">
            <div class="bug-report-meta-card">
                <strong>Console Capture</strong>
                <span id="bug-report-console-count">0 buffered lines</span>
            </div>
            <div class="bug-report-meta-card">
                <strong>Game Log Capture</strong>
                <span id="bug-report-game-log-count">0 buffered lines</span>
            </div>
        </div>
        <div id="bug-report-status"></div>
        <div id="bug-report-actions">
            <button id="btn-bug-report-cancel" class="bug-report-button" type="button">Cancel</button>
            <button id="btn-bug-report-submit" class="bug-report-button" type="button">Submit</button>
        </div>
    </div>
`;

function injectDialogChrome() {
    if (!document.getElementById('bug-report-styles')) {
        const style = document.createElement('style');
        style.id = 'bug-report-styles';
        style.textContent = BUG_REPORT_CSS;
        document.head.appendChild(style);
    }
    // Only inject the DOM if a page hasn't already provided it inline.
    if (!document.getElementById('bug-report-modal')) {
        const container = document.createElement('div');
        container.innerHTML = BUG_REPORT_HTML;
        document.body.appendChild(container);
    }
}

// ---- Dialog controller -------------------------------------------------------
export function initBugReport(config) {
    const {
        triggerButtonId,
        getNetworkClient,
        getGameLogs,
        getConsoleLogs = () => (typeof window.getRecentConsoleLogs === 'function' ? window.getRecentConsoleLogs() : []),
        getTurnInfo = () => document.getElementById('turn-info')?.textContent || '',
        getMode = () => document.getElementById('game-mode')?.value || 'unknown',
    } = config;

    installConsoleCapture();
    injectDialogChrome();

    let bugReportSubmitting = false;
    let connectionPollHandle = null;
    // The currently-wired network client. `wireNetworkClient` updates this (used
    // by tests that inject a mock transport, and by pages whose client variable
    // the module cannot reassign). Falls back to the page-provided getter.
    let activeClient = null;

    function currentClient() {
        return activeClient || (typeof getNetworkClient === 'function' ? getNetworkClient() : null);
    }

    function getReportConsoleLogs() {
        return getConsoleLogs() || [];
    }
    function getReportGameLogs() {
        return getGameLogs() || [];
    }

    function refreshCaptureCounts() {
        const consoleCount = document.getElementById('bug-report-console-count');
        const gameLogCount = document.getElementById('bug-report-game-log-count');
        if (consoleCount) {
            consoleCount.textContent = `${getReportConsoleLogs().length} buffered lines`;
        }
        if (gameLogCount) {
            gameLogCount.textContent = `${getReportGameLogs().length} buffered lines`;
        }
    }

    function setStatus(message = '', { isError = false, issueUrl = null } = {}) {
        const status = document.getElementById('bug-report-status');
        if (!status) {
            return;
        }
        // Any explicit status write clears the precheck-banner marker so the
        // connection poll won't later wipe a legit message (mtg-596).
        delete status.dataset.precheckBanner;
        status.innerHTML = '';
        status.classList.toggle('error', isError);
        if (!message && !issueUrl) {
            return;
        }
        if (message) {
            status.appendChild(document.createTextNode(message));
        }
        if (issueUrl) {
            if (message) {
                status.appendChild(document.createTextNode(' '));
            }
            const link = document.createElement('a');
            link.href = issueUrl;
            link.target = '_blank';
            link.rel = 'noopener noreferrer';
            link.textContent = issueUrl;
            status.appendChild(link);
        }
    }

    function setSubmitting(isSubmitting) {
        bugReportSubmitting = isSubmitting;
        const submitButton = document.getElementById('btn-bug-report-submit');
        const cancelButton = document.getElementById('btn-bug-report-cancel');
        if (submitButton) {
            submitButton.disabled = isSubmitting;
            submitButton.textContent = isSubmitting ? 'Preparing Report...' : 'Submit';
        }
        if (cancelButton) {
            cancelButton.disabled = isSubmitting;
        }
    }

    // mtg-596: bug reports are filed by the native server over the active
    // WebSocket; there's no point letting the user write one with no connection.
    function isConnectionReady() {
        const client = currentClient();
        return !!(client
            && typeof client.isConnected === 'function'
            && client.isConnected()
            && typeof client.send === 'function');
    }

    // mtg-596: reflect the live WS state on the open dialog — disable Submit and
    // show a persistent inline banner up front when there is no connection.
    function applyConnectionState() {
        if (bugReportSubmitting) {
            return;
        }
        const submitButton = document.getElementById('btn-bug-report-submit');
        const connected = isConnectionReady();
        if (submitButton) {
            submitButton.disabled = !connected;
        }
        const status = document.getElementById('bug-report-status');
        if (!connected) {
            setStatus(
                'Not connected — bug reports need an active server connection. Start or join a network game to file a report.',
                { isError: true }
            );
            if (status) {
                status.dataset.precheckBanner = 'true';
            }
        } else if (status && status.dataset.precheckBanner === 'true') {
            delete status.dataset.precheckBanner;
            setStatus('');
        }
    }

    function startConnectionPolling() {
        stopConnectionPolling();
        connectionPollHandle = setInterval(applyConnectionState, 750);
    }
    function stopConnectionPolling() {
        if (connectionPollHandle) {
            clearInterval(connectionPollHandle);
            connectionPollHandle = null;
        }
    }

    function openModal() {
        refreshCaptureCounts();
        setStatus('');
        setSubmitting(false);
        document.getElementById('bug-report-overlay').classList.add('show');
        document.getElementById('bug-report-modal').classList.add('show');
        applyConnectionState();
        startConnectionPolling();
        document.getElementById('bug-report-description').focus();
    }

    function clearFormState() {
        document.getElementById('bug-report-description').value = '';
        document.getElementById('bug-report-password').value = '';
        setStatus('');
    }

    function closeModal({ resetStatus = true } = {}) {
        if (bugReportSubmitting) {
            return;
        }
        stopConnectionPolling();
        document.getElementById('bug-report-overlay').classList.remove('show');
        document.getElementById('bug-report-modal').classList.remove('show');
        if (resetStatus) {
            clearFormState();
        }
    }

    async function submitDraft() {
        const descriptionEl = document.getElementById('bug-report-description');
        const passwordEl = document.getElementById('bug-report-password');
        const description = descriptionEl.value.trim();
        if (!description) {
            setStatus('Enter a bug description before submitting.', { isError: true });
            descriptionEl.focus();
            return;
        }

        setSubmitting(true);
        setStatus('Capturing current logs and assembling bug report payload...');

        await new Promise((resolve) => setTimeout(resolve, 150));

        const payload = {
            description,
            trustedPassword: passwordEl.value.trim(),
            capturedAt: new Date().toISOString(),
            turnInfo: getTurnInfo(),
            mode: getMode(),
            consoleLogs: getReportConsoleLogs(),
            gameLogs: getReportGameLogs(),
        };

        const message = {
            type: 'bug_report',
            description: payload.description,
            game_logs: payload.gameLogs.join('\n'),
            console_logs: payload.consoleLogs.join('\n'),
            trusted_password: payload.trustedPassword || undefined,
        };

        window.lastBugReportDraft = payload;
        window.lastBugReportSubmissionMessage = message;
        refreshCaptureCounts();
        console.log('[BugReport] Captured draft payload', {
            descriptionLength: payload.description.length,
            trustedPasswordProvided: Boolean(payload.trustedPassword),
            consoleLogCount: payload.consoleLogs.length,
            gameLogCount: payload.gameLogs.length,
            turnInfo: payload.turnInfo,
        });

        const client = currentClient();
        if (!isConnectionReady()) {
            setSubmitting(false);
            setStatus('Bug report submission requires an active network WebSocket connection.', { isError: true });
            return;
        }

        try {
            setStatus('Submitting bug report to server...');
            client.send(JSON.stringify(message));
        } catch (error) {
            setSubmitting(false);
            setStatus(`Failed to send bug report: ${error.message}`, { isError: true });
        }
    }

    function handleResult(result) {
        window.lastBugReportResult = result;
        setSubmitting(false);

        if (!result?.success) {
            setStatus(result?.error || 'Bug report submission failed.', { isError: true });
            return;
        }

        document.getElementById('bug-report-description').value = '';
        document.getElementById('bug-report-password').value = '';
        refreshCaptureCounts();

        if (result.issue_url) {
            setStatus('Bug report filed:', { issueUrl: result.issue_url });
        } else {
            setStatus('Bug report saved locally');
        }
    }

    // Wire the network client's result callback (re-wire whenever the page
    // hands us a fresh client instance).
    function wireNetworkClient(client) {
        activeClient = client || null;
        if (client) {
            client.onBugReportResult = handleResult;
        }
        return client;
    }

    // Wire buttons.
    const trigger = triggerButtonId ? document.getElementById(triggerButtonId) : null;
    if (trigger) {
        trigger.addEventListener('click', openModal);
    }
    document.getElementById('btn-bug-report-cancel').addEventListener('click', () => closeModal());
    document.getElementById('btn-bug-report-submit').addEventListener('click', () => { submitDraft(); });
    document.getElementById('bug-report-overlay').addEventListener('click', () => closeModal());

    refreshCaptureCounts();

    // Test hook surface, matching the previous window.__bugReportTestHelpers API.
    const api = {
        openModal,
        closeModal,
        submitDraft,
        handleResult,
        wireNetworkClient,
        refreshCaptureCounts,
        applyConnectionState,
        setStatus,
        setSubmitting,
    };
    window.__bugReportTestHelpers = {
        setNetworkClient(client) {
            const wired = wireNetworkClient(client);
            window.__bugReportTestTransport = wired;
            return wired;
        },
        handleResult(result) {
            handleResult(result);
        },
    };
    return api;
}
