// WASM Browser Test using Playwright
// Run with: node test_wasm.js

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');

async function runTest() {
    let server;
    let browser;

    try {
        // Start a simple HTTP server
        console.log('Starting HTTP server...');
        server = spawn('python3', ['-m', 'http.server', '8765'], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });

        // Wait for server to start
        await new Promise(resolve => setTimeout(resolve, 1000));

        // Launch browser
        console.log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox']
        });
        const page = await browser.newPage();

        // Collect console messages
        const logs = [];
        page.on('console', msg => {
            logs.push(`[${msg.type()}] ${msg.text()}`);
            console.log(`  Browser: ${msg.text()}`);
        });

        page.on('pageerror', err => {
            console.error(`  Page error: ${err.message}`);
        });

        // Navigate to the test page
        console.log('Loading WASM module and card database...');
        await page.goto('http://localhost:8765/index.html', {
            waitUntil: 'networkidle',
            timeout: 60000
        });

        // Wait for WASM and data to initialize
        await page.waitForFunction(() => {
            return document.getElementById('wasm-status')?.textContent === 'Ready';
        }, { timeout: 30000 });

        console.log('\n=== WASM Module Loaded Successfully ===\n');

        // Get version
        const version = await page.evaluate(() => {
            return document.getElementById('wasm-version')?.textContent;
        });
        console.log(`WASM Version: ${version}`);

        // Check database stats
        const dbStats = await page.evaluate(() => {
            return document.getElementById('db-stats')?.textContent;
        });
        console.log(`Database: ${dbStats}`);

        // Check available decks
        const deckCount = await page.evaluate(() => {
            return document.getElementById('p1-deck')?.options.length || 0;
        });
        console.log(`Available decks: ${deckCount}`);

        if (deckCount === 0) {
            throw new Error('No decks loaded! Make sure to run "mtg export-wasm" first.');
        }

        // Click "New Game" button
        console.log('\nCreating new game...');
        await page.click('#btn-new-game');
        await page.waitForTimeout(500);

        // Get initial game state
        const initialState = await page.evaluate(() => {
            return {
                turnInfo: document.getElementById('turn-info')?.textContent,
                p1Life: document.getElementById('p1-life')?.textContent,
                p2Life: document.getElementById('p2-life')?.textContent,
                p1Deck: document.getElementById('p1-deck-name')?.textContent,
                p2Deck: document.getElementById('p2-deck-name')?.textContent
            };
        });
        console.log(`Initial state: ${initialState.turnInfo}`);
        console.log(`  ${initialState.p1Deck} (Life: ${initialState.p1Life})`);
        console.log(`  ${initialState.p2Deck} (Life: ${initialState.p2Life})`);

        // Run one turn
        console.log('\nRunning one turn...');
        await page.click('#btn-run-turn');
        await page.waitForTimeout(500);

        const afterOneTurn = await page.evaluate(() => {
            return {
                turnInfo: document.getElementById('turn-info')?.textContent,
                p1Life: document.getElementById('p1-life')?.textContent,
                p2Life: document.getElementById('p2-life')?.textContent
            };
        });
        console.log(`After 1 turn: ${afterOneTurn.turnInfo}`);
        console.log(`  P1 Life: ${afterOneTurn.p1Life}, P2 Life: ${afterOneTurn.p2Life}`);

        // Create a new game and run full game
        console.log('\nCreating new game for full run...');
        await page.click('#btn-new-game');
        await page.waitForTimeout(200);

        console.log('Running full AI game (max 100 turns)...');
        await page.click('#btn-run-game');
        await page.waitForTimeout(5000); // Games with real cards take longer

        // Get final result
        const finalState = await page.evaluate(() => {
            return {
                turnInfo: document.getElementById('turn-info')?.textContent,
                p1Life: document.getElementById('p1-life')?.textContent,
                p2Life: document.getElementById('p2-life')?.textContent,
                result: document.getElementById('result')?.textContent,
                resultVisible: document.getElementById('result')?.style.display !== 'none'
            };
        });

        console.log(`\n=== Game Result ===`);
        console.log(`Final state: ${finalState.turnInfo}`);
        console.log(`P1 Life: ${finalState.p1Life}`);
        console.log(`P2 Life: ${finalState.p2Life}`);
        if (finalState.resultVisible) {
            console.log(`Result: ${finalState.result}`);
        }

        // Get logs (first 10 lines)
        const gameLogs = await page.evaluate(() => {
            return document.getElementById('logs')?.textContent?.split('\n').slice(0, 10).join('\n');
        });
        console.log(`\nFirst 10 log lines:\n${gameLogs}`);

        console.log('\n=== All Tests Passed! ===\n');

        return true;
    } catch (error) {
        console.error('\n=== Test Failed ===');
        console.error(error.message);
        return false;
    } finally {
        if (browser) await browser.close();
        if (server) server.kill();
    }
}

runTest().then(success => {
    process.exit(success ? 0 : 1);
});
