// Bug Report E2E Test using Playwright
// Run with: node test_bug_report.js
//
// This test:
// 1. Loads the fancy TUI page
// 2. Launches a local game to expose the floating controls widget
// 3. Verifies the bug report modal UI and validation
// 4. Verifies connect-on-demand (mtg-5ejgo): in a SOLO/local game with no live
//    game WebSocket, Submit is enabled (no "not connected" dead-end) and the
//    widget lazily opens a transient lobby connection to file the report,
//    runs the two-phase flow over it, and closes it.
// 5. Verifies cancel/reset behavior
// 6. Mocks the two-phase bug_report_stored + bug_report_issue_result responses
//    (mtg-5ejgo) and asserts the two-checkbox flow: box 1 checks on the disk
//    confirmation, box 2 checks + links on the issue result, the GitHub-failed
//    case shows a failure (never a spinner) yet still finalizes the button, the
//    client-side backstop finalizes if the second message never arrives, and a
//    disk-write failure surfaces on box 1 and stays retryable.

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

const HTTP_PORT = 8768;

function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

function ensureDir(dir) {
    if (!fs.existsSync(dir)) {
        fs.mkdirSync(dir, { recursive: true });
    }
}

// mtg-35z3s page 3: tui_game.html is a pure renderer with no built-in launcher
// form — the game is booted from URL params before this runs. Here we just wait
// for the controls, expand them, and run a few turns so the bug-report capture
// has real game logs.
async function launchLocalFancyTui(page, testResults) {
    await page.waitForSelector('#game-controls', { state: 'visible', timeout: 10000 });
    await page.waitForTimeout(500);
    await page.click('#btn-toggle-controls');
    await page.waitForSelector('#controls-panel', { state: 'visible', timeout: 5000 });

    // Run a few turns so game logs are populated for bug report testing
    for (let i = 0; i < 3; i++) {
        await page.click('#btn-run-turn');
        await page.waitForTimeout(300);
    }

    testResults.steps.push({ name: 'tui_launch', timestamp: new Date().toISOString() });
}

async function openBugReportModal(page) {
    await page.click('#btn-bug-report');
    await page.waitForSelector('#bug-report-modal', { state: 'visible', timeout: 5000 });
}

async function closeBugReportModal(page) {
    await page.click('#btn-bug-report-cancel');
    await page.waitForSelector('#bug-report-modal', { state: 'hidden', timeout: 5000 });
}

async function readBugReportForm(page) {
    return await page.evaluate(() => ({
        description: document.getElementById('bug-report-description')?.value || '',
        password: document.getElementById('bug-report-password')?.value || '',
        statusText: document.getElementById('bug-report-status')?.textContent || '',
        issueUrl: document.querySelector('#bug-report-status a')?.href || null,
    }));
}

