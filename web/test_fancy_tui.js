// Fancy TUI Browser Test using Playwright
// Run with: node test_fancy_tui.js

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');

async function runTest() {
    let server;
    let browser;

    try {
        // Start a simple HTTP server
        console.log('Starting HTTP server...');
        server = spawn('python3', ['-m', 'http.server', '8766'], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });

        // Wait for server to start
        await new Promise(resolve => setTimeout(resolve, 1000));

        // Launch browser
        console.log('Launching browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        // Collect console messages and errors
        const logs = [];
        const errors = [];
        page.on('console', msg => {
            const text = `[${msg.type()}] ${msg.text()}`;
            logs.push(text);
            console.log(`  Browser: ${msg.text()}`);
        });

        page.on('pageerror', err => {
            errors.push(err.message);
            console.error(`  Page error: ${err.message}`);
        });

        // Navigate to the fancy TUI page
        console.log('Loading fancy TUI page...');
        await page.goto('http://localhost:8766/fancy.html', {
            waitUntil: 'networkidle',
            timeout: 60000
        });

        // Wait for WASM to initialize (setup bar should be visible)
        console.log('Waiting for WASM to load...');
        await page.waitForSelector('#setup-bar', { state: 'visible', timeout: 30000 });

        console.log('\n=== WASM Module Loaded ===\n');

        // Get status
        const status = await page.evaluate(() => {
            return document.getElementById('status')?.textContent;
        });
        console.log(`Status: ${status}`);

        // Check available decks
        const deckCount = await page.evaluate(() => {
            return document.getElementById('p1-deck')?.options.length || 0;
        });
        console.log(`Available decks: ${deckCount}`);

        if (deckCount === 0) {
            throw new Error('No decks loaded! Make sure to run "mtg export-wasm" first.');
        }

        // Take screenshot before launching TUI
        const screenshotDir = path.join(__dirname, 'screenshots');
        if (!fs.existsSync(screenshotDir)) {
            fs.mkdirSync(screenshotDir);
        }
        await page.screenshot({ path: path.join(screenshotDir, '01_setup.png'), fullPage: true });
        console.log('Screenshot saved: screenshots/01_setup.png');

        // Launch the fancy TUI
        console.log('\nLaunching fancy TUI...');
        await page.click('#btn-launch');

        // Wait for the canvas to appear
        await page.waitForSelector('#mtg-fancy-tui-canvas', { state: 'visible', timeout: 10000 });
        console.log('Canvas is visible');

        // Wait a moment for rendering
        await page.waitForTimeout(2000);

        // Take screenshot after launch
        await page.screenshot({ path: path.join(screenshotDir, '02_tui_launched.png'), fullPage: true });
        console.log('Screenshot saved: screenshots/02_tui_launched.png');

        // Check for any errors
        const hasPanicError = errors.some(e => e.includes('panic') || e.includes('unreachable'));
        if (hasPanicError) {
            console.error('\n=== WASM Panic Detected ===');
            throw new Error('WASM panicked during TUI rendering');
        }

        // Wait a bit longer and take another screenshot to check for growth
        console.log('\nWaiting 3 more seconds to check for growth issues...');
        await page.waitForTimeout(3000);

        await page.screenshot({ path: path.join(screenshotDir, '03_after_wait.png'), fullPage: true });
        console.log('Screenshot saved: screenshots/03_after_wait.png');

        // Check canvas size didn't explode
        const canvasSize = await page.evaluate(() => {
            const canvas = document.getElementById('mtg-fancy-tui-canvas');
            return {
                width: canvas?.width,
                height: canvas?.height,
                clientWidth: canvas?.clientWidth,
                clientHeight: canvas?.clientHeight
            };
        });
        console.log(`Canvas size: ${canvasSize.width}x${canvasSize.height} (client: ${canvasSize.clientWidth}x${canvasSize.clientHeight})`);

        // Check for texture size error
        const hasTextureError = logs.some(l => l.includes('maximum supported texture'));
        if (hasTextureError) {
            throw new Error('Texture size exceeded WebGL maximum');
        }

        // Check for ongoing panics
        const finalErrors = errors.filter(e => e.includes('panic') || e.includes('unreachable'));
        if (finalErrors.length > 0) {
            console.error('\nPanic errors found:');
            finalErrors.forEach(e => console.error(`  - ${e}`));
            throw new Error('WASM panicked');
        }

        console.log('\n=== Fancy TUI Test Passed! ===\n');
        console.log('Screenshots saved in web/screenshots/');

        return true;
    } catch (error) {
        console.error('\n=== Test Failed ===');
        console.error(error.message);

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
                    console.log('Failure screenshot saved: screenshots/failure.png');
                }
            } catch (e) {
                // Ignore screenshot errors
            }
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
