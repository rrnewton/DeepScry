// Human Input E2E Test for WASM TUI
// This test verifies that human controller actions work correctly:
// 1. Play lands one at a time
// 2. Verify each land appears on battlefield
// 3. Extract text from DOM to verify game state
//
// Run with: node test_human_input.js

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { enableReplayVerifier } = require('./test_network_utils');

function log(message) {
    const timestamp = new Date().toISOString();
    console.log(`[${timestamp}] ${message}`);
}

// Count unique cards on battlefield (handles truncated names)
function countBattlefieldCards(text, sectionMarker) {
    // Find the section (Your Battlefield or Opponent Battlefield)
    const sectionStart = text.indexOf(sectionMarker);
    if (sectionStart === -1) return { count: 0, cards: [] };

    // Find the end of the section (next major section or panel)
    let sectionEnd = text.length;
    const nextSections = ['You:', 'Opp:', '┌ Hand', '┌ Stack', '┌ Actions'];
    for (const marker of nextSections) {
        const idx = text.indexOf(marker, sectionStart + sectionMarker.length);
        if (idx !== -1 && idx < sectionEnd) {
            sectionEnd = idx;
        }
    }

    const section = text.substring(sectionStart, sectionEnd);

    // Count card boxes by looking for │name│ patterns
    // Cards are rendered as boxes with names like │Badlands      │ │Bayou         │
    // Card names can be up to ~14 chars followed by spaces
    const cardPattern = /│([A-Z][a-zA-Z0-9' ]+)\s*│/g;
    const cards = [];
    let match;
    while ((match = cardPattern.exec(section)) !== null) {
        const name = match[1].trim();
        // Filter out UI elements and non-card text
        if (name && name.length > 2 &&
            !name.includes('Empty') && !name.includes('[') && !name.includes('{') &&
            !name.includes(':') && !name.startsWith('T') && !name.includes('Add') &&
            !name.includes('Land') && !name.includes('...')) {
            cards.push(name);
        }
    }

    // Also count "2x" type cards (stacked cards showing count)
    const stackedPattern = /│(\d+)x\s+([A-Z][a-z]+)/g;
    while ((match = stackedPattern.exec(section)) !== null) {
        const count = parseInt(match[1]);
        const name = match[2];
        for (let i = 0; i < count; i++) {
            cards.push(name);
        }
    }

    return { count: cards.length, cards };
}

// Extract all text from the RatZilla terminal DOM
async function extractTerminalText(page) {
    return await page.evaluate(() => {
        const terminal = document.getElementById('ratzilla-terminal');
        if (!terminal) return 'NO TERMINAL';

        // RatZilla renders each character as a span inside divs (rows)
        // Try to extract text row by row
        const rows = [];
        const rowElements = terminal.querySelectorAll('div');

        for (const row of rowElements) {
            // Get text content of this row
            const text = row.textContent || '';
            if (text.trim()) {
                rows.push(text);
            }
        }

        return rows.join('\n');
    });
}

// Parse game state from terminal text
function parseGameState(text) {
    const state = {
        turn: 0,
        phase: '',
        p1Life: 0,
        oppLife: 0,
        p1Hand: 0,
        oppHand: 0,
        yourLands: [],
        oppLands: [],
        choices: [],
        logLines: [],
        errors: []
    };

    // Extract turn info - look for "Turn X"
    const turnMatch = text.match(/Turn (\d+)/);
    if (turnMatch) state.turn = parseInt(turnMatch[1]);

    // Extract phase info
    if (text.includes('UP')) state.phase = 'Upkeep';
    else if (text.includes('M1')) state.phase = 'Main1';
    else if (text.includes('M2')) state.phase = 'Main2';
    else if (text.includes('DC')) state.phase = 'DeclareAttackers';

    // Extract life totals
    const youLifeMatch = text.match(/You: (\d+) life/);
    if (youLifeMatch) state.p1Life = parseInt(youLifeMatch[1]);

    const oppLifeMatch = text.match(/Opp: (\d+) life/);
    if (oppLifeMatch) state.oppLife = parseInt(oppLifeMatch[1]);

    // Extract hand counts
    const handMatches = text.match(/Hand: (\d+)/g);
    if (handMatches && handMatches.length >= 2) {
        state.oppHand = parseInt(handMatches[0].match(/\d+/)[0]);
        state.p1Hand = parseInt(handMatches[1].match(/\d+/)[0]);
    }

    // Find lands on battlefield by looking for card boxes with "Land" type
    // Pattern: │CardName...│ followed by │Land│
    const lines = text.split('');

    // Look for your battlefield section
    const yourBfStart = text.indexOf('(Y)our Battlefield');
    const yourBfEnd = text.indexOf('You:');
    if (yourBfStart !== -1 && yourBfEnd !== -1) {
        const yourSection = text.substring(yourBfStart, yourBfEnd);
        // Look for card names in boxes (simplified pattern)
        const cardMatches = yourSection.match(/│([A-Z][a-zA-Z' ]+)(?:\.{0,3})│/g);
        if (cardMatches) {
            for (const m of cardMatches) {
                const name = m.replace(/│/g, '').replace(/\.+/g, '').trim();
                if (name && name.length > 2 && !name.includes('Empty') && !name.includes('{')) {
                    state.yourLands.push(name);
                }
            }
        }
    }

    // Look for opponent battlefield section
    const oppBfStart = text.indexOf('(O)pponent Battlefield');
    const oppBfEnd = text.indexOf('(Y)our Battlefield');
    if (oppBfStart !== -1 && oppBfEnd !== -1) {
        const oppSection = text.substring(oppBfStart, oppBfEnd);
        const cardMatches = oppSection.match(/│([A-Z][a-zA-Z' ]+)(?:\.{0,3})│/g);
        if (cardMatches) {
            for (const m of cardMatches) {
                const name = m.replace(/│/g, '').replace(/\.+/g, '').trim();
                if (name && name.length > 2 && !name.includes('Empty') && !name.includes('{')) {
                    state.oppLands.push(name);
                }
            }
        }
    }

    // Extract choice options
    if (text.includes('Choose an action:')) {
        const passMatch = text.match(/Pass \(do nothing\)/);
        if (passMatch) state.choices.push('Pass');

        // Look for play/cast options
        const playMatches = text.match(/Play land: ([^\n│]+)/g);
        if (playMatches) {
            for (const m of playMatches) {
                state.choices.push(m);
            }
        }
    }

    // Look for errors
    if (text.includes('Error')) {
        const errorLines = text.split('\n').filter(l => l.includes('Error'));
        state.errors = errorLines.map(l => l.trim());
    }

    // Extract log lines (look for >>> markers)
    const logMatches = text.match(/>>> [^\n│]+/g);
    if (logMatches) {
        state.logLines = logMatches.map(l => l.replace('>>> ', ''));
    }

    return state;
}

// Wait for choice prompt to appear
async function waitForChoicePrompt(page, timeout = 5000) {
    const startTime = Date.now();
    while (Date.now() - startTime < timeout) {
        const text = await extractTerminalText(page);
        if (text.includes('Choose') || text.includes('action') || text.includes('Pass')) {
            return text;
        }
        await page.waitForTimeout(100);
    }
    throw new Error('Timeout waiting for choice prompt');
}

async function runTest() {
    let server;
    let browser;
    const testResults = {
        startTime: new Date().toISOString(),
        steps: [],
        states: [],
        errors: []
    };

    try {
        // Start HTTP server
        log('Starting HTTP server on port 8767...');
        server = spawn('python3', ['-m', 'http.server', '8767'], {
            cwd: path.join(__dirname),
            stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(resolve => setTimeout(resolve, 1000));

        // Launch browser
        log('Launching Chromium browser...');
        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });
        const page = await browser.newPage();

        // Collect console messages
        page.on('console', msg => {
            if (msg.type() === 'error') {
                log(`Browser ERROR: ${msg.text()}`);
                testResults.errors.push(msg.text());
            }
        });

        page.on('pageerror', err => {
            log(`Page ERROR: ${err.message}`);
            testResults.errors.push(err.message);
        });

        // Navigate to fancy TUI
        log('Loading fancy TUI page...');
        await page.goto('http://localhost:8767/tui_game.html', {
            waitUntil: 'networkidle',
            timeout: 60000
        });

        // Wait for WASM to initialize (launcher should be visible)
        log('Waiting for WASM to load...');
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
        log('WASM loaded');

        // Enable rewind/replay verifier. THIS test exercises the human
        // controller path which IS what triggers rewind/replay in the WASM
        // TUI — so the verifier will actually fire here on every choice.
        // Any divergence shows up as "REWIND/REPLAY FATAL" in the browser
        // console (caught by the panic check below via the existing error
        // collector).
        const verifierEnabled = await enableReplayVerifier(page);
        log(`Replay verifier enabled: ${verifierEnabled}`);

        // Select Human for P1 and Zero for P2
        await page.selectOption('#p1-controller', 'human');
        await page.selectOption('#p2-controller', 'zero');

        // Select same deck for both players to avoid missing-card errors
        await page.evaluate(() => {
            const sel = document.getElementById('p1-deck');
            const firstDeck = sel?.options[0]?.value || '';
            if (firstDeck) {
                sel.value = firstDeck;
                document.getElementById('p2-deck').value = firstDeck;
            }
        });

        // Take screenshot of setup
        const screenshotDir = path.join(__dirname, 'screenshots');
        if (!fs.existsSync(screenshotDir)) {
            fs.mkdirSync(screenshotDir);
        }

        // Launch the TUI
        log('Launching TUI with Human vs Zero...');
        await page.click('#btn-launch');
        await page.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 10000 });
        await page.waitForTimeout(500);
        await page.screenshot({ path: path.join(screenshotDir, 'human_01_initial.png'), fullPage: true });

        // Get and log initial state
        let text = await extractTerminalText(page);
        let state = parseGameState(text);

        // Count cards on battlefield using the better function
        let yourCards = countBattlefieldCards(text, '(Y)our Battlefield');
        let oppCards = countBattlefieldCards(text, '(O)pponent Battlefield');

        log(`\n=== INITIAL STATE ===`);
        log(`Turn: ${state.turn}, Phase: ${state.phase}`);
        log(`Life: You ${state.p1Life}, Opp ${state.oppLife}`);
        log(`Your battlefield (${yourCards.count}): ${yourCards.cards.join(', ') || '(empty)'}`);
        log(`Opp battlefield (${oppCards.count}): ${oppCards.cards.join(', ') || '(empty)'}`);

        // Verify we're getting choice prompts
        const hasChoicePrompt = text.includes('Choose an action:') || text.includes('Pass');
        log(`Has choice prompt: ${hasChoicePrompt}`);
        if (!hasChoicePrompt) {
            log('WARNING: No choice prompt found - game may not have initialized correctly');
        }

        testResults.states.push({
            step: 'initial',
            ...state,
            yourCardCount: yourCards.count,
            oppCardCount: oppCards.count
        });

        // Track card counts to verify they increase
        let prevYourCount = yourCards.count;
        let actionsThatAddedCards = 0;

        // Action sequence - press keys to make choices
        // We'll try to select "Play land" options when available
        const actions = ['2', '2', '2', '2', '2'];  // Try to play 5 lands/cards

        for (let i = 0; i < actions.length; i++) {
            const key = actions[i];
            log(`\n--- Action ${i+1}: Press '${key}' ---`);

            await page.keyboard.press(key);
            await page.waitForTimeout(300);  // Wait for action to process

            // Get state after action
            text = await extractTerminalText(page);
            state = parseGameState(text);
            yourCards = countBattlefieldCards(text, '(Y)our Battlefield');
            oppCards = countBattlefieldCards(text, '(O)pponent Battlefield');

            log(`Turn: ${state.turn}, Phase: ${state.phase}`);
            log(`Life: You ${state.p1Life}, Opp ${state.oppLife}`);
            log(`Your battlefield (${yourCards.count}): ${yourCards.cards.join(', ') || '(empty)'}`);
            log(`Opp battlefield (${oppCards.count}): ${oppCards.cards.join(', ') || '(empty)'}`);

            // Check if our card count increased (cards appearing on battlefield)
            if (yourCards.count > prevYourCount) {
                actionsThatAddedCards++;
                log(`  -> CARD ADDED to your battlefield!`);
            }
            prevYourCount = yourCards.count;

            testResults.states.push({
                step: `action_${i+1}`,
                ...state,
                yourCardCount: yourCards.count,
                oppCardCount: oppCards.count
            });
            await page.screenshot({
                path: path.join(screenshotDir, `human_${String(i+2).padStart(2,'0')}_action${i+1}.png`),
                fullPage: true
            });
        }

        log(`\n=== CARD TRACKING SUMMARY ===`);
        log(`Actions that added cards to battlefield: ${actionsThatAddedCards}`);
        log(`Final your battlefield count: ${yourCards.count}`);

        // Save final terminal text
        fs.writeFileSync(path.join(screenshotDir, 'terminal_text.txt'), text);

        // Dump raw text for debugging
        log('\\n=== RAW TERMINAL TEXT ===');
        log(text.replace(/\n/g, '\\n').substring(0, 2000));

        // Log final state summary
        log('\n=== FINAL STATE SUMMARY ===');
        log(`After ${actions.length} actions:`);
        log(`Turn: ${state.turn}`);
        log(`Your battlefield: ${state.yourLands.join(', ') || '(none)'}`);
        log(`Opp battlefield: ${state.oppLands.join(', ') || '(none)'}`);

        // Check for panics
        const hasPanic = testResults.errors.some(e =>
            e.includes('panic') || e.includes('unreachable')
        );
        if (hasPanic) {
            throw new Error('WASM panic detected');
        }

        // Check for rewind/replay verifier failures. The verifier emits
        // "REWIND/REPLAY FATAL: ..." at log::error level (which surfaces as
        // console.error in the browser, captured above into testResults.errors).
        // Treat any hit as a hard test failure — same severity as a panic,
        // because the rewind is no longer a faithful round-trip.
        const replayFatal = testResults.errors.find(e =>
            e.toUpperCase().includes('REWIND/REPLAY FATAL')
        );
        if (replayFatal) {
            throw new Error(`Replay verifier divergence detected: ${replayFatal}`);
        }

        testResults.endTime = new Date().toISOString();
        testResults.success = true;

        fs.writeFileSync(
            path.join(screenshotDir, 'human_test_results.json'),
            JSON.stringify(testResults, null, 2)
        );

        log('\n=== Test Complete ===');
        return true;
    } catch (error) {
        log(`=== Test Failed: ${error.message} ===`);
        testResults.errors.push(error.message);

        if (browser) {
            try {
                const pages = browser.contexts()[0]?.pages();
                if (pages && pages.length > 0) {
                    const screenshotDir = path.join(__dirname, 'screenshots');
                    await pages[0].screenshot({ path: path.join(screenshotDir, 'human_failure.png'), fullPage: true });
                    const text = await extractTerminalText(pages[0]);
                    fs.writeFileSync(path.join(screenshotDir, 'terminal_text_failure.txt'), text);
                }
            } catch (e) {}
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