async function runTest() {
    let server = null;
    let browser = null;
    const testResults = {
        startTime: new Date().toISOString(),
        steps: [],
        errors: [],
        browserLogs: []
    };

    const screenshotDir = path.join(__dirname, 'screenshots');
    ensureDir(screenshotDir);

    try {
        const wasmPkgPath = path.join(__dirname, 'pkg', 'mtg_engine.js');
        if (!fs.existsSync(wasmPkgPath)) {
            throw new Error('WASM package not found. Run: make wasm or make wasm-dev');
        }

        log(`Starting HTTP server on port ${HTTP_PORT}...`);
        server = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });
        server.stdout.on('data', data => log(`Server: ${data.toString().trim()}`));
        server.stderr.on('data', data => log(`Server: ${data.toString().trim()}`));
        await new Promise(resolve => setTimeout(resolve, 1000));

        log('Launching Chromium browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        page.on('console', msg => {
            const entry = {
                timestamp: new Date().toISOString(),
                type: msg.type(),
                text: msg.text()
            };
            testResults.browserLogs.push(entry);
            log(`Browser [${msg.type()}]: ${msg.text()}`);
        });

        page.on('pageerror', err => {
            const entry = {
                timestamp: new Date().toISOString(),
                error: err.message
            };
            testResults.errors.push(entry);
            log(`Page ERROR: ${err.message}`);
        });

        // mtg-35z3s page 3: boot a local heuristic-vs-heuristic game directly
        // from URL params (the launcher form was deleted; tui_game.html is now a
        // pure renderer). Same boot path as test_fancy_tui.js.
        log('Loading fancy TUI page (local boot via params)...');
        const base = `http://localhost:${HTTP_PORT}`;
        const firstDeck = await firstBuiltinDeck(base);
        log(`Selected deck for both players: ${firstDeck}`);
        await page.goto(localGameUrl(base, 'tui_game.html', {
            deck: firstDeck, p1: 'heuristic', p2: 'heuristic',
        }), { waitUntil: 'networkidle', timeout: 60000 });

        log('Waiting for WASM to load + game to render...');
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 30000 });
        testResults.steps.push({ name: 'wasm_init', timestamp: new Date().toISOString() });

        await page.screenshot({ path: path.join(screenshotDir, 'bug_report_01_setup.png'), fullPage: true });

        log('Launching local fancy TUI...');
        await launchLocalFancyTui(page, testResults);
        await page.screenshot({ path: path.join(screenshotDir, 'bug_report_02_controls.png'), fullPage: true });

        log('Checking bug report button visibility...');
        await page.waitForSelector('#btn-bug-report', { state: 'visible', timeout: 5000 });
        testResults.steps.push({ name: 'button_visible', timestamp: new Date().toISOString() });

        log('Opening modal and verifying fields...');
        await openBugReportModal(page);
        await page.waitForSelector('#bug-report-description', { state: 'visible', timeout: 5000 });
        // The password field is inside a collapsed <details> (advanced options).
        // Expand the section first so the field is visible for interaction.
        const advDetails = await page.$('#bug-report-advanced');
        if (advDetails) {
            await advDetails.evaluate((el) => { el.open = true; });
        }
        await page.waitForSelector('#bug-report-password', { state: 'visible', timeout: 5000 });
        await page.screenshot({ path: path.join(screenshotDir, 'bug_report_03_modal.png'), fullPage: true });
        testResults.steps.push({ name: 'modal_open', timestamp: new Date().toISOString() });

        // mtg-5ejgo: in a SOLO/local game there is no live game WS, but the
        // widget connects on demand to the lobby endpoint — so Submit must be
        // ENABLED with NO "Not connected — start a network game" banner (that old
        // mtg-596 dead-end is replaced by connect-on-demand).
        log('Checking SOLO game: connect-on-demand enables Submit, no "not connected" banner...');
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            const submit = document.getElementById('btn-bug-report-submit');
            return submit && !submit.disabled && !status.includes('Not connected');
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'solo_connect_on_demand_enabled', timestamp: new Date().toISOString() });

        log('Checking cancel resets fields and reopen keeps Submit enabled (solo)...');
        await page.fill('#bug-report-description', 'This text should be cleared when the modal closes.');
        await page.fill('#bug-report-password', 'reset-me');
        await closeBugReportModal(page);
        await openBugReportModal(page);
        const resetAfterClose = await readBugReportForm(page);
        if (resetAfterClose.description !== '' || resetAfterClose.password !== '') {
            throw new Error('Bug report form fields did not reset after closing the modal');
        }
        if (resetAfterClose.statusText.includes('Not connected')) {
            throw new Error('Reopened solo dialog should NOT show a not-connected banner');
        }
        testResults.steps.push({ name: 'close_reset', timestamp: new Date().toISOString() });

        // ── SOLO SUBMIT (mtg-5ejgo): connect-on-demand transient WS ───────────
        // With NO live game client, the widget must lazily open a transient lobby
        // connection, send the report (incl. local game logs for repro), run the
        // two-checkbox flow over it, and close the connection when finalized. We
        // stub the transient factory so no real server is needed in the test.
        log('SOLO submit: opens transient lobby connection, runs two-phase flow, then closes it...');
        await page.evaluate(() => {
            window.__bugReportTransientLog = [];
            window.__bugReportTransientFactory = (url) => {
                const conn = {
                    url,
                    sent: [],
                    closed: false,
                    onBugReportStored: null,
                    onBugReportIssueResult: null,
                    isConnected() { return !this.closed; },
                    send(json) { this.sent.push(JSON.parse(json)); },
                    close() { this.closed = true; },
                };
                window.__bugReportTransientLog.push(conn);
                window.__bugReportLastTransient = conn;
                return Promise.resolve(conn);
            };
        });
        await page.fill('#bug-report-description', 'Solo game bug: this should still file a report.');
        await page.click('#btn-bug-report-submit');
        // The transient connection opened and the report was sent over it.
        await page.waitForFunction(() => {
            const conn = window.__bugReportLastTransient;
            return conn && conn.sent.length === 1 && conn.sent[0].type === 'bug_report'
                && typeof conn.url === 'string' && conn.url.includes('/lobby');
        }, { timeout: 5000 });
        const soloSent = await page.evaluate(() => window.__bugReportLastTransient.sent[0]);
        if (!soloSent.description.includes('Solo game bug')) {
            throw new Error('Solo submission sent the wrong description');
        }
        if (!soloSent.game_logs) {
            throw new Error('Solo submission omitted local game logs (needed for repro)');
        }
        // Drive both phases over the transient connection.
        await page.evaluate(() => {
            window.__bugReportLastTransient.onBugReportStored({
                type: 'bug_report_stored', success: true, report_dir: 'bug_reports/solo1', error: null
            });
            window.__bugReportLastTransient.onBugReportIssueResult({
                type: 'bug_report_issue_result', issue_url: 'https://github.com/example/repo/issues/777', error: null
            });
        });
        await page.waitForFunction(() => {
            const disk = document.getElementById('bug-report-check-disk');
            const github = document.getElementById('bug-report-check-github');
            const submit = document.getElementById('btn-bug-report-submit');
            const conn = window.__bugReportLastTransient;
            return disk && disk.dataset.state === 'ok'
                && github && github.dataset.state === 'ok'
                && submit && submit.disabled && submit.textContent === 'Already submitted'
                && conn && conn.closed === true;
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'solo_submit_two_phase', timestamp: new Date().toISOString() });
        await closeBugReportModal(page);
        // Remove the stub so the remaining cases use the injected live client.
        await page.evaluate(() => { delete window.__bugReportTransientFactory; delete window.__bugReportLastTransient; });

        log('Installing mock CONNECTED network transport...');
        await page.evaluate(() => {
            window.__bugReportSentMessages = [];
            window.__bugReportTestHelpers.setNetworkClient({
                isConnected() {
                    return true;
                },
                send(json) {
                    window.__bugReportSentMessages.push(JSON.parse(json));
                }
            });
        });

        // mtg-596: with a connected transport the precheck must enable Submit
        // and clear the not-connected banner.
        log('Checking precheck (connected) enables Submit and clears banner...');
        await openBugReportModal(page);
        await page.waitForFunction(() => {
            const submit = document.getElementById('btn-bug-report-submit');
            const status = document.getElementById('bug-report-status')?.textContent || '';
            return submit && !submit.disabled && !status.includes('Not connected');
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'precheck_connected', timestamp: new Date().toISOString() });

        log('Checking empty-description validation (connected)...');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            return status.includes('Enter a bug description before submitting.');
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'validation_error', timestamp: new Date().toISOString() });

        // ── HAPPY PATH (mtg-5ejgo): two checkboxes, phase 1 then phase 2 ──────
        // Box 1 ("saved to disk") checks on the immediate stored-confirmation;
        // box 2 ("filed on GitHub") checks + shows a link on the issue result;
        // then Submit finalizes to a disabled "Already submitted".
        log('Two-phase happy path: stored-confirm checks box 1, issue-result checks box 2 + link...');
        await page.fill('#bug-report-description', 'Successful bug report should show issue link.');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 1, { timeout: 5000 });
        const sentSuccessMessage = await page.evaluate(() => window.__bugReportSentMessages[0]);
        if (sentSuccessMessage.type !== 'bug_report') {
            throw new Error('Successful submission did not send type=bug_report');
        }
        if (!sentSuccessMessage.description.includes('Successful bug report')) {
            throw new Error('Successful submission sent the wrong description');
        }
        if (!sentSuccessMessage.game_logs || !sentSuccessMessage.console_logs) {
            throw new Error('Successful submission omitted captured logs');
        }
        if ('trusted_password' in sentSuccessMessage) {
            throw new Error('trusted_password should be omitted when password field is left blank');
        }
        // Both checkboxes should be present and pending before any server reply.
        await page.waitForFunction(() => {
            const disk = document.getElementById('bug-report-check-disk');
            const github = document.getElementById('bug-report-check-github');
            return disk && github && disk.dataset.state === 'pending' && github.dataset.state === 'pending';
        }, { timeout: 5000 });

        // Phase 1: stored confirmation → box 1 checks, box 2 still pending,
        // Submit still disabled (not yet finalized).
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportStored({
                type: 'bug_report_stored',
                success: true,
                report_dir: 'bug_reports/1780537595823',
                error: null
            });
        });
        await page.waitForFunction(() => {
            const disk = document.getElementById('bug-report-check-disk');
            const github = document.getElementById('bug-report-check-github');
            const submit = document.getElementById('btn-bug-report-submit');
            return disk && disk.dataset.state === 'ok'
                && github && github.dataset.state === 'pending'
                && submit && submit.disabled && submit.textContent !== 'Already submitted';
        }, { timeout: 5000 });

        // Phase 2: issue result with a URL → box 2 checks + link, Submit finalizes.
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportIssueResult({
                type: 'bug_report_issue_result',
                issue_url: 'https://github.com/example/repo/issues/123',
                error: null
            });
        });
        await page.waitForFunction(() => {
            const github = document.getElementById('bug-report-check-github');
            const link = document.querySelector('#bug-report-check-github a');
            const submit = document.getElementById('btn-bug-report-submit');
            return github && github.dataset.state === 'ok'
                && link && link.href.includes('/issues/123')
                && submit && submit.disabled && submit.textContent === 'Already submitted';
        }, { timeout: 5000 });
        const successState = await readBugReportForm(page);
        if (!successState.issueUrl || !successState.issueUrl.includes('/issues/123')) {
            throw new Error('Successful bug report did not render the issue URL');
        }
        testResults.steps.push({ name: 'success_two_phase', timestamp: new Date().toISOString() });
        await closeBugReportModal(page);

        // ── GITHUB-FAILED PATH (mtg-5ejgo): no spinner, button still finalizes ─
        // Box 1 checks, box 2 shows a clear failure ("report saved"), and the
        // Submit button STILL finalizes because the report IS saved.
        log('GitHub-failed path: box 2 shows failure (no spinner), button still finalizes...');
        await openBugReportModal(page);
        await page.waitForFunction(() => {
            const submit = document.getElementById('btn-bug-report-submit');
            return submit && !submit.disabled;
        }, { timeout: 5000 });
        await page.fill('#bug-report-description', 'GitHub down should not spin forever.');
        await page.fill('#bug-report-password', 'wrong-password');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 2, { timeout: 5000 });
        const sentErrorMessage = await page.evaluate(() => window.__bugReportSentMessages[1]);
        if (sentErrorMessage.trusted_password !== 'wrong-password') {
            throw new Error('Bug report error case did not serialize trusted_password');
        }
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportStored({
                type: 'bug_report_stored',
                success: true,
                report_dir: 'bug_reports/1780537595999',
                error: null
            });
        });
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportIssueResult({
                type: 'bug_report_issue_result',
                issue_url: null,
                error: 'GitHub issue creation failed: gh not found'
            });
        });
        await page.waitForFunction(() => {
            const disk = document.getElementById('bug-report-check-disk');
            const github = document.getElementById('bug-report-check-github');
            const submit = document.getElementById('btn-bug-report-submit');
            const githubText = github?.textContent || '';
            return disk && disk.dataset.state === 'ok'
                && github && github.dataset.state === 'fail'
                && githubText.includes('report saved')
                && submit && submit.disabled && submit.textContent === 'Already submitted';
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'github_failed_no_spinner', timestamp: new Date().toISOString() });
        await closeBugReportModal(page);

        // ── CLIENT-SIDE TIMEOUT BACKSTOP (mtg-5ejgo) ──────────────────────────
        // If the phase-2 message never arrives, the client itself must resolve
        // box 2 to "status unknown — report saved" and finalize the button. Use a
        // short backstop so the test does not wait the production 18s.
        log('Client-side backstop: missing issue-result still finalizes (no spinner)...');
        await page.evaluate(() => { window.__bugReportBackstopMs = 800; });
        await openBugReportModal(page);
        await page.waitForFunction(() => {
            const submit = document.getElementById('btn-bug-report-submit');
            return submit && !submit.disabled;
        }, { timeout: 5000 });
        await page.fill('#bug-report-description', 'Server drops the second message entirely.');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 3, { timeout: 5000 });
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportStored({
                type: 'bug_report_stored',
                success: true,
                report_dir: 'bug_reports/1780537596111',
                error: null
            });
        });
        // Deliberately send NO issue result; the backstop must fire.
        await page.waitForFunction(() => {
            const github = document.getElementById('bug-report-check-github');
            const submit = document.getElementById('btn-bug-report-submit');
            const githubText = github?.textContent || '';
            return github && github.dataset.state === 'unknown'
                && githubText.includes('report saved')
                && submit && submit.disabled && submit.textContent === 'Already submitted';
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'client_backstop', timestamp: new Date().toISOString() });
        await closeBugReportModal(page);
        await page.evaluate(() => { delete window.__bugReportBackstopMs; });

        // ── DISK-WRITE FAILURE (mtg-5ejgo): box 1 shows ✗, user may retry ─────
        log('Disk-write failure: box 1 shows error, Submit re-enabled (not finalized)...');
        await openBugReportModal(page);
        await page.waitForFunction(() => {
            const submit = document.getElementById('btn-bug-report-submit');
            return submit && !submit.disabled;
        }, { timeout: 5000 });
        await page.fill('#bug-report-description', 'Disk is full on the server.');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 4, { timeout: 5000 });
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportStored({
                type: 'bug_report_stored',
                success: false,
                report_dir: null,
                error: 'No space left on device'
            });
        });
        await page.waitForFunction(() => {
            const disk = document.getElementById('bug-report-check-disk');
            const submit = document.getElementById('btn-bug-report-submit');
            const diskText = disk?.textContent || '';
            return disk && disk.dataset.state === 'fail'
                && diskText.includes('No space left on device')
                && submit && !submit.disabled && submit.textContent === 'Submit';
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'disk_failure_retryable', timestamp: new Date().toISOString() });

        await closeBugReportModal(page);
        await page.screenshot({ path: path.join(screenshotDir, 'bug_report_04_complete.png'), fullPage: true });

        const hasPanicError = testResults.errors.some(e =>
            e.error.includes('panic') || e.error.includes('unreachable')
        );
        if (hasPanicError) {
            throw new Error('WASM panicked during bug report test');
        }

        testResults.endTime = new Date().toISOString();
        testResults.success = true;
        fs.writeFileSync(
            path.join(screenshotDir, 'bug_report_test_results.json'),
            JSON.stringify(testResults, null, 2)
        );
        log('Bug report E2E test passed');
        return true;
    } catch (error) {
        log(`=== Test Failed: ${error.message} ===`);
        testResults.endTime = new Date().toISOString();
        testResults.success = false;
        testResults.failureReason = error.message;

        try {
            fs.writeFileSync(
                path.join(screenshotDir, 'bug_report_test_results.json'),
                JSON.stringify(testResults, null, 2)
            );
        } catch (e) {
            // Ignore result write failures
        }

        return false;
    } finally {
        if (browser) await browser.close();
        if (server) server.kill();
    }
}

runTest().then(success => {
    process.exit(success ? 0 : 1);
});
