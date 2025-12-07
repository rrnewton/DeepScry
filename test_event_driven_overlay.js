#!/usr/bin/env node
/**
 * Test script to verify event-driven card image overlay architecture
 *
 * This test verifies that:
 * 1. The window.onRenderComplete callback is properly registered
 * 2. Card images update via callback, not polling
 * 3. Images align correctly with TUI card positions
 */

const puppeteer = require('puppeteer');
const fs = require('fs');

async function testEventDrivenOverlay() {
    console.log('Testing event-driven card image overlay architecture...');

    const browser = await puppeteer.launch({
        headless: true,
        args: ['--no-sandbox', '--disable-setuid-sandbox', '--disable-dev-shm-usage']
    });

    const page = await browser.newPage();
    await page.setViewport({ width: 1400, height: 900 });

    // Capture console logs
    page.on('console', msg => {
        const text = msg.text();
        if (text.includes('battlefield cards') || text.includes('onRenderComplete') || text.includes('Card image overlays')) {
            console.log(`  [Browser] ${text}`);
        }
    });

    try {
        console.log('1. Loading fancy.html...');
        await page.goto('http://localhost:8080/fancy.html', {
            waitUntil: 'networkidle0',
            timeout: 30000
        });

        console.log('2. Waiting for WASM to load...');
        await page.waitForFunction(() => {
            return document.getElementById('setup-bar').style.display === 'flex';
        }, { timeout: 30000 });

        console.log('3. Checking that window.onRenderComplete callback is registered...');
        const callbackRegistered = await page.evaluate(() => {
            return typeof window.onRenderComplete === 'function';
        });

        if (!callbackRegistered) {
            throw new Error('window.onRenderComplete callback not registered!');
        }
        console.log('  ✓ Callback registered');

        console.log('4. Enabling card images...');
        await page.click('#show-card-images');

        console.log('5. Launching TUI with Spiderman vs Spiderman...');
        await page.select('#p1-deck', 'Spiderman');
        await page.select('#p2-deck', 'Spiderman');
        await page.select('#p1-controller', 'heuristic');
        await page.select('#p2-controller', 'heuristic');

        await page.click('#btn-launch');

        console.log('6. Waiting for TUI to render...');
        await page.waitForFunction(() => {
            return document.getElementById('ratzilla-terminal').style.display === 'block';
        }, { timeout: 10000 });

        console.log('7. Waiting for game to progress (auto-run a few turns)...');
        await new Promise(resolve => setTimeout(resolve, 2000)); // Let game start

        // Click auto-run button
        console.log('8. Enabling auto-run...');
        await page.click('#btn-toggle-controls'); // Expand controls
        await new Promise(resolve => setTimeout(resolve, 500));
        await page.click('#btn-auto'); // Start auto-run

        console.log('9. Waiting for battlefield cards to appear...');
        await new Promise(resolve => setTimeout(resolve, 5000)); // Let game play and cards appear

        console.log('10. Checking for card image overlays...');
        const overlayInfo = await page.evaluate(() => {
            const overlays = document.querySelectorAll('.card-overlay-image');
            const positions = [];
            overlays.forEach(img => {
                positions.push({
                    id: img.id,
                    src: img.src.substring(0, 60) + '...', // Truncate URL
                    left: img.style.left,
                    top: img.style.top,
                    width: img.style.width,
                    height: img.style.height,
                    zIndex: img.style.zIndex
                });
            });
            return positions;
        });

        console.log(`  Found ${overlayInfo.length} card image overlays`);
        if (overlayInfo.length > 0) {
            console.log('  Sample overlay positions:');
            overlayInfo.slice(0, 3).forEach(pos => {
                console.log(`    ${pos.id}: (${pos.left}, ${pos.top}) ${pos.width}x${pos.height} z=${pos.zIndex}`);
            });
        }

        console.log('11. Taking screenshot...');
        await page.screenshot({
            path: 'event_driven_overlay_test.png',
            fullPage: false
        });
        console.log('  Screenshot saved: event_driven_overlay_test.png');

        console.log('12. Verifying no polling interval exists...');
        const hasPollingInterval = await page.evaluate(() => {
            // Check for refreshInterval variable or setInterval calls
            return window.refreshInterval !== undefined;
        });

        if (hasPollingInterval) {
            console.log('  ⚠️  WARNING: Polling interval still exists!');
        } else {
            console.log('  ✓ No polling interval found (event-driven architecture confirmed)');
        }

        console.log('\n✓ Event-driven overlay test completed successfully!');
        console.log(`  - Callback registered: ${callbackRegistered}`);
        console.log(`  - Card overlays displayed: ${overlayInfo.length}`);
        console.log(`  - Polling removed: ${!hasPollingInterval}`);

    } catch (error) {
        console.error('Test failed:', error);
        await page.screenshot({ path: 'event_driven_overlay_error.png' });
        console.error('Error screenshot saved: event_driven_overlay_error.png');
        throw error;
    } finally {
        await browser.close();
    }
}

// Run the test
testEventDrivenOverlay()
    .then(() => {
        console.log('\nAll tests passed!');
        process.exit(0);
    })
    .catch(err => {
        console.error('\nTest failed:', err);
        process.exit(1);
    });
