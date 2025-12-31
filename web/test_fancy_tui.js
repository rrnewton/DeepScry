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
        await page.goto('http://localhost:8766/fancy.html', {
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
