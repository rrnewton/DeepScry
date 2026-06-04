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
 * The network client is expected to expose `isConnected()`, `send(json)`, and
 * the two settable two-phase callbacks `onBugReportStored` (phase 1: disk-write
 * confirmation) and `onBugReportIssueResult` (phase 2: GitHub issue outcome) —
 * see web/network.js and mtg-5ejgo.
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
        #bug-report-progress { display: flex; flex-direction: column; gap: 8px; }
        .bug-report-check-row {
            display: flex; align-items: baseline; gap: 8px;
            color: #dfe7f5; font-size: 13px; line-height: 1.4; flex-wrap: wrap;
        }
        .bug-report-check-icon { font-size: 15px; min-width: 18px; }
        .bug-report-check-row[data-state="pending"] { color: #9fb3d1; }
        .bug-report-check-row[data-state="ok"] .bug-report-check-icon { color: #4ade80; }
        .bug-report-check-row[data-state="fail"] { color: #ff8f8f; }
        .bug-report-check-row[data-state="unknown"] { color: #f4c453; }
        .bug-report-check-detail { width: 100%; color: #9fb3d1; font-size: 12px; margin-left: 26px; }
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
        <details id="bug-report-advanced" style="margin-bottom: 14px;">
            <summary style="cursor: pointer; color: var(--muted, #9fb3d1); font-size: 12px; user-select: none; padding: 4px 0;">
                Advanced options
            </summary>
            <div class="bug-report-field" style="margin-top: 10px;">
                <label class="bug-report-label" for="bug-report-password">Trusted bug-report password</label>
                <input id="bug-report-password" class="bug-report-input" type="password" placeholder="Leave blank unless you have a trusted reporter key">
            </div>
        </details>
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

    // Two-phase submission state (mtg-5ejgo). The widget shows two checkboxes —
    // "saved to disk" (phase 1) and "filed on GitHub" (phase 2) — and is
    // "finalized" once the flow reaches a terminal state, after which Submit is
    // permanently disabled ("Already submitted") so the user cannot double-file.
    let bugReportFinalized = false;
    let backstopHandle = null;
    // Each phase is one of: 'pending' | 'ok' | 'fail' | 'unknown'.
    let progressState = { disk: 'pending', diskError: null, github: 'pending', githubUrl: null, githubError: null };

    // Client-side timeout backstop: if the server's phase-2 message (or any
    // message) never arrives, the client resolves the pending box(es) to an
    // "unknown — report saved" state and finalizes the button, so a dropped or
    // late response can NEVER leave a forever-spinner. Overridable for tests.
    const DEFAULT_BACKSTOP_MS = 18000;
    function backstopMs() {
        const override = Number(window.__bugReportBackstopMs);
        return Number.isFinite(override) && override > 0 ? override : DEFAULT_BACKSTOP_MS;
    }

    function currentClient() {
        return activeClient || (typeof getNetworkClient === 'function' ? getNetworkClient() : null);
    }

    // The page-provided log getters may reach into the WASM client (e.g.
    // tui_get_logs_json). During wasm/network e2e startup the dialog chrome is
    // injected and refreshCaptureCounts() runs BEFORE the WASM module is
    // initialized, so calling those getters throws
    // "Cannot read properties of undefined (reading '__wbindgen_free')". Guard
    // both getters in ONE place (DRY) so a not-yet-ready client degrades to an
    // empty capture rather than crashing the whole page (mtg-587).
    function safeLogs(getter) {
        try {
            return getter() || [];
        } catch (_error) {
            return [];
        }
    }
    function getReportConsoleLogs() {
        return safeLogs(getConsoleLogs);
    }
    function getReportGameLogs() {
        return safeLogs(getGameLogs);
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
        // While a submission is in flight (or finalized) the connection poll must
        // not touch the Submit button or status area — the progress checkboxes
        // and finalize logic own them (mtg-5ejgo).
        if (bugReportSubmitting || bugReportFinalized) {
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
        // Every open starts a fresh report: clear any finalized/progress state
        // from a previous submission so the two checkboxes reset (mtg-5ejgo).
        resetSubmissionState();
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
        // A submission in flight (phase 1 sent, awaiting phase 2 or the backstop)
        // keeps the modal open so the user sees the outcome. Once finalized the
        // modal is freely closable (mtg-5ejgo).
        if (bugReportSubmitting && !bugReportFinalized) {
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
            client.send(JSON.stringify(message));
            // Two-phase UX (mtg-5ejgo): the report is now in flight. Show the two
            // checkboxes and arm the client-side backstop. The Submit button stays
            // disabled until the flow finalizes.
            startSubmissionProgress();
        } catch (error) {
            setSubmitting(false);
            setStatus(`Failed to send bug report: ${error.message}`, { isError: true });
        }
    }

    // ---- Two-phase progress state machine (mtg-5ejgo) ------------------------

    const CHECK_GLYPHS = { pending: '☐', ok: '☑', fail: '✗', unknown: '⚠' };

    function clearBackstop() {
        if (backstopHandle) {
            clearTimeout(backstopHandle);
            backstopHandle = null;
        }
    }

    function resetSubmissionState() {
        clearBackstop();
        bugReportFinalized = false;
        bugReportSubmitting = false;
        progressState = { disk: 'pending', diskError: null, github: 'pending', githubUrl: null, githubError: null };
        const submitButton = document.getElementById('btn-bug-report-submit');
        const cancelButton = document.getElementById('btn-bug-report-cancel');
        if (submitButton) {
            submitButton.textContent = 'Submit';
        }
        if (cancelButton) {
            cancelButton.textContent = 'Cancel';
            cancelButton.disabled = false;
        }
    }

    function buildCheckRow(id, state, label) {
        const row = document.createElement('div');
        row.id = id;
        row.className = 'bug-report-check-row';
        row.dataset.state = state;
        const icon = document.createElement('span');
        icon.className = 'bug-report-check-icon';
        icon.textContent = CHECK_GLYPHS[state] || CHECK_GLYPHS.pending;
        row.appendChild(icon);
        row.appendChild(document.createTextNode(` ${label}`));
        return row;
    }

    function renderProgress() {
        const status = document.getElementById('bug-report-status');
        if (!status) {
            return;
        }
        delete status.dataset.precheckBanner;
        status.classList.remove('error');
        status.innerHTML = '';

        const container = document.createElement('div');
        container.id = 'bug-report-progress';

        const diskLabels = {
            pending: 'Saving bug report to disk…',
            ok: 'Bug report saved to disk',
            fail: `Save to disk failed: ${progressState.diskError || 'unknown error'}`,
            unknown: 'Save-to-disk status unknown',
        };
        container.appendChild(buildCheckRow('bug-report-check-disk', progressState.disk, diskLabels[progressState.disk]));

        const githubLabels = {
            pending: 'Filing bug report on GitHub…',
            ok: 'Bug report filed on GitHub:',
            fail: 'GitHub filing failed — report saved',
            unknown: 'GitHub filing status unknown — report saved',
        };
        const githubRow = buildCheckRow('bug-report-check-github', progressState.github, githubLabels[progressState.github]);
        if (progressState.github === 'ok' && progressState.githubUrl) {
            githubRow.appendChild(document.createTextNode(' '));
            const link = document.createElement('a');
            link.href = progressState.githubUrl;
            link.target = '_blank';
            link.rel = 'noopener noreferrer';
            link.textContent = progressState.githubUrl;
            githubRow.appendChild(link);
        }
        if (progressState.github === 'fail' && progressState.githubError) {
            const detail = document.createElement('div');
            detail.className = 'bug-report-check-detail';
            detail.textContent = progressState.githubError;
            githubRow.appendChild(detail);
        }
        container.appendChild(githubRow);
        status.appendChild(container);
    }

    function startSubmissionProgress() {
        bugReportSubmitting = true;
        bugReportFinalized = false;
        progressState = { disk: 'pending', diskError: null, github: 'pending', githubUrl: null, githubError: null };
        const submitButton = document.getElementById('btn-bug-report-submit');
        const cancelButton = document.getElementById('btn-bug-report-cancel');
        if (submitButton) {
            submitButton.disabled = true;
            submitButton.textContent = 'Submitting…';
        }
        // Allow the user to close the window while waiting; the backstop still
        // resolves the boxes in the background.
        if (cancelButton) {
            cancelButton.disabled = false;
        }
        renderProgress();
        clearBackstop();
        backstopHandle = setTimeout(() => {
            if (bugReportFinalized) {
                return;
            }
            if (progressState.disk === 'pending') {
                progressState.disk = 'unknown';
            }
            if (progressState.github === 'pending') {
                progressState.github = 'unknown';
            }
            renderProgress();
            finalizeSubmission();
        }, backstopMs());
    }

    function restoreEditableControls() {
        bugReportSubmitting = false;
        const submitButton = document.getElementById('btn-bug-report-submit');
        const cancelButton = document.getElementById('btn-bug-report-cancel');
        if (submitButton) {
            submitButton.textContent = 'Submit';
            submitButton.disabled = !isConnectionReady();
        }
        if (cancelButton) {
            cancelButton.disabled = false;
        }
    }

    function finalizeSubmission() {
        clearBackstop();
        bugReportFinalized = true;
        bugReportSubmitting = false;
        stopConnectionPolling();
        const submitButton = document.getElementById('btn-bug-report-submit');
        const cancelButton = document.getElementById('btn-bug-report-cancel');
        if (submitButton) {
            submitButton.disabled = true;
            submitButton.textContent = 'Already submitted';
        }
        if (cancelButton) {
            cancelButton.disabled = false;
            cancelButton.textContent = 'Close';
        }
        // The report is saved (or the user has been told its status); clear the
        // draft fields so a reopen starts fresh.
        const descriptionEl = document.getElementById('bug-report-description');
        const passwordEl = document.getElementById('bug-report-password');
        if (descriptionEl) {
            descriptionEl.value = '';
        }
        if (passwordEl) {
            passwordEl.value = '';
        }
        refreshCaptureCounts();
    }

    // Phase 1: the server confirms (or reports failure of) the disk write.
    function handleStored(message) {
        window.lastBugReportStored = message;
        if (message?.success) {
            progressState.disk = 'ok';
            renderProgress();
            return;
        }
        // Disk write failed — the genuinely-bad case. Show the error on box 1 and
        // let the user retry: stop the backstop and re-enable Submit. We do NOT
        // finalize, because the report was NOT saved.
        progressState.disk = 'fail';
        progressState.diskError = message?.error || 'The server failed to save the bug report.';
        clearBackstop();
        renderProgress();
        restoreEditableControls();
    }

    // Phase 2: the server reports the GitHub issue outcome (link, failure, or
    // timeout). Either way the report is already saved, so we finalize.
    function handleIssueResult(message) {
        window.lastBugReportIssueResult = message;
        if (bugReportFinalized) {
            return;
        }
        if (message?.issue_url) {
            progressState.github = 'ok';
            progressState.githubUrl = message.issue_url;
        } else {
            progressState.github = 'fail';
            progressState.githubError = message?.error || null;
        }
        renderProgress();
        finalizeSubmission();
    }

    // Wire the network client's result callback (re-wire whenever the page
    // hands us a fresh client instance).
    function wireNetworkClient(client) {
        activeClient = client || null;
        if (client) {
            client.onBugReportStored = handleStored;
            client.onBugReportIssueResult = handleIssueResult;
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
        handleStored,
        handleIssueResult,
        wireNetworkClient,
        refreshCaptureCounts,
        applyConnectionState,
        setStatus,
        setSubmitting,
        isSubmitting: () => bugReportSubmitting,
        isFinalized: () => bugReportFinalized,
    };
    window.__bugReportTestHelpers = {
        setNetworkClient(client) {
            const wired = wireNetworkClient(client);
            window.__bugReportTestTransport = wired;
            return wired;
        },
        handleStored(message) {
            handleStored(message);
        },
        handleIssueResult(message) {
            handleIssueResult(message);
        },
    };
    return api;
}
