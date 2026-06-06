// Deep-dive E2E test for native_game.html Native GUI
// Tests: deck combos, human-vs-AI, edge cases, tui_game.html comparison
// Run with: node18 test_game_gui_deep.js

const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const fs = require('fs');
const { listBuiltinDecks, firstBuiltinDeck, pickBuiltinDeck, localGameUrl } = require('./game_boot_params');

// mtg-682 page 3 / mtg-692: the game pages are PURE renderers (no built-in
// launcher / deck-collection dropdown). These regexes approximate the OLD
// launcher's DECK_COLLECTIONS filters (now living in launcher.html) just enough
// to pick a representative built-in deck per "collection" for this non-gate
// rendering test — it does not need the exact partition, only two distinct
// loadable decks per label.
const COLLECTION_DECK_RE = {
    old_school:    /^\d\d_|classic|erhnam|ponza|stasis|zoo|monolith/,
    booster_draft: /avatar|spiderman/,
    championship_2025: /manfield|shibata|davis|henry/,
};

function log(msg) { console.log(`[${new Date().toISOString()}] ${msg}`); }

async function runTest() {
    let server, browser;
    const PORT = 8770;
    const findings = [];
    function finding(sev, msg) { findings.push({ sev, msg }); log(`[${sev}] ${msg}`); }

    try {
        server = spawn('python3', ['-m', 'http.server', String(PORT)], {
            cwd: path.join(__dirname), stdio: ['ignore', 'pipe', 'pipe']
        });
        await new Promise(r => setTimeout(r, 1000));

        browser = await chromium.launch({
            headless: true,
            args: ['--no-sandbox', '--enable-unsafe-swiftshader']
        });

        const ssDir = path.join(__dirname, 'screenshots');
        if (!fs.existsSync(ssDir)) fs.mkdirSync(ssDir);

        const jsErrors = [];

        // ================================================================
        // SECTION 1: Different deck combinations
        // ================================================================
        log('\n========== SECTION 1: Deck Combinations ==========');

        const deckTests = [
            { name: 'old_school', collection: 'old_school' },
            { name: 'booster_draft', collection: 'booster_draft' },
            { name: 'championship_2025', collection: 'championship_2025' },
        ];

        for (const dt of deckTests) {
            const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
            page.on('pageerror', err => jsErrors.push(err.message));

            // mtg-692: boot native_game.html from URL params (mode=local) — the
            // pure renderer has no launcher/collection dropdown. Pick two distinct
            // representative built-in decks for this collection label.
            const base = `http://localhost:${PORT}`;
            const re = COLLECTION_DECK_RE[dt.collection] || /.*/;
            const p1Deck = await pickBuiltinDeck(base, re);
            // Pick a DIFFERENT deck for P2 (asymmetric, as the original first-vs-last
            // launcher selection was): first built-in that isn't p1Deck.
            const allDecks = await listBuiltinDecks(base);
            const p2Deck = allDecks.find(d => re.test(d) && d !== p1Deck)
                || allDecks.find(d => d !== p1Deck)
                || p1Deck;
            finding('OK', `${dt.name}: booting ${p1Deck} vs ${p2Deck} (built-in, param boot)`);

            try {
                await page.goto(localGameUrl(base, 'native_game.html', {
                    p1Deck, p2Deck, p1: 'heuristic', p2: 'heuristic', seed: 99,
                }), { waitUntil: 'load', timeout: 30000 });
                await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 15000 });
                await page.waitForTimeout(300);

                // Auto-run 10 turns
                for (let i = 0; i < 10; i++) {
                    await page.keyboard.press('Space');
                    await page.waitForTimeout(100);
                }

                const state = await page.evaluate(() => {
                    const bar = document.getElementById('status-bar');
                    return {
                        statusText: bar?.textContent || '',
                        logCount: document.querySelectorAll('#log-body .log-entry').length,
                        playerField: document.querySelectorAll('#player-field-cards .card').length,
                        oppField: document.querySelectorAll('#opp-field-cards .card').length,
                        handCards: document.querySelectorAll('#hand-cards .card').length,
                        gameOver: bar?.textContent?.includes('GAME OVER') || false
                    };
                });

                finding('OK', `${dt.name} (${p1Deck} vs ${p2Deck}): ${state.statusText.trim().substring(0,60)}, log=${state.logCount}, field=${state.playerField}+${state.oppField}, hand=${state.handCards}`);

                await page.screenshot({ path: path.join(ssDir, `deep_deck_${dt.name}.png`) });

                // Check: no NON-creature land shows P/T. A land that has been
                // animated into a creature (CR 208.3, common in the avatar decks)
                // legitimately shows P/T, so we only flag lands that are NOT also
                // creatures (the `.creature` class marks animated lands).
                const landPT = await page.evaluate(() => {
                    const cards = document.querySelectorAll('#player-field-cards .card, #opp-field-cards .card');
                    const issues = [];
                    cards.forEach(c => {
                        const isLand = c.classList.contains('land');
                        const isCreature = c.classList.contains('creature');
                        const hasPT = c.querySelector('.card-pt');
                        if (isLand && !isCreature && hasPT) issues.push(c.querySelector('.card-name')?.textContent);
                    });
                    return issues;
                });
                if (landPT.length > 0) {
                    finding('FAIL', `${dt.name}: Non-creature lands with P/T: ${landPT.join(', ')}`);
                } else {
                    finding('OK', `${dt.name}: No non-creature land shows P/T`);
                }

            } catch (e) {
                finding('FAIL', `${dt.name}: Game failed — ${e.message}`);
                await page.screenshot({ path: path.join(ssDir, `deep_deck_${dt.name}_fail.png`) });
            }

            await page.close();
        }

        // ================================================================
        // SECTION 2: Human-vs-AI interaction
        // ================================================================
        log('\n========== SECTION 2: Human vs AI Interaction ==========');
        {
            const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
            page.on('pageerror', err => jsErrors.push(err.message));

            // mtg-692: boot Human-vs-Heuristic local game from URL params.
            const dbase = `http://localhost:${PORT}`;
            const hDeck = await firstBuiltinDeck(dbase);
            await page.goto(localGameUrl(dbase, 'native_game.html', {
                deck: hDeck, p1: 'human', p2: 'heuristic', seed: 42,
            }), { waitUntil: 'load', timeout: 30000 });
            await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 15000 });
            await page.waitForTimeout(300);

            // Advance to get priority
            await page.keyboard.press('Space');
            await page.waitForTimeout(300);

            await page.screenshot({ path: path.join(ssDir, 'deep_human_01_priority.png') });

            // Test: prompt should say "Priority P1: Choose action" or similar
            const prompt1 = await page.evaluate(() =>
                document.getElementById('actions-prompt')?.textContent || ''
            );
            finding(prompt1 ? 'OK' : 'FAIL', `Human prompt: "${prompt1}"`);

            // Test: arrow navigation 1-by-1
            const choices = await page.evaluate(() =>
                document.querySelectorAll('#actions-body .action-item').length
            );
            finding('OK', `Human choices available: ${choices}`);

            if (choices >= 2) {
                // Navigate down to second choice
                await page.keyboard.press('ArrowDown');
                await page.waitForTimeout(100);
                const sel1 = await page.evaluate(() => {
                    const items = document.querySelectorAll('#actions-body .action-item');
                    for (let i = 0; i < items.length; i++) {
                        if (items[i].classList.contains('selected')) return i;
                    }
                    return -1;
                });
                finding(sel1 === 1 ? 'OK' : 'FAIL', `Arrow down: selected index=${sel1} (expected 1)`);

                // Select with Enter (play something)
                await page.keyboard.press('Enter');
                await page.waitForTimeout(500);

                await page.screenshot({ path: path.join(ssDir, 'deep_human_02_after_enter.png') });

                const logAfter = await page.evaluate(() =>
                    document.querySelectorAll('#log-body .log-entry').length
                );
                finding(logAfter > 0 ? 'OK' : 'WARN', `After Enter: ${logAfter} log entries`);

                // Check if battlefield has a card now
                const fieldAfter = await page.evaluate(() =>
                    document.querySelectorAll('#player-field-cards .card').length
                );
                finding('OK', `After playing: ${fieldAfter} cards on your battlefield`);
            }

            // Continue game with Space presses, check for discard prompt
            let discardSeen = false;
            for (let i = 0; i < 20; i++) {
                await page.keyboard.press('Space');
                await page.waitForTimeout(100);

                const p = await page.evaluate(() =>
                    document.getElementById('actions-prompt')?.textContent || ''
                );
                if (p.toLowerCase().includes('discard')) {
                    discardSeen = true;
                    finding('OK', `Discard prompt appeared: "${p}"`);
                    await page.screenshot({ path: path.join(ssDir, 'deep_human_03_discard.png') });

                    // Verify discard choices are listed
                    const discardChoices = await page.evaluate(() =>
                        Array.from(document.querySelectorAll('#actions-body .action-item'))
                            .map(e => e.textContent.trim())
                    );
                    finding('OK', `Discard choices: ${discardChoices.length} — ${discardChoices.slice(0,3).join(', ')}...`);
                    break;
                }
            }
            if (!discardSeen) {
                finding('INFO', 'Discard prompt not reached in this game flow (game-dependent)');
            }

            // Test number key quick-select
            await page.keyboard.press('Space');
            await page.waitForTimeout(200);
            const hasChoicesForNum = await page.evaluate(() =>
                document.querySelectorAll('#actions-body .action-item').length
            );
            if (hasChoicesForNum >= 2) {
                await page.keyboard.press('2');
                await page.waitForTimeout(300);
                finding('OK', 'Number key "2" pressed — game advanced');
            }

            await page.screenshot({ path: path.join(ssDir, 'deep_human_04_midgame.png') });
            await page.close();
        }

        // ================================================================
        // SECTION 3: tui_game.html vs native_game.html comparison (same seed)
        // ================================================================
        log('\n========== SECTION 3: tui_game.html vs native_game.html Comparison ==========');
        {
            // Close and reopen browser to avoid connection pool issues
            await browser.close();
            // Restart HTTP server (python http.server can stall after many requests)
            server.kill();
            await new Promise(r => setTimeout(r, 500));
            server = spawn('python3', ['-m', 'http.server', String(PORT)], {
                cwd: path.join(__dirname), stdio: ['ignore', 'pipe', 'pipe']
            });
            await new Promise(r => setTimeout(r, 1000));
            browser = await chromium.launch({
                headless: true,
                args: ['--no-sandbox', '--enable-unsafe-swiftshader']
            });

            // Run native_game.html with seed 77, AI vs AI, 5 turns
            // mtg-692: boot from URL params (same seed 77, AI vs AI).
            const cmpBase = `http://localhost:${PORT}`;
            const cmpDeck = await firstBuiltinDeck(cmpBase);
            const gamePage = await browser.newPage({ viewport: { width: 1280, height: 720 } });
            await gamePage.goto(localGameUrl(cmpBase, 'native_game.html', {
                deck: cmpDeck, p1: 'heuristic', p2: 'heuristic', seed: 77,
            }), { waitUntil: 'load', timeout: 30000 });
            await gamePage.waitForSelector('#game-area.show', { state: 'visible', timeout: 15000 });
            await gamePage.waitForTimeout(300);

            for (let i = 0; i < 5; i++) {
                await gamePage.keyboard.press('Space');
                await gamePage.waitForTimeout(200);
            }

            const gameState = await gamePage.evaluate(() => ({
                logs: Array.from(document.querySelectorAll('#log-body .log-entry')).map(e => e.textContent),
                playerLife: document.querySelector('#player-info-body .life-total')?.textContent || '',
                oppLife: document.querySelector('#opp-info-body .life-total')?.textContent || '',
                turn: document.getElementById('status-bar')?.textContent?.match(/Turn (\d+)/)?.[1] || '?',
                handCount: document.querySelectorAll('#hand-cards .card').length,
                playerField: Array.from(document.querySelectorAll('#player-field-cards .card')).map(c =>
                    c.querySelector('.card-name')?.textContent || '?'
                ),
                oppField: Array.from(document.querySelectorAll('#opp-field-cards .card')).map(c =>
                    c.querySelector('.card-name')?.textContent || '?'
                )
            }));

            await gamePage.screenshot({ path: path.join(ssDir, 'deep_compare_game.png') });
            await gamePage.close();

            // Now run tui_game.html with same seed
            // mtg-692: boot tui_game.html from URL params (same deck + seed 77).
            const fancyPage = await browser.newPage({ viewport: { width: 1280, height: 720 } });
            await fancyPage.goto(localGameUrl(cmpBase, 'tui_game.html', {
                deck: cmpDeck, p1: 'heuristic', p2: 'heuristic', seed: 77,
            }), { waitUntil: 'load', timeout: 30000 });
            await fancyPage.waitForSelector('#ratzilla-terminal', { state: 'visible', timeout: 10000 });
            await fancyPage.waitForSelector('#game-controls', { state: 'visible', timeout: 10000 });
            await fancyPage.waitForTimeout(500);

            await fancyPage.click('#btn-toggle-controls');
            await fancyPage.waitForSelector('#controls-panel', { state: 'visible', timeout: 5000 });

            for (let i = 0; i < 5; i++) {
                await fancyPage.click('#btn-run-turn');
                await fancyPage.waitForTimeout(200);
            }

            const fancyTurnInfo = await fancyPage.evaluate(() =>
                document.getElementById('turn-info')?.textContent || 'N/A'
            );

            await fancyPage.screenshot({ path: path.join(ssDir, 'deep_compare_fancy.png') });
            await fancyPage.close();

            // Compare
            finding('OK', `native_game.html at Turn ${gameState.turn}: life P1=${gameState.playerLife} P2=${gameState.oppLife}`);
            finding('OK', `native_game.html player field: [${gameState.playerField.join(', ')}]`);
            finding('OK', `native_game.html opp field: [${gameState.oppField.join(', ')}]`);
            finding('OK', `native_game.html hand count: ${gameState.handCount}`);
            finding('OK', `native_game.html log entries: ${gameState.logs.length}`);
            finding('OK', `tui_game.html turn info: "${fancyTurnInfo}"`);

            // Check that logs don't contain <Choice> entries
            const choiceLeaks = gameState.logs.filter(l => l.startsWith('<Choice>'));
            finding(choiceLeaks.length === 0 ? 'OK' : 'FAIL',
                `Log filter: ${choiceLeaks.length} <Choice> entries leaked (should be 0)`);
        }

        // ================================================================
        // SECTION 4: Edge cases
        // ================================================================
        log('\n========== SECTION 4: Edge Cases ==========');
        {
            const page = await browser.newPage({ viewport: { width: 1280, height: 720 } });
            page.on('pageerror', err => jsErrors.push(err.message));

            // mtg-692: boot heuristic-vs-heuristic local game from URL params.
            const ecBase = `http://localhost:${PORT}`;
            const ecDeck = await firstBuiltinDeck(ecBase);
            await page.goto(localGameUrl(ecBase, 'native_game.html', {
                deck: ecDeck, p1: 'heuristic', p2: 'heuristic', seed: 42,
            }), { waitUntil: 'load', timeout: 30000 });
            await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 15000 });
            await page.waitForTimeout(300);

            // Auto-run to completion
            await page.keyboard.press('a');
            await page.waitForTimeout(8000);
            await page.keyboard.press('a');
            await page.waitForTimeout(300);

            const finalState = await page.evaluate(() => ({
                gameOver: document.getElementById('status-bar')?.textContent?.includes('GAME OVER') || false,
                statusText: document.getElementById('status-bar')?.textContent?.trim() || '',
                playerLife: document.querySelector('#player-info-body .life-total')?.textContent || '',
                oppLife: document.querySelector('#opp-info-body .life-total')?.textContent || '',
                playerField: document.querySelectorAll('#player-field-cards .card').length,
                oppField: document.querySelectorAll('#opp-field-cards .card').length,
                handCards: document.querySelectorAll('#hand-cards .card').length,
                logCount: document.querySelectorAll('#log-body .log-entry').length,
                stackItems: document.querySelectorAll('#stack-body .stack-item').length,
                scrollH: document.body.scrollHeight,
                clientH: document.body.clientHeight,
                hasErrorBanner: document.getElementById('js-error-banner')?.style.display !== 'none'
                    && document.getElementById('js-error-banner')?.style.display !== ''
            }));

            finding(finalState.gameOver ? 'OK' : 'WARN',
                `Game over: ${finalState.gameOver} — ${finalState.statusText.substring(0, 80)}`);
            finding('OK', `Final life: P1=${finalState.playerLife}, P2=${finalState.oppLife}`);
            finding('OK', `Final battlefield: player=${finalState.playerField}, opp=${finalState.oppField}`);
            finding('OK', `Final hand: ${finalState.handCards} cards`);
            finding('OK', `Final log: ${finalState.logCount} entries`);
            finding(finalState.scrollH <= finalState.clientH + 1 ? 'OK' : 'FAIL',
                `No scroll even at game over: scrollH=${finalState.scrollH}, clientH=${finalState.clientH}`);
            finding(!finalState.hasErrorBanner ? 'OK' : 'FAIL',
                `JS error banner: ${finalState.hasErrorBanner ? 'VISIBLE' : 'hidden'}`);

            // Verify creatures show P/T, lands don't
            const ptCheck = await page.evaluate(() => {
                const cards = document.querySelectorAll('#player-field-cards .card, #opp-field-cards .card');
                const landWithPT = [];
                const creatureNoPT = [];
                cards.forEach(c => {
                    const name = c.querySelector('.card-name')?.textContent || '?';
                    const isLand = c.classList.contains('land');
                    const isCreature = c.classList.contains('creature');
                    const hasPT = !!c.querySelector('.card-pt');
                    // An animated land (also a creature, CR 208.3) legitimately
                    // shows P/T — only flag NON-creature lands that show P/T.
                    if (isLand && !isCreature && hasPT) landWithPT.push(name);
                    if (isCreature && !hasPT) creatureNoPT.push(name);
                });
                return { landWithPT, creatureNoPT };
            });
            finding(ptCheck.landWithPT.length === 0 ? 'OK' : 'FAIL',
                `Lands with P/T (should be 0): ${ptCheck.landWithPT.length} — ${ptCheck.landWithPT.join(', ')}`);
            finding(ptCheck.creatureNoPT.length === 0 ? 'OK' : 'WARN',
                `Creatures without P/T: ${ptCheck.creatureNoPT.length} — ${ptCheck.creatureNoPT.join(', ')}`);

            await page.screenshot({ path: path.join(ssDir, 'deep_edge_gameover.png') });

            // mtg-682 page 3 / mtg-692: pure renderer has no launcher to return
            // to — exit (q) navigates to the LOBBY (index.html). Assert that.
            await page.keyboard.press('q');
            await page.waitForFunction(() => /index\.html$/.test(window.location.pathname), null, { timeout: 5000 })
                .catch(() => {});
            const backToLobby = /index\.html$/.test(new URL(page.url()).pathname);
            finding(backToLobby ? 'OK' : 'WARN', `Exit returns to lobby (index.html): ${backToLobby}`);

            await page.close();
        }

        // ================================================================
        // SECTION 5: Check for panics
        // ================================================================
        const panics = jsErrors.filter(e => e.includes('panic') || e.includes('unreachable'));
        finding(panics.length === 0 ? 'OK' : 'FAIL',
            `WASM panics: ${panics.length}`);
        if (panics.length > 0) {
            panics.forEach(p => finding('FAIL', `  Panic: ${p.substring(0, 100)}`));
        }

        // ================================================================
        // RESULTS
        // ================================================================
        log('\n' + '='.repeat(60));
        const counts = { OK: 0, WARN: 0, FAIL: 0, INFO: 0 };
        findings.forEach(f => { counts[f.sev] = (counts[f.sev] || 0) + 1; });
        log(`RESULTS: ${counts.OK} OK, ${counts.WARN} WARN, ${counts.FAIL} FAIL, ${counts.INFO} INFO`);
        log('='.repeat(60));

        findings.filter(f => f.sev === 'FAIL').forEach(f => log(`  FAIL: ${f.msg}`));
        findings.filter(f => f.sev === 'WARN').forEach(f => log(`  WARN: ${f.msg}`));

        const resultsPath = path.join(ssDir, 'deep_test_results.json');
        fs.writeFileSync(resultsPath, JSON.stringify({ findings, counts }, null, 2));

        return counts.FAIL === 0;
    } catch (error) {
        log(`FATAL: ${error.message}`);
        log(error.stack);
        return false;
    } finally {
        if (browser) await browser.close();
        if (server) server.kill();
    }
}

runTest().then(ok => {
    log(ok ? '\n=== DEEP TEST PASSED ===' : '\n=== DEEP TEST HAD FAILURES ===');
    process.exit(ok ? 0 : 1);
});
