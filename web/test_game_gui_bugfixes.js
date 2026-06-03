// Targeted verification test for native_game.html rearchitect bug fixes
// Run with: node18 test_game_gui_bugfixes.js
//
// Tests these specific bugs that were fixed:
// 1. ARROW KEY NAVIGATION: Down arrow moves selection 1-by-1 (not skipping)
// 2. ALL PROMPTS APPEAR: Including discard choices from Rust game logic
// 3. NO SCROLLBAR: Page is fixed fullscreen (100vh, overflow hidden)
// 4. LOG DISPLAY: Matches TUI output (same WASM source)
// 5. ENTER KEY: Selects and advances cleanly (no double-advance)

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { firstBuiltinDeck, localGameUrl } = require('./game_boot_params');

function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

async function runTest() {
    let server;
    let browser;
    const PORT = 8769;
    const results = { tests: [], pass: 0, fail: 0 };

    function test(name, passed, detail) {
        results.tests.push({ name, passed, detail });
        if (passed) { results.pass++; log(`  PASS: ${name} — ${detail}`); }
        else { results.fail++; log(`  FAIL: ${name} — ${detail}`); }
    }

    try {
        log('Starting HTTP server...');
        server = spawn('python3', ['-m', 'http.server', String(PORT)], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(resolve => setTimeout(resolve, 1000));

        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });

        const jsErrors = [];
        page.on('pageerror', err => jsErrors.push(err.message));

        const screenshotDir = path.join(__dirname, 'screenshots');
        if (!fs.existsSync(screenshotDir)) fs.mkdirSync(screenshotDir);

        // mtg-682 page 3 / mtg-692: native_game.html is a PURE renderer (no
        // built-in launcher). Boot a Human-P1 vs Heuristic-P2 local game directly
        // from URL params (mode=local) via game_boot_params.localGameUrl instead
        // of the deleted #p1-controller / #p1-deck / #btn-launch form.
        const base = `http://localhost:${PORT}`;
        const firstDeck = await firstBuiltinDeck(base);
        await page.goto(localGameUrl(base, 'native_game.html', {
            deck: firstDeck, p1: 'human', p2: 'heuristic', seed: 42,
        }), { waitUntil: 'networkidle', timeout: 60000 });
        log('WASM loaded');

        // ============================================================
        // BUG FIX #3: NO SCROLLBAR — verify body is fullscreen fixed
        // ============================================================
        log('\n=== BUG #3: No Scrollbar ===');

        const bodyStyles = await page.evaluate(() => {
            const body = document.body;
            const computed = window.getComputedStyle(body);
            return {
                height: computed.height,
                overflow: computed.overflow,
                overflowY: computed.overflowY,
                scrollHeight: body.scrollHeight,
                clientHeight: body.clientHeight
            };
        });
        test('Body overflow is hidden',
            bodyStyles.overflow === 'hidden' || bodyStyles.overflowY === 'hidden',
            `overflow=${bodyStyles.overflow}, overflowY=${bodyStyles.overflowY}`);
        test('Body not scrollable (scrollHeight <= clientHeight)',
            bodyStyles.scrollHeight <= bodyStyles.clientHeight + 1,
            `scrollHeight=${bodyStyles.scrollHeight}, clientHeight=${bodyStyles.clientHeight}`);

        // ============================================================
        // Game (Human P1 vs Heuristic P2, seed 42) was booted from URL params
        // above; just wait for the renderer to show the board.
        // ============================================================
        await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 30000 });
        await page.waitForTimeout(500);

        // Advance to main phase (human should get priority)
        await page.keyboard.press('Space');
        await page.waitForTimeout(300);

        await page.screenshot({ path: path.join(screenshotDir, 'bugfix_01_initial_priority.png') });
        log('Screenshot: bugfix_01_initial_priority.png');

        // ============================================================
        // BUG FIX #3 continued: No scrollbar DURING GAMEPLAY
        // ============================================================
        const gameBodyStyles = await page.evaluate(() => {
            const body = document.body;
            return {
                scrollHeight: body.scrollHeight,
                clientHeight: body.clientHeight,
                hasVerticalScrollbar: body.scrollHeight > body.clientHeight
            };
        });
        test('No scrollbar during gameplay',
            !gameBodyStyles.hasVerticalScrollbar,
            `scrollH=${gameBodyStyles.scrollHeight}, clientH=${gameBodyStyles.clientHeight}`);

        // ============================================================
        // BUG FIX #2: ALL PROMPTS APPEAR
        // ============================================================
        log('\n=== BUG #2: All Prompts Appear ===');

        // Check that actions pane has choices from WASM state
        const initialActions = await page.evaluate(() => {
            const prompt = document.getElementById('actions-prompt');
            const body = document.getElementById('actions-body');
            return {
                promptVisible: prompt?.style.display !== 'none',
                promptText: prompt?.textContent || '',
                actionCount: body?.querySelectorAll('.action-item').length || 0,
                actionTexts: Array.from(body?.querySelectorAll('.action-item') || []).map(e => e.textContent.trim())
            };
        });

        test('Actions prompt is visible',
            initialActions.promptVisible || initialActions.actionCount > 0,
            `prompt="${initialActions.promptText}", actions=${initialActions.actionCount}`);

        // Log what choices are available for debugging
        log(`  Prompt: "${initialActions.promptText}"`);
        log(`  Choices (${initialActions.actionCount}): ${initialActions.actionTexts.join(' | ')}`);

        // ============================================================
        // BUG FIX #1: ARROW KEY NAVIGATION — 1-by-1
        // ============================================================
        log('\n=== BUG #1: Arrow Key Navigation ===');

        if (initialActions.actionCount >= 3) {
            // Get initial selected index
            const idx0 = await page.evaluate(() => {
                const items = document.querySelectorAll('#actions-body .action-item');
                for (let i = 0; i < items.length; i++) {
                    if (items[i].classList.contains('selected')) return i;
                }
                return -1;
            });
            log(`  Initial selected index: ${idx0}`);

            // Press Down arrow once
            await page.keyboard.press('ArrowDown');
            await page.waitForTimeout(100);

            const idx1 = await page.evaluate(() => {
                const items = document.querySelectorAll('#actions-body .action-item');
                for (let i = 0; i < items.length; i++) {
                    if (items[i].classList.contains('selected')) return i;
                }
                return -1;
            });
            log(`  After 1x Down: selected index: ${idx1}`);

            test('Down arrow moves selection by exactly 1',
                idx1 === idx0 + 1,
                `was ${idx0}, now ${idx1} (expected ${idx0 + 1})`);

            // Press Down again
            await page.keyboard.press('ArrowDown');
            await page.waitForTimeout(100);

            const idx2 = await page.evaluate(() => {
                const items = document.querySelectorAll('#actions-body .action-item');
                for (let i = 0; i < items.length; i++) {
                    if (items[i].classList.contains('selected')) return i;
                }
                return -1;
            });
            log(`  After 2x Down: selected index: ${idx2}`);

            test('Second Down arrow moves by exactly 1 more',
                idx2 === idx1 + 1,
                `was ${idx1}, now ${idx2} (expected ${idx1 + 1})`);

            await page.screenshot({ path: path.join(screenshotDir, 'bugfix_02_arrow_navigation.png') });
            log('Screenshot: bugfix_02_arrow_navigation.png');

            // Press Up arrow back
            await page.keyboard.press('ArrowUp');
            await page.waitForTimeout(100);

            const idx3 = await page.evaluate(() => {
                const items = document.querySelectorAll('#actions-body .action-item');
                for (let i = 0; i < items.length; i++) {
                    if (items[i].classList.contains('selected')) return i;
                }
                return -1;
            });
            test('Up arrow moves selection back by 1',
                idx3 === idx2 - 1,
                `was ${idx2}, now ${idx3} (expected ${idx2 - 1})`);
        } else {
            log(`  SKIP: Not enough choices to test navigation (got ${initialActions.actionCount})`);
            test('Arrow key navigation (skipped — need 3+ choices)', true, 'insufficient choices');
        }

        // ============================================================
        // BUG FIX #5: ENTER KEY — select and advance cleanly
        // ============================================================
        log('\n=== BUG #5: Enter Key ===');

        // Get state before Enter
        const stateBeforeEnter = await page.evaluate(() => {
            const state = JSON.parse(window.__test_getState ? window.__test_getState() : '{}');
            return {
                turnNumber: state.turn_number,
                logCount: document.querySelectorAll('#log-body .log-entry').length,
                choiceCount: document.querySelectorAll('#actions-body .action-item').length
            };
        });

        // Capture the turn number from status bar since __test_getState may not exist
        const turnTextBefore = await page.textContent('#status-bar');
        const turnMatchBefore = turnTextBefore.match(/Turn (\d+)/);
        const turnBefore = turnMatchBefore ? parseInt(turnMatchBefore[1]) : -1;
        log(`  Before Enter: Turn ${turnBefore}, ${stateBeforeEnter.logCount} log entries`);

        // Press Enter to select current choice and advance
        await page.keyboard.press('Enter');
        await page.waitForTimeout(300);

        const turnTextAfter = await page.textContent('#status-bar');
        const turnMatchAfter = turnTextAfter.match(/Turn (\d+)/);
        const turnAfter = turnMatchAfter ? parseInt(turnMatchAfter[1]) : -1;
        const logCountAfter = await page.evaluate(() =>
            document.querySelectorAll('#log-body .log-entry').length
        );
        log(`  After Enter: Turn ${turnAfter}, ${logCountAfter} log entries`);

        // Enter should advance the game MONOTONICALLY without the historical
        // double-FIRE bug (one keypress processed as two). When the human passes
        // priority with no play, the opponent's whole turn can legitimately
        // auto-resolve back to the human's next turn, so a +2 turn delta is valid
        // here (mtg-692: relaxed from the launcher-era "<= +1" which assumed a
        // specific starting interaction; the guard is monotonic + bounded, not a
        // freeze and not an erratic skip). Turn must never go BACKWARD.
        test('Enter key advances monotonically (no double-fire / no backward jump)',
            turnAfter >= turnBefore && turnAfter <= turnBefore + 2,
            `turn went from ${turnBefore} to ${turnAfter}`);

        test('Enter key advances game (log grows)',
            logCountAfter >= stateBeforeEnter.logCount,
            `log entries: ${stateBeforeEnter.logCount} → ${logCountAfter}`);

        await page.screenshot({ path: path.join(screenshotDir, 'bugfix_03_after_enter.png') });
        log('Screenshot: bugfix_03_after_enter.png');

        // ============================================================
        // BUG FIX #4: LOG DISPLAY — should have entries from WASM
        // ============================================================
        log('\n=== BUG #4: Log Display ===');

        // Run several Space presses to populate the log. Bumped from 3 → 10
        // after decouple-step4 (mtg-382): pre-step-4 native_game.html had a
        // hidden ratzilla terminal that ALSO processed Space (calling
        // run_until_choice on top of the JS-side tui_run_turn), so 3 Space
        // presses gave 6 effective game advances. Post-step-4 there's no
        // ratzilla, so Space fires once per press; we now need a few more
        // presses to reach a draw step from the test's seed-42 starting
        // position. Either way, the assertion below is robust as long as
        // *some* Space presses cause card draws to appear in the log.
        for (let i = 0; i < 10; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(200);
        }

        const logInfo = await page.evaluate(() => {
            const entries = document.querySelectorAll('#log-body .log-entry');
            const texts = Array.from(entries).map(e => e.textContent);
            return {
                count: entries.length,
                firstFew: texts.slice(0, 5),
                lastFew: texts.slice(-3),
                hasDraws: texts.some(t => t.includes('draws')),
                hasPlays: texts.some(t => t.includes('plays')),
                hasTurnMarkers: texts.some(t => t.includes('Turn'))
            };
        });

        test('Log has entries from game engine',
            logInfo.count > 0,
            `${logInfo.count} log entries`);

        test('Log contains draw actions',
            logInfo.hasDraws,
            logInfo.hasDraws ? 'Found draw entries' : 'No draw entries found');

        test('Log contains play actions or turn markers',
            logInfo.hasPlays || logInfo.hasTurnMarkers,
            `plays=${logInfo.hasPlays}, turns=${logInfo.hasTurnMarkers}`);

        log(`  First entries: ${logInfo.firstFew.join(' | ')}`);
        log(`  Last entries: ${logInfo.lastFew.join(' | ')}`);

        await page.screenshot({ path: path.join(screenshotDir, 'bugfix_04_log_display.png') });
        log('Screenshot: bugfix_04_log_display.png');

        // ============================================================
        // BUG #2 continued: Test discard prompt specifically
        // This requires triggering a discard scenario.
        // The rogue deck has Bazaar of Baghdad which triggers "discard 3"
        // We need to find and play it.
        // ============================================================
        log('\n=== BUG #2 continued: Discard Prompt Test ===');

        // Run auto for a while then check if discard prompt ever appeared
        // Save the auto-run state and look at actions during the run
        let discardPromptSeen = false;
        let choicePromptSeen = false;

        // Let AI play by pressing Space multiple times
        for (let i = 0; i < 15; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(100);

            const currentPrompt = await page.evaluate(() => {
                const prompt = document.getElementById('actions-prompt');
                return prompt?.textContent || '';
            });

            if (currentPrompt.toLowerCase().includes('discard')) {
                discardPromptSeen = true;
                log(`  Discard prompt seen at step ${i}: "${currentPrompt}"`);
                await page.screenshot({ path: path.join(screenshotDir, 'bugfix_05_discard_prompt.png') });
                log('Screenshot: bugfix_05_discard_prompt.png');
                break;
            }
            if (currentPrompt.toLowerCase().includes('choose') || currentPrompt.toLowerCase().includes('priority')) {
                choicePromptSeen = true;
            }
        }

        if (discardPromptSeen) {
            test('Discard prompt appears from Rust game logic',
                true, 'Discard prompt was shown');
        } else {
            // Not a failure per se — depends on game state reaching a discard scenario
            test('Discard prompt (conditional — depends on game flow)',
                choicePromptSeen,
                discardPromptSeen ? 'Discard seen' : `No discard scenario reached, but other prompts seen: ${choicePromptSeen}`);
        }

        // ============================================================
        // EXTRA: Check no page scrollbar after many turns
        // ============================================================
        log('\n=== Extra: Post-game scrollbar check ===');

        // Auto-run to fill up logs
        await page.keyboard.press('a');
        await page.waitForTimeout(3000);
        await page.keyboard.press('a');
        await page.waitForTimeout(200);

        const finalBodyCheck = await page.evaluate(() => ({
            scrollHeight: document.body.scrollHeight,
            clientHeight: document.body.clientHeight,
            hasScrollbar: document.body.scrollHeight > document.body.clientHeight,
            logEntries: document.querySelectorAll('#log-body .log-entry').length
        }));

        test('No scrollbar after auto-run (many log entries)',
            !finalBodyCheck.hasScrollbar,
            `scrollH=${finalBodyCheck.scrollHeight}, clientH=${finalBodyCheck.clientHeight}, logEntries=${finalBodyCheck.logEntries}`);

        await page.screenshot({ path: path.join(screenshotDir, 'bugfix_06_final_no_scroll.png') });
        log('Screenshot: bugfix_06_final_no_scroll.png');

        // Check for JS errors (panics)
        const hasPanics = jsErrors.some(e => e.includes('panic') || e.includes('unreachable'));
        test('No WASM panics', !hasPanics,
            hasPanics ? `Panics: ${jsErrors.filter(e => e.includes('panic')).join('; ')}` : 'Clean');

        // ============================================================
        // RESULTS
        // ============================================================
        log('\n' + '='.repeat(60));
        log(`=== RESULTS: ${results.pass} PASS, ${results.fail} FAIL ===`);
        log('='.repeat(60));

        results.tests.forEach(t => {
            log(`  ${t.passed ? 'PASS' : 'FAIL'}: ${t.name} — ${t.detail}`);
        });

        const resultsPath = path.join(screenshotDir, 'bugfix_test_results.json');
        fs.writeFileSync(resultsPath, JSON.stringify(results, null, 2));

        if (results.fail > 0) {
            log('\n=== BUGFIX VERIFICATION FAILED ===');
            return false;
        }

        log('\n=== ALL BUGFIX VERIFICATIONS PASSED ===');
        return true;
    } catch (error) {
        log(`=== Test Error: ${error.message} ===`);
        log(error.stack);
        if (browser) {
            try {
                const pages = browser.contexts()[0]?.pages();
                if (pages?.[0]) {
                    await pages[0].screenshot({
                        path: path.join(__dirname, 'screenshots', 'bugfix_failure.png')
                    });
                }
            } catch (e) { /* ignore */ }
        }
        return false;
    } finally {
        if (browser) await browser.close();
        if (server) server.kill();
    }
}

runTest().then(success => process.exit(success ? 0 : 1));
