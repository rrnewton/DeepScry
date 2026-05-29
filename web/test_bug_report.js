// Bug Report E2E Test using Playwright
// Run with: node test_bug_report.js
//
// This test:
// 1. Loads the fancy TUI page
// 2. Launches a local game to expose the floating controls widget
// 3. Verifies the bug report modal UI and validation
// 4. Verifies disconnected submission handling
// 5. Verifies cancel/reset behavior
// 6. Mocks bug_report_result responses for success and failure flows

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

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

async function launchLocalFancyTui(page, testResults) {
    const deckCount = await page.evaluate(() => {
        return document.getElementById('p1-deck')?.options.length || 0;
    });
    if (deckCount === 0) {
        throw new Error('No decks loaded! Make sure to run "mtg export-wasm" first.');
    }

    // Select same deck for both players to avoid missing-card errors
    await page.evaluate(() => {
        const sel = document.getElementById('p1-deck');
        const firstDeck = sel?.options[0]?.value || '';
        if (firstDeck) {
            sel.value = firstDeck;
            document.getElementById('p2-deck').value = firstDeck;
        }
    });

    // Set both controllers to heuristic AI so turns auto-advance
    await page.selectOption('#p1-controller', 'heuristic');
    await page.selectOption('#p2-controller', 'heuristic');

    await page.click('#btn-launch');
    await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 10000 });
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

        log('Loading fancy TUI page...');
        await page.goto(`http://localhost:${HTTP_PORT}/tui_game.html`, {
            waitUntil: 'networkidle',
            timeout: 60000
        });

        log('Waiting for WASM to load...');
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
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
        await page.waitForSelector('#bug-report-password', { state: 'visible', timeout: 5000 });
        await page.screenshot({ path: path.join(screenshotDir, 'bug_report_03_modal.png'), fullPage: true });
        testResults.steps.push({ name: 'modal_open', timestamp: new Date().toISOString() });

        log('Checking empty-description validation...');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            return status.includes('Enter a bug description before submitting.');
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'validation_error', timestamp: new Date().toISOString() });

        log('Checking disconnected submission error...');
        await page.fill('#bug-report-description', 'Expected connected server; bug report should reject when no WebSocket is present.');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            return status.includes('requires an active network WebSocket connection');
        }, { timeout: 5000 });
        const disconnectedMessage = await page.evaluate(() => window.lastBugReportSubmissionMessage || null);
        if (!disconnectedMessage || disconnectedMessage.type !== 'bug_report') {
            throw new Error('Disconnected submission did not assemble bug_report JSON');
        }
        testResults.steps.push({ name: 'disconnected_error', timestamp: new Date().toISOString() });

        log('Checking cancel closes modal and resets form...');
        await page.fill('#bug-report-description', 'This text should be cleared when the modal closes.');
        await page.fill('#bug-report-password', 'reset-me');
        await closeBugReportModal(page);
        await openBugReportModal(page);
        const resetAfterClose = await readBugReportForm(page);
        if (resetAfterClose.description !== '' || resetAfterClose.password !== '' || resetAfterClose.statusText !== '') {
            throw new Error('Bug report form did not reset after closing the modal');
        }
        testResults.steps.push({ name: 'close_reset', timestamp: new Date().toISOString() });

        log('Installing mock network transport...');
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

        log('Submitting successful bug report and checking issue URL...');
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
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportResult({
                success: true,
                issue_url: 'https://github.com/example/repo/issues/123',
                error: null
            });
        });
        await page.waitForFunction(() => {
            const link = document.querySelector('#bug-report-status a');
            return !!(link && link.href.includes('/issues/123'));
        }, { timeout: 5000 });
        const successState = await readBugReportForm(page);
        if (successState.description !== '' || successState.password !== '') {
            throw new Error('Bug report form did not reset after successful submission');
        }
        if (!successState.issueUrl || !successState.issueUrl.includes('/issues/123')) {
            throw new Error('Successful bug report did not render the issue URL');
        }
        testResults.steps.push({ name: 'success_result', timestamp: new Date().toISOString() });

        log('Submitting failing bug report and checking error display...');
        await page.fill('#bug-report-description', 'Failed bug report should show the server error.');
        await page.fill('#bug-report-password', 'wrong-password');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 2, { timeout: 5000 });
        const sentErrorMessage = await page.evaluate(() => window.__bugReportSentMessages[1]);
        if (sentErrorMessage.trusted_password !== 'wrong-password') {
            throw new Error('Bug report error case did not serialize trusted_password');
        }
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportResult({
                success: false,
                issue_url: null,
                error: 'Trusted bug-report password was rejected'
            });
        });
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            return status.includes('Trusted bug-report password was rejected');
        }, { timeout: 5000 });
        testResults.steps.push({ name: 'error_result', timestamp: new Date().toISOString() });

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
