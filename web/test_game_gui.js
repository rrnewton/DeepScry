// Native Web GUI (game.html) Browser Test using Playwright
// Run with: node18 test_game_gui.js
//
// This test:
// 1. Loads game.html (native web GUI, NOT the TUI-in-browser)
// 2. Verifies launcher UI elements render correctly
// 3. Launches a game with AI vs AI
// 4. Verifies the 3-column game layout renders
// 5. Steps through turns, takes screenshots at each stage
// 6. Verifies card rendering, player info, log, actions
// 7. Tests auto-run to game completion
// 8. Tests exit and return to launcher

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

async function runTest() {
    let server;
    let browser;
    const PORT = 8767;
    const testResults = {
        startTime: new Date().toISOString(),
        steps: [],
        errors: [],
        browserLogs: [],
        findings: []
    };

    function finding(severity, msg) {
        testResults.findings.push({ severity, msg });
        log(`[${severity}] ${msg}`);
    }

    try {
        // Start HTTP server
        log('Starting HTTP server on port ' + PORT + '...');
        server = spawn('python3', ['-m', 'http.server', String(PORT)], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });
        server.stderr.on('data', (data) => {
            // python http.server logs to stderr
        });

        await new Promise(resolve => setTimeout(resolve, 1000));

        // Launch browser
        log('Launching Chromium...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });

        // Collect console messages and errors
        page.on('console', msg => {
            testResults.browserLogs.push({
                timestamp: new Date().toISOString(),
                type: msg.type(),
                text: msg.text()
            });
            if (msg.type() === 'error') {
                log(`Browser ERROR: ${msg.text()}`);
            }
        });

        page.on('pageerror', err => {
            testResults.errors.push({
                timestamp: new Date().toISOString(),
                error: err.message
            });
            log(`Page ERROR: ${err.message}`);
        });

        const screenshotDir = path.join(__dirname, 'screenshots');
        if (!fs.existsSync(screenshotDir)) {
            fs.mkdirSync(screenshotDir);
        }

        // ========== STEP 1: Load game.html ==========
        log('Loading game.html...');
        const loadStart = Date.now();
        await page.goto(`http://localhost:${PORT}/game.html`, {
            waitUntil: 'networkidle',
            timeout: 60000
        });
        testResults.steps.push({
            name: 'page_load',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - loadStart
        });

        // Wait for WASM init
        const wasmStart = Date.now();
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
        testResults.steps.push({
            name: 'wasm_init',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - wasmStart
        });
        log('WASM initialized');

        // ========== STEP 2: Verify launcher UI ==========
        log('Verifying launcher UI...');

        // Check header
        const headerText = await page.textContent('.header h1');
        if (!headerText.includes('Native GUI')) {
            finding('WARN', `Header says "${headerText}" — expected "Native GUI"`);
        } else {
            finding('OK', `Header: "${headerText}"`);
        }

        // Check status shows WASM version and deck count
        const status = await page.textContent('#status');
        if (!status.includes('WASM') || !status.includes('decks')) {
            finding('WARN', `Status text unexpected: "${status}"`);
        } else {
            finding('OK', `Status: "${status}"`);
        }

        // Check player panes exist
        const p1Pane = await page.$('.player-pane.p1');
        const p2Pane = await page.$('.player-pane.p2');
        if (!p1Pane || !p2Pane) {
            throw new Error('Player panes not found');
        }
        finding('OK', 'Both player panes present');

        // Check deck dropdowns populated
        const p1DeckCount = await page.evaluate(() =>
            document.getElementById('p1-deck')?.options.length || 0
        );
        const p2DeckCount = await page.evaluate(() =>
            document.getElementById('p2-deck')?.options.length || 0
        );
        if (p1DeckCount === 0 || p2DeckCount === 0) {
            throw new Error(`Deck dropdowns empty: P1=${p1DeckCount}, P2=${p2DeckCount}`);
        }
        finding('OK', `Deck dropdowns: P1=${p1DeckCount} decks, P2=${p2DeckCount} decks`);

        // Check controller dropdowns
        const p1Controllers = await page.evaluate(() => {
            const sel = document.getElementById('p1-controller');
            return Array.from(sel.options).map(o => o.value);
        });
        if (!p1Controllers.includes('human') || !p1Controllers.includes('heuristic')) {
            finding('WARN', 'Missing controller options');
        } else {
            finding('OK', `P1 controllers: ${p1Controllers.join(', ')}`);
        }

        // Check navigation links
        const links = await page.evaluate(() => {
            return Array.from(document.querySelectorAll('.header a')).map(a => ({
                text: a.textContent,
                href: a.getAttribute('href')
            }));
        });
        finding('OK', `Nav links: ${links.map(l => `${l.text} -> ${l.href}`).join(', ')}`);

        // Screenshot: launcher
        await page.screenshot({ path: path.join(screenshotDir, 'game_01_launcher.png'), fullPage: true });
        log('Screenshot: game_01_launcher.png');

        // ========== STEP 3: Configure and launch game ==========
        log('Configuring game...');

        // Set both to heuristic AI
        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');

        // Use the same deck for both players
        const firstDeck = await page.evaluate(() => {
            const sel = document.getElementById('p1-deck');
            return sel?.options[0]?.value || '';
        });
        if (firstDeck) {
            await page.evaluate((deck) => {
                document.getElementById('p2-deck').value = deck;
            }, firstDeck);
        }

        // Set a fixed seed for reproducibility
        await page.fill('#game-seed', '42');

        // Screenshot: configured
        await page.screenshot({ path: path.join(screenshotDir, 'game_02_configured.png'), fullPage: true });
        log('Screenshot: game_02_configured.png');

        // Launch
        log('Launching game...');
        const launchStart = Date.now();
        await page.click('#btn-launch');

        // Wait for game area to become visible
        await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 15000 });
        testResults.steps.push({
            name: 'game_launch',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - launchStart
        });
        log(`Game launched in ${Date.now() - launchStart}ms`);

        // Small delay for initial render
        await page.waitForTimeout(500);

        // ========== STEP 4: Verify game layout ==========
        log('Verifying game layout...');

        // Check 3-column layout
        const gameArea = await page.$('#game-area.show');
        if (!gameArea) throw new Error('Game area not visible');

        // Check all panes exist
        const paneIds = [
            'pane-log', 'pane-actions', 'pane-opp-info', 'pane-opp-field',
            'pane-player-field', 'pane-hand', 'pane-card-details',
            'pane-player-info'
        ];
        for (const id of paneIds) {
            const pane = await page.$(`#${id}`);
            if (!pane) {
                finding('FAIL', `Missing pane: #${id}`);
            }
        }
        finding('OK', `All ${paneIds.length} game panes present`);

        // Check status bar
        const statusBar = await page.$('#status-bar');
        if (!statusBar) {
            finding('FAIL', 'Status bar missing');
        } else {
            const barText = await page.textContent('#status-bar');
            finding('OK', `Status bar: "${barText.trim().substring(0, 80)}..."`);
        }

        // Check controls overlay
        const controls = await page.$('#game-controls');
        if (!controls) {
            finding('FAIL', 'Game controls overlay missing');
        } else {
            finding('OK', 'Game controls overlay present');
        }

        // Screenshot: initial game state
        await page.screenshot({ path: path.join(screenshotDir, 'game_03_initial.png'), fullPage: true });
        log('Screenshot: game_03_initial.png');

        // ========== STEP 5: Verify player info rendering ==========
        log('Checking player info...');

        const playerInfo = await page.evaluate(() => {
            const playerBody = document.getElementById('player-info-body');
            const oppBody = document.getElementById('opp-info-body');
            return {
                playerText: playerBody?.textContent || '',
                oppText: oppBody?.textContent || '',
            };
        });

        if (playerInfo.playerText.includes('life')) {
            finding('OK', 'Player info shows life total');
        } else {
            finding('WARN', 'Player info missing life total');
        }

        if (playerInfo.playerText.includes('Library')) {
            finding('OK', 'Player info shows library count');
        } else {
            finding('WARN', 'Player info missing library count');
        }

        // ========== STEP 6: Step through turns ==========
        const turnsToRun = 5;
        log(`Running ${turnsToRun} turns via keyboard (Space)...`);

        for (let i = 1; i <= turnsToRun; i++) {
            const turnStart = Date.now();

            // Press Space to advance turn
            await page.keyboard.press('Space');
            await page.waitForTimeout(300);

            const turnDuration = Date.now() - turnStart;
            testResults.steps.push({
                name: `turn_${i}`,
                timestamp: new Date().toISOString(),
                durationMs: turnDuration
            });

            // Get status bar info
            const statusText = await page.textContent('#status-bar');
            log(`Turn ${i}: ${statusText.trim().substring(0, 60)} (${turnDuration}ms)`);

            // Screenshot every other turn
            if (i % 2 === 1 || i === turnsToRun) {
                await page.screenshot({
                    path: path.join(screenshotDir, `game_04_turn_${i.toString().padStart(2, '0')}.png`),
                    fullPage: true
                });
            }
        }

        // ========== STEP 7: Verify game state after turns ==========
        log('Checking game state after turns...');

        // Check log has entries
        const logEntryCount = await page.evaluate(() =>
            document.querySelectorAll('#log-body .log-entry').length
        );
        if (logEntryCount > 0) {
            finding('OK', `Log has ${logEntryCount} entries`);
        } else {
            finding('WARN', 'Log is empty after turns');
        }

        // Check hand cards
        const handCardCount = await page.evaluate(() =>
            document.querySelectorAll('#hand-cards .card').length
        );
        finding('OK', `Hand shows ${handCardCount} card elements`);

        // Check hand header count
        const handHeader = await page.textContent('#hand-header');
        finding('OK', `Hand header: "${handHeader}"`);

        // Check battlefield cards
        const playerFieldCount = await page.evaluate(() =>
            document.querySelectorAll('#player-field-cards .card').length
        );
        const oppFieldCount = await page.evaluate(() =>
            document.querySelectorAll('#opp-field-cards .card').length
        );
        finding('OK', `Battlefield: player=${playerFieldCount} cards, opp=${oppFieldCount} cards`);

        // Check stack
        const stackText = await page.textContent('#stack-body');
        finding('OK', `Stack: "${stackText.trim().substring(0, 40)}"`);

        // Screenshot: mid-game state
        await page.screenshot({ path: path.join(screenshotDir, 'game_05_midgame.png'), fullPage: true });
        log('Screenshot: game_05_midgame.png');

        // ========== STEP 8: Test card click interaction ==========
        log('Testing card click...');

        // Try to click a card in hand or battlefield
        const cardToClick = await page.$('#hand-cards .card, #player-field-cards .card, #opp-field-cards .card');
        if (cardToClick) {
            await cardToClick.click();
            await page.waitForTimeout(200);

            const detailsContent = await page.textContent('#card-details-body');
            if (detailsContent && !detailsContent.includes('Click a card')) {
                finding('OK', `Card click shows details: "${detailsContent.trim().substring(0, 50)}..."`);
            } else {
                finding('WARN', 'Card click did not update details pane');
            }

            // Screenshot: card details
            await page.screenshot({ path: path.join(screenshotDir, 'game_06_card_details.png'), fullPage: true });
            log('Screenshot: game_06_card_details.png');
        } else {
            finding('WARN', 'No cards found to click');
        }

        // ========== STEP 9: Test auto-run ==========
        log('Testing auto-run...');
        const autoStart = Date.now();

        // Press 'a' for auto-run
        await page.keyboard.press('a');
        await page.waitForTimeout(3000);

        // Stop auto-run
        await page.keyboard.press('a');
        await page.waitForTimeout(200);

        testResults.steps.push({
            name: 'auto_run',
            timestamp: new Date().toISOString(),
            durationMs: Date.now() - autoStart
        });

        const afterAutoStatus = await page.textContent('#status-bar');
        log(`After auto-run: ${afterAutoStatus.trim().substring(0, 80)}`);

        // Check if game is over
        const gameOver = await page.evaluate(() => {
            const bar = document.getElementById('status-bar');
            return bar?.textContent?.includes('GAME OVER') || false;
        });

        if (gameOver) {
            finding('OK', 'Game completed (GAME OVER detected)');
        } else {
            // Run more turns to try to finish
            log('Game not over yet, running more auto turns...');
            await page.keyboard.press('a');
            await page.waitForTimeout(5000);
            await page.keyboard.press('a');
        }

        // Screenshot: after auto-run
        await page.screenshot({ path: path.join(screenshotDir, 'game_07_after_autorun.png'), fullPage: true });
        log('Screenshot: game_07_after_autorun.png');

        // ========== STEP 10: Check for errors ==========
        log('Checking for JS errors...');

        const errorBanner = await page.evaluate(() => {
            const banner = document.getElementById('js-error-banner');
            return banner ? banner.style.display : 'not-found';
        });
        if (errorBanner === 'none' || errorBanner === 'not-found') {
            finding('OK', 'No JS error banner displayed');
        } else {
            const errorText = await page.textContent('#js-error-messages');
            finding('FAIL', `JS error banner visible: ${errorText.substring(0, 100)}`);
        }

        // Check for WASM panics
        const hasPanic = testResults.errors.some(e =>
            e.error.includes('panic') || e.error.includes('unreachable')
        );
        if (hasPanic) {
            finding('FAIL', 'WASM panic detected!');
            throw new Error('WASM panicked during game.html test');
        } else {
            finding('OK', 'No WASM panics');
        }

        // ========== STEP 11: Test exit ==========
        log('Testing exit...');
        await page.keyboard.press('q');
        await page.waitForTimeout(500);

        // Should return to launcher
        const launcherVisible = await page.evaluate(() =>
            document.getElementById('launcher')?.classList.contains('show') || false
        );
        if (launcherVisible) {
            finding('OK', 'Exit returns to launcher');
        } else {
            finding('WARN', 'Exit did not return to launcher (game may have already ended)');
        }

        // Screenshot: after exit
        await page.screenshot({ path: path.join(screenshotDir, 'game_08_after_exit.png'), fullPage: true });
        log('Screenshot: game_08_after_exit.png');

        // ========== RESULTS ==========
        testResults.endTime = new Date().toISOString();
        testResults.success = true;

        const resultsPath = path.join(screenshotDir, 'game_gui_test_results.json');
        fs.writeFileSync(resultsPath, JSON.stringify(testResults, null, 2));
        log(`Results saved to: ${resultsPath}`);

        log('\n=== Performance Summary ===');
        testResults.steps.forEach(step => {
            log(`  ${step.name}: ${step.durationMs}ms`);
        });

        log('\n=== Findings Summary ===');
        const counts = { OK: 0, WARN: 0, FAIL: 0 };
        testResults.findings.forEach(f => { counts[f.severity] = (counts[f.severity] || 0) + 1; });
        log(`  OK: ${counts.OK}  WARN: ${counts.WARN}  FAIL: ${counts.FAIL}`);

        if (counts.FAIL > 0) {
            log('\n=== game.html GUI Test FAILED ===');
            return false;
        }

        log('\n=== game.html GUI Test PASSED ===');
        log('Screenshots saved in web/screenshots/game_*.png');
        return true;
    } catch (error) {
        log(`=== Test Failed: ${error.message} ===`);
        testResults.endTime = new Date().toISOString();
        testResults.success = false;
        testResults.failureReason = error.message;

        if (browser) {
            try {
                const pages = browser.contexts()[0]?.pages();
                if (pages && pages.length > 0) {
                    await pages[0].screenshot({
                        path: path.join(__dirname, 'screenshots', 'game_failure.png'),
                        fullPage: true
                    });
                    log('Failure screenshot: game_failure.png');
                }
            } catch (e) { /* ignore */ }
        }

        try {
            const resultsPath = path.join(__dirname, 'screenshots', 'game_gui_test_results.json');
            fs.writeFileSync(resultsPath, JSON.stringify(testResults, null, 2));
        } catch (e) { /* ignore */ }

        return false;
    } finally {
        if (browser) await browser.close();
        if (server) server.kill();
    }
}

runTest().then(success => {
    process.exit(success ? 0 : 1);
});
