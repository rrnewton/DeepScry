// Fancy TUI Browser Test using Playwright
// Run with: node test_fancy_tui.js
//
// This test:
// 1. Loads the fancy TUI page
// 2. Launches a game
// 3. Steps through several turns
// 4. Takes screenshots at each step
// 5. Logs timestamps for performance correlation

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { enableReplayVerifier, checkForFatalErrors } = require('./test_network_utils');

// Timestamped logging for correlation with server logs
function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

async function runTest() {
    let server;
    let browser;
    const testResults = {
        startTime: new Date().toISOString(),
        steps: [],
        errors: [],
        browserLogs: []
    };

    try {
        // Start a simple HTTP server
        log('Starting HTTP server on port 8766...');
        server = spawn('python3', ['-m', 'http.server', '8766'], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });

        // Capture server output
        server.stdout.on('data', (data) => {
            log(`Server: ${data.toString().trim()}`);
        });
        server.stderr.on('data', (data) => {
            log(`Server: ${data.toString().trim()}`);
        });

        // Wait for server to start
        await new Promise(resolve => setTimeout(resolve, 1000));

        // Launch browser
        log('Launching Chromium browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        // Collect console messages and errors with timestamps
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

        // Navigate to the fancy TUI page
        log('Loading fancy TUI page...');
        const loadStart = Date.now();
        await page.goto('http://localhost:8766/tui_game.html', {
            waitUntil: 'networkidle',
            timeout: 60000
        });
        testResults.steps.push({
            name: 'page_load',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - loadStart
        });

        // Wait for WASM to initialize (launcher should be visible)
        log('Waiting for WASM to load...');
        const wasmStart = Date.now();
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
        testResults.steps.push({
            name: 'wasm_init',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - wasmStart
        });

        log('=== WASM Module Loaded ===');

        // Enable the rewind/replay verifier so any divergence between the
        // pre-rewind snapshot and the post-replay state surfaces as a
        // "REWIND/REPLAY FATAL" log entry that this test treats as failure.
        // Heuristic-vs-heuristic games don't normally rewind (only the human
        // controller path triggers rewind/replay), but the toggle is cheap
        // and harmless when no rewinds occur — and it guards future test
        // additions that DO drive human input through this script.
        const verifierEnabled = await enableReplayVerifier(page);
        log(`Replay verifier enabled: ${verifierEnabled}`);

        // Get status
        const status = await page.evaluate(() => {
            return document.getElementById('status')?.textContent;
        });
        log(`Status: ${status}`);

        // Check available decks
        const deckCount = await page.evaluate(() => {
            return document.getElementById('p1-deck')?.options.length || 0;
        });
        log(`Available decks: ${deckCount}`);

        if (deckCount === 0) {
            throw new Error('No decks loaded! Make sure to run "mtg export-wasm" first.');
        }

        // Take screenshot before launching TUI
        const screenshotDir = path.join(__dirname, 'screenshots');
        if (!fs.existsSync(screenshotDir)) {
            fs.mkdirSync(screenshotDir);
        }

        await page.screenshot({ path: path.join(screenshotDir, '01_setup.png'), fullPage: true });
        log('Screenshot: 01_setup.png');

        // Select same deck for both players to avoid missing-card errors
        const firstDeck = await page.evaluate(() => {
            const sel = document.getElementById('p1-deck');
            return sel?.options[0]?.value || '';
        });
        if (firstDeck) {
            await page.evaluate((deck) => {
                document.getElementById('p1-deck').value = deck;
                document.getElementById('p2-deck').value = deck;
            }, firstDeck);
            log(`Selected deck for both players: ${firstDeck}`);
        }

        // Set both controllers to heuristic AI so turns auto-advance
        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');

        // Launch the fancy TUI
        log('Clicking "Launch Fancy TUI" button...');
        const launchStart = Date.now();
        await page.click('#btn-launch');

        // Wait for the RatZilla terminal to appear (it's a div, not canvas)
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 10000 });

        // Wait for game controls to appear (indicates TUI is ready)
        await page.waitForSelector('#game-controls', { state: 'visible', timeout: 10000 });
        testResults.steps.push({
            name: 'tui_launch',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - launchStart
        });
        log('TUI launched successfully');

        // Wait a moment for initial rendering
        await page.waitForTimeout(500);

        // Take screenshot after launch
        await page.screenshot({ path: path.join(screenshotDir, '02_tui_launched.png'), fullPage: true });
        log('Screenshot: 02_tui_launched.png');

        // Get initial turn info
        let turnInfo = await page.evaluate(() => {
            return document.getElementById('turn-info')?.textContent || 'N/A';
        });
        log(`Initial turn info: ${turnInfo}`);

        // Expand the controls panel (it starts collapsed)
        await page.click('#btn-toggle-controls');
        await page.waitForSelector('#controls-panel', { state: 'visible', timeout: 5000 });
        log('Expanded controls panel');

        // Step through several turns
        const turnsToRun = 5;
        for (let i = 1; i <= turnsToRun; i++) {
            log(`Running turn ${i}/${turnsToRun}...`);
            const turnStart = Date.now();

            // Click the "Run 1 Turn" button
            await page.click('#btn-run-turn');

            // Wait a moment for the turn to complete and render
            await page.waitForTimeout(300);

            const turnDuration = Date.now() - turnStart;
            testResults.steps.push({
                name: `turn_${i}`,
                timestamp: new Date().toISOString(),
                durationMs: turnDuration
            });

            // Get updated turn info
            turnInfo = await page.evaluate(() => {
                return document.getElementById('turn-info')?.textContent || 'N/A';
            });
            log(`After turn ${i}: ${turnInfo} (${turnDuration}ms)`);

            // Take screenshot every turn
            await page.screenshot({
                path: path.join(screenshotDir, `03_turn_${i.toString().padStart(2, '0')}.png`),
                fullPage: true
            });
        }

        log('Screenshot: 03_turn_XX.png (for each turn)');

        // Test bug report modal, payload capture, disconnected error, and response handling
        log('Testing bug report modal...');
        await page.evaluate(() => {
            console.log('[TestBugReport] console-log-marker');
            console.warn('[TestBugReport] console-warn-marker');
            console.error('[TestBugReport] console-error-marker');
        });
        await page.click('#btn-bug-report');
        await page.waitForSelector('#bug-report-modal', { state: 'visible', timeout: 5000 });
        // mtg-596: in this local (non-network) game there is no server WS, so the
        // precheck must show a persistent "not connected" banner up front AND
        // disable Submit — before the user types anything.
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            const submit = document.getElementById('btn-bug-report-submit');
            return status.includes('Not connected') && submit && submit.disabled;
        }, { timeout: 5000 });

        // Install a connected mock transport so the precheck enables Submit and
        // the report can actually be assembled + sent (mtg-587/596).
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
        await page.waitForFunction(() => {
            const submit = document.getElementById('btn-bug-report-submit');
            return submit && !submit.disabled;
        }, { timeout: 5000 });

        await page.fill('#bug-report-description', 'Expected remote submission success.');
        await page.fill('#bug-report-password', 'trusted-secret');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 1, { timeout: 5000 });
        // Draft payload capture (description, password, console + game logs).
        const bugReportDraft = await page.evaluate(() => window.lastBugReportDraft || null);
        if (!bugReportDraft) {
            throw new Error('Bug report draft payload was not captured');
        }
        if (bugReportDraft.description !== 'Expected remote submission success.') {
            throw new Error('Bug report description was not captured correctly');
        }
        if (bugReportDraft.trustedPassword !== 'trusted-secret') {
            throw new Error('Bug report password was not captured correctly');
        }
        if (!Array.isArray(bugReportDraft.consoleLogs) || !bugReportDraft.consoleLogs.some(line => line.includes('console-error-marker'))) {
            throw new Error('Bug report console log capture missing expected marker');
        }
        if (!Array.isArray(bugReportDraft.gameLogs) || bugReportDraft.gameLogs.length === 0) {
            throw new Error('Bug report game logs were not captured');
        }
        const sentBugReport = await page.evaluate(() => window.__bugReportSentMessages[0]);
        if (sentBugReport.trusted_password !== 'trusted-secret') {
            throw new Error('Bug report trusted_password field was not serialized correctly');
        }
        if (!sentBugReport.console_logs.includes('console-error-marker')) {
            throw new Error('Bug report console_logs string missing expected marker');
        }
        if (sentBugReport.type !== 'bug_report') {
            throw new Error('Bug report was not sent with the correct message type');
        }
        if (sentBugReport.description !== 'Expected remote submission success.') {
            throw new Error('Bug report send payload used the wrong description');
        }
        if (!sentBugReport.game_logs || !sentBugReport.console_logs) {
            throw new Error('Bug report send payload did not include serialized logs');
        }

        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportResult({
                success: true,
                issue_url: 'https://github.com/example/issues/123',
                error: null
            });
        });
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status');
            const link = status?.querySelector('a');
            return link && link.href.includes('/issues/123');
        }, { timeout: 5000 });
        const clearedAfterSuccess = await page.evaluate(() => ({
            description: document.getElementById('bug-report-description')?.value || '',
            password: document.getElementById('bug-report-password')?.value || ''
        }));
        if (clearedAfterSuccess.description !== '' || clearedAfterSuccess.password !== '') {
            throw new Error('Bug report form did not reset after successful submission');
        }

        await page.fill('#bug-report-description', 'Expected local-save success without issue URL.');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 2, { timeout: 5000 });
        await page.evaluate(() => {
            window.__bugReportTestTransport.onBugReportResult({
                success: true,
                issue_url: null,
                error: null
            });
        });
        await page.waitForFunction(() => {
            const status = document.getElementById('bug-report-status')?.textContent || '';
            return status.includes('Bug report saved locally');
        }, { timeout: 5000 });

        await page.fill('#bug-report-description', 'Expected server-side validation error.');
        await page.click('#btn-bug-report-submit');
        await page.waitForFunction(() => (window.__bugReportSentMessages || []).length === 3, { timeout: 5000 });
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

        await page.click('#btn-bug-report-cancel');
        await page.waitForSelector('#bug-report-modal', { state: 'hidden', timeout: 5000 });

        // Test auto-run briefly
        log('Testing auto-run mode...');
        const autoStart = Date.now();
        await page.click('#btn-auto');

        // Let it run for 2 seconds
        await page.waitForTimeout(2000);

        // Stop auto-run
        await page.click('#btn-auto');
        testResults.steps.push({
            name: 'auto_run_test',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - autoStart
        });

        // Get final turn info
        turnInfo = await page.evaluate(() => {
            return document.getElementById('turn-info')?.textContent || 'N/A';
        });
        log(`After auto-run: ${turnInfo}`);

        // Take final screenshot
        await page.screenshot({ path: path.join(screenshotDir, '04_after_autorun.png'), fullPage: true });
        log('Screenshot: 04_after_autorun.png');

        // Check for game over
        const gameOver = await page.evaluate(() => {
            const info = document.getElementById('turn-info');
            return info?.textContent?.includes('Game Over') || false;
        });
        if (gameOver) {
            log('Game ended during test');
        }

        // Check for any panic errors
        const hasPanicError = testResults.errors.some(e =>
            e.error.includes('panic') || e.error.includes('unreachable')
        );
        if (hasPanicError) {
            log('=== WASM Panic Detected ===');
            throw new Error('WASM panicked during test');
        }

        // Check for REWIND/REPLAY FATAL or other fatal patterns surfaced via
        // the browser console. checkForFatalErrors covers desync messages and
        // — since this commit — replay-verifier divergences. Treat any hit as
        // a hard failure; the verifier is too noisy to ignore.
        const fatalLog = checkForFatalErrors(testResults.browserLogs);
        if (fatalLog) {
            log('=== Fatal browser-log entry detected ===');
            log(fatalLog);
            throw new Error(`Fatal browser-log entry detected: ${fatalLog}`);
        }

        // Test exit button
        log('Testing exit button...');
        // Note: Exit reloads the page, so we just verify the button exists
        const exitButtonExists = await page.evaluate(() => {
            return !!document.getElementById('btn-exit');
        });
        if (!exitButtonExists) {
            throw new Error('Exit button not found');
        }

        testResults.endTime = new Date().toISOString();
        testResults.success = true;

        // Write test results to JSON
        const resultsPath = path.join(screenshotDir, 'test_results.json');
        fs.writeFileSync(resultsPath, JSON.stringify(testResults, null, 2));
        log(`Test results saved to: ${resultsPath}`);

        // Print performance summary
        log('\n=== Performance Summary ===');
        testResults.steps.forEach(step => {
            log(`  ${step.name}: ${step.durationMs}ms`);
        });

        log('\n=== Fancy TUI Test Passed! ===');
        log('Screenshots saved in web/screenshots/');

        return true;
    } catch (error) {
        log(`=== Test Failed: ${error.message} ===`);
        testResults.endTime = new Date().toISOString();
        testResults.success = false;
        testResults.failureReason = error.message;

        // Try to take a failure screenshot
        if (browser) {
            try {
                const pages = browser.contexts()[0]?.pages();
                if (pages && pages.length > 0) {
                    const screenshotDir = path.join(__dirname, 'screenshots');
                    if (!fs.existsSync(screenshotDir)) {
                        fs.mkdirSync(screenshotDir);
                    }
                    await pages[0].screenshot({ path: path.join(screenshotDir, 'failure.png'), fullPage: true });
                    log('Failure screenshot saved: screenshots/failure.png');
                }
            } catch (e) {
                // Ignore screenshot errors
            }
        }

        // Still save results on failure
        try {
            const screenshotDir = path.join(__dirname, 'screenshots');
            if (!fs.existsSync(screenshotDir)) {
                fs.mkdirSync(screenshotDir);
            }
            const resultsPath = path.join(screenshotDir, 'test_results.json');
            fs.writeFileSync(resultsPath, JSON.stringify(testResults, null, 2));
        } catch (e) {
            // Ignore
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
