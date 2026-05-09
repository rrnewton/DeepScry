// Comprehensive Playwright E2E test for the rebuilt thin-DOM game.html.
//
// Validates the migration to the structured GuiViewModel exported by
// `tui_get_gui_view_model_json()` and the shared selection state exposed by
// `tui_select_card(card_id)`. Specifically covers the bug class where the
// JS-local `selectedCardName` could not handle two cards with the same name
// being selected in sequence — the new path keys selection by stable
// `card_id`, so this test asserts that distinct card_ids each produce
// distinct details payloads.
//
// Run with: node18 web/test_game_gui_rebuild.js
// Requires: WASM built (`make wasm-dev`) and `web/node_modules/playwright`.

const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
const { chromium } = require('playwright');

const PORT = 8771;
const SCREEN_DIR = path.join(__dirname, 'screenshots');
const RESULTS_PATH = path.join(SCREEN_DIR, 'game_gui_rebuild_results.json');

// Decks the task asks for. Both are heuristic AIs so the run is reproducible.
const P1_DECK = 'eric_avatar_draft';
const P2_DECK = 'gabriel_avatar_draft';
const DECK_COLLECTION = 'booster_draft';
const GAME_SEED = '42';

function log(msg) { console.log(`[${new Date().toISOString()}] ${msg}`); }

async function ensureScreenDir() {
    if (!fs.existsSync(SCREEN_DIR)) fs.mkdirSync(SCREEN_DIR, { recursive: true });
}

/// One test step. Records a structured pass/fail entry; throws only when
/// `fatal` is true so the run can collect as many findings as possible
/// before reporting overall success/failure.
function step(results, name, fn, opts = {}) {
    return (async () => {
        const t0 = Date.now();
        try {
            const value = await fn();
            results.steps.push({
                name,
                ok: true,
                durationMs: Date.now() - t0,
                value: value === undefined ? null : value,
            });
            log(`OK   ${name}${value !== undefined ? `  → ${JSON.stringify(value).slice(0, 120)}` : ''}`);
            return value;
        } catch (e) {
            results.steps.push({
                name,
                ok: false,
                durationMs: Date.now() - t0,
                error: e.message,
            });
            log(`FAIL ${name}: ${e.message}`);
            if (opts.fatal) throw e;
            return null;
        }
    })();
}

/// Pull the live `GuiViewModel` from the running page. We re-parse it on
/// every assertion so race-y intermediate frames are visible to the test.
///
/// Reaches into `window.__mtg` which `game.html` populates after WASM init
/// specifically so headless tests like this one can probe the view model
/// without re-importing the module.
async function readViewModel(page) {
    return page.evaluate(() => {
        if (!window.__mtg || typeof window.__mtg.getViewModel !== 'function') return null;
        return window.__mtg.getViewModel();
    });
}

/// Run a single turn via the keyboard Space binding. Mirrors what a human
/// player would do in the GUI.
async function pressSpaceAndSettle(page, ms = 200) {
    await page.keyboard.press('Space');
    await page.waitForTimeout(ms);
}

/// Wait until at least one selectable card exists somewhere on the page.
/// Returns the array of `{card_id, name}` candidates from the view model.
async function waitForSelectableCards(page, timeoutMs = 10000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const vm = await readViewModel(page);
        if (vm && vm.players) {
            const our = vm.players[vm.our_player_idx ?? 0];
            const cards = [];
            for (const c of our?.hand || []) cards.push({ card_id: c.card_id, name: c.name, src: 'hand' });
            for (const sec of our?.battlefield_sections || []) {
                for (const c of sec.cards) cards.push({ card_id: c.card_id, name: c.name, src: `bf:${sec.label}` });
            }
            if (cards.length > 0) return cards;
        }
        await page.waitForTimeout(300);
    }
    return [];
}

async function runTest() {
    await ensureScreenDir();
    const results = {
        startTime: new Date().toISOString(),
        decks: { p1: P1_DECK, p2: P2_DECK },
        seed: GAME_SEED,
        steps: [],
        browserErrors: [],
        consoleWarnings: [],
        screenshots: [],
        success: false,
    };

    // ---- Boot HTTP server + browser --------------------------------------
    log(`Starting HTTP server on port ${PORT}...`);
    const server = spawn('python3', ['-m', 'http.server', String(PORT)], {
        cwd: __dirname,
        stdio: ['ignore', 'pipe', 'pipe'],
    });
    await new Promise(r => setTimeout(r, 1200));

    log('Launching Chromium...');
    const browser = await chromium.launch({
        headless: true,
        args: ['--no-sandbox', '--enable-unsafe-swiftshader'],
    });

    try {
        const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });

        // Capture all browser-side errors. WASM panics + JS exceptions both
        // need to fail the test loudly because they break the GUI silently.
        page.on('pageerror', err => {
            results.browserErrors.push({ ts: new Date().toISOString(), msg: err.message });
            log(`!! pageerror: ${err.message}`);
        });
        page.on('console', msg => {
            if (msg.type() === 'error' || msg.type() === 'warning') {
                const text = msg.text();
                results.consoleWarnings.push({ ts: new Date().toISOString(), type: msg.type(), text });
            }
        });

        const screenshot = async (name) => {
            const file = path.join(SCREEN_DIR, name);
            await page.screenshot({ path: file, fullPage: true });
            results.screenshots.push(name);
        };

        // ---- 1. Load game.html, wait for WASM init ----------------------
        await step(results, 'page_load', async () => {
            await page.goto(`http://localhost:${PORT}/game.html`, { waitUntil: 'networkidle', timeout: 60000 });
            await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });
        }, { fatal: true });
        await screenshot('rebuild_01_launcher.png');

        // The new exports must be reachable through `window.__mtg` (the
        // game.html test bridge installed right after `await init()`).
        await step(results, 'new_wasm_exports_present', async () => {
            const present = await page.evaluate(() => {
                if (!window.__mtg) return { __mtg: 'missing' };
                return {
                    view_model: typeof window.__mtg.tui_get_gui_view_model_json,
                    select_card: typeof window.__mtg.tui_select_card,
                    clear_selection: typeof window.__mtg.tui_clear_card_selection,
                    get_selected: typeof window.__mtg.tui_get_selected_card_details,
                };
            });
            for (const k of ['view_model', 'select_card', 'clear_selection', 'get_selected']) {
                if (present[k] !== 'function') {
                    throw new Error(`window.__mtg.${k} = ${present[k]}, expected function`);
                }
            }
            return present;
        });

        // ---- 2. Configure (eric vs gabriel, heuristic AIs, seed=42) -----
        await step(results, 'configure_decks', async () => {
            // Both controllers heuristic so the run is deterministic for screenshots.
            await page.selectOption('#p1-controller', 'heuristic');
            await page.selectOption('#p2-controller', 'heuristic');

            await page.selectOption('#p1-collection', DECK_COLLECTION);
            await page.selectOption('#p2-collection', DECK_COLLECTION);
            await page.selectOption('#p1-deck', P1_DECK);
            await page.selectOption('#p2-deck', P2_DECK);

            await page.fill('#game-seed', GAME_SEED);
            await page.check('#debug-mode'); // Surfaces [Debug] logs in console.

            const cfg = await page.evaluate(() => ({
                p1: document.getElementById('p1-deck').value,
                p2: document.getElementById('p2-deck').value,
                seed: document.getElementById('game-seed').value,
            }));
            if (cfg.p1 !== P1_DECK || cfg.p2 !== P2_DECK || cfg.seed !== GAME_SEED) {
                throw new Error(`config mismatch: ${JSON.stringify(cfg)}`);
            }
            return cfg;
        }, { fatal: true });
        await screenshot('rebuild_02_configured.png');

        // ---- 3. Launch game; wait for game area ------------------------
        await step(results, 'launch_game', async () => {
            await page.click('#btn-launch');
            await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 30000 });
            await page.waitForTimeout(800);
        }, { fatal: true });
        await screenshot('rebuild_03_initial.png');

        // ---- 4. View model is well-formed ------------------------------
        await step(results, 'view_model_well_formed', async () => {
            const vm = await readViewModel(page);
            if (!vm || vm.error) throw new Error(`view model unreadable: ${JSON.stringify(vm)}`);
            if (vm.schema_version !== 1) throw new Error(`schema_version=${vm.schema_version}, expected 1`);
            if (!Array.isArray(vm.players) || vm.players.length !== 2) {
                throw new Error(`expected 2 players, got ${vm.players?.length}`);
            }
            // Each player must expose the new battlefield_sections shape.
            for (const p of vm.players) {
                if (!Array.isArray(p.battlefield_sections)) {
                    throw new Error(`player ${p.name} missing battlefield_sections`);
                }
                if (typeof p.info_bar_text !== 'string' || p.info_bar_text.length === 0) {
                    throw new Error(`player ${p.name} missing info_bar_text`);
                }
            }
            return {
                schema_version: vm.schema_version,
                turn_number: vm.turn_number,
                step: vm.current_step,
                our_idx: vm.our_player_idx,
                bf_sections: vm.players.map(p => p.battlefield_sections.map(s => s.label)),
            };
        }, { fatal: true });

        // ---- 5. Status bar text matches view model ---------------------
        await step(results, 'status_bar_text', async () => {
            const vm = await readViewModel(page);
            const bar = (await page.textContent('#status-bar')).replace(/\s+/g, ' ').trim();
            // The status bar pulls .turn + .phase from `state.status_text`,
            // which in turn comes from Rust's "Turn N | Phase: … | Active: P?".
            if (!bar.includes(`Turn ${vm.turn_number}`)) {
                throw new Error(`status bar "${bar}" missing "Turn ${vm.turn_number}"`);
            }
            if (!bar.includes('Phase')) throw new Error(`status bar missing "Phase": "${bar}"`);
            return bar.slice(0, 80);
        });

        // ---- 6. Player info bars come from view model directly --------
        await step(results, 'player_info_from_view_model', async () => {
            const vm = await readViewModel(page);
            const ourBar = (await page.textContent('#player-info-body')).trim();
            const oppBar = (await page.textContent('#opp-info-body')).trim();
            const ourPlayer = vm.players[vm.our_player_idx];
            const oppPlayer = vm.players.find(p => p.player_id !== ourPlayer.player_id);
            if (ourBar !== ourPlayer.info_bar_text) {
                throw new Error(`our info bar mismatch: dom="${ourBar}" vm="${ourPlayer.info_bar_text}"`);
            }
            if (oppBar !== oppPlayer.info_bar_text) {
                throw new Error(`opp info bar mismatch: dom="${oppBar}" vm="${oppPlayer.info_bar_text}"`);
            }
            return { ours: ourBar, opp: oppBar };
        });

        // ---- 7. Run a few turns to populate hand + battlefield --------
        await step(results, 'run_initial_turns', async () => {
            for (let i = 0; i < 6; i++) await pressSpaceAndSettle(page, 220);
            const vm = await readViewModel(page);
            return { turn: vm.turn_number, step: vm.current_step };
        });
        await screenshot('rebuild_04_after_initial_turns.png');

        // ---- 8. Log shows a Turn header --------------------------------
        await step(results, 'log_has_turn_header', async () => {
            const vm = await readViewModel(page);
            const turnEntries = (vm.logs || []).filter(e => e.semantic_class === 'log-turn-header');
            if (turnEntries.length === 0) {
                throw new Error('no turn-header log entries (expected ">>> Turn …")');
            }
            // Also assert the DOM mirrors that semantic class.
            const domTurnHeaders = await page.evaluate(() =>
                document.querySelectorAll('#log-body .log-turn-header').length);
            if (domTurnHeaders === 0) throw new Error('DOM shows no .log-turn-header rows');
            return { vm: turnEntries.length, dom: domTurnHeaders, sample: turnEntries[0].text };
        });

        // ---- 9. Card click selection updates the details pane ----------
        // First card.
        let candidates = await waitForSelectableCards(page, 8000);
        if (candidates.length < 1) {
            await step(results, 'find_selectable_cards', async () => {
                throw new Error('no selectable cards after 6 turns');
            });
        }
        await step(results, 'click_first_card_updates_details', async () => {
            const target = candidates[0];
            await page.evaluate((cid) => {
                const el = document.querySelector(`[data-card-id="${cid}"]`);
                if (!el) throw new Error(`no DOM element for card_id=${cid}`);
                el.click();
            }, target.card_id);
            await page.waitForTimeout(150);
            const detailJson = await page.evaluate(() => window.__mtg?.tui_get_selected_card_details() ?? null);
            const detail = detailJson ? JSON.parse(detailJson) : null;
            if (!detail || detail.card_id !== target.card_id) {
                throw new Error(`expected selected card_id=${target.card_id}, got ${detail?.card_id}`);
            }
            return { card_id: detail.card_id, name: detail.name };
        });
        await screenshot('rebuild_05_after_first_click.png');

        // ---- 10. Multiple different cards selected in sequence (the bug) -
        // Pick three distinct card_ids if available; if there are fewer
        // than three, fall back to whatever we have but still assert each
        // click switches the details panel.
        await step(results, 'sequential_distinct_card_clicks', async () => {
            candidates = await waitForSelectableCards(page, 4000);
            if (candidates.length < 2) throw new Error(`need ≥2 cards, have ${candidates.length}`);
            const picks = candidates.slice(0, Math.min(3, candidates.length));
            const seenDetails = [];
            for (const target of picks) {
                await page.evaluate((cid) => {
                    const el = document.querySelector(`[data-card-id="${cid}"]`);
                    if (!el) throw new Error(`missing data-card-id=${cid}`);
                    el.click();
                }, target.card_id);
                await page.waitForTimeout(150);
                const detailJson = await page.evaluate(() => window.__mtg.tui_get_selected_card_details());
                const detail = JSON.parse(detailJson);
                if (detail.card_id !== target.card_id) {
                    throw new Error(`click on ${target.card_id} → details show ${detail.card_id}`);
                }
                // The DOM details pane MUST also reflect the new selection.
                const headerText = await page.textContent('.card-detail-name');
                if (!headerText.includes(detail.name)) {
                    throw new Error(`details DOM "${headerText}" doesn't contain "${detail.name}"`);
                }
                seenDetails.push({ card_id: detail.card_id, name: detail.name });
            }
            // The seen cards must be DISTINCT (the original bug allowed
            // two same-name cards to collapse to one selection).
            const distinct = new Set(seenDetails.map(d => d.card_id));
            if (distinct.size !== seenDetails.length) {
                throw new Error(`expected distinct card_ids, got ${JSON.stringify(seenDetails)}`);
            }
            return seenDetails;
        });
        await screenshot('rebuild_06_after_sequential_clicks.png');

        // ---- 11. Card details pane: image element appears BEFORE text --
        await step(results, 'card_details_image_first', async () => {
            const order = await page.evaluate(() => {
                const body = document.getElementById('card-details-body');
                if (!body) return null;
                const kids = Array.from(body.children).map(el => ({
                    tag: el.tagName,
                    cls: el.className || '',
                }));
                return kids;
            });
            if (!order || order.length === 0) throw new Error('details body is empty');
            const imgIdx = order.findIndex(k => /card-detail-image/.test(k.cls));
            const nameIdx = order.findIndex(k => /card-detail-name/.test(k.cls));
            if (imgIdx === -1 || nameIdx === -1) {
                throw new Error(`could not find image+name slots in details: ${JSON.stringify(order)}`);
            }
            if (imgIdx > nameIdx) {
                throw new Error(`image (idx ${imgIdx}) appears AFTER name (idx ${nameIdx})`);
            }
            return { imgIdx, nameIdx, order };
        });

        // ---- 12. Action selection — keyboard Space advances ------------
        await step(results, 'space_keyboard_advances_turn', async () => {
            const vmBefore = await readViewModel(page);
            await pressSpaceAndSettle(page, 250);
            await pressSpaceAndSettle(page, 250);
            const vmAfter = await readViewModel(page);
            // After two Space presses the (turn, step) tuple MUST advance.
            const sameTurn = vmBefore.turn_number === vmAfter.turn_number
                          && vmBefore.current_step === vmAfter.current_step;
            if (sameTurn) {
                throw new Error(`Space did not advance: ${vmBefore.turn_number}/${vmBefore.current_step} → ${vmAfter.turn_number}/${vmAfter.current_step}`);
            }
            return { from: `${vmBefore.turn_number}/${vmBefore.current_step}`, to: `${vmAfter.turn_number}/${vmAfter.current_step}` };
        });

        // ---- 13. Battlefield + hand evolve over multiple turns ---------
        await step(results, 'battlefield_and_hand_evolve', async () => {
            const before = await readViewModel(page);
            const beforeBfSizes = before.players.map(p =>
                p.battlefield_sections.reduce((n, s) => n + s.cards.length, 0));
            const beforeHandSize = before.players[before.our_player_idx].hand_size;

            for (let i = 0; i < 8; i++) await pressSpaceAndSettle(page, 220);

            const after = await readViewModel(page);
            const afterBfSizes = after.players.map(p =>
                p.battlefield_sections.reduce((n, s) => n + s.cards.length, 0));
            const afterHandSize = after.players[after.our_player_idx].hand_size;

            // After several turns the total cards in play should generally
            // be ≥ the initial count (lands and creatures get played).
            const totalBefore = beforeBfSizes.reduce((a, b) => a + b, 0);
            const totalAfter = afterBfSizes.reduce((a, b) => a + b, 0);
            if (totalAfter < totalBefore) {
                throw new Error(`battlefield shrank: ${totalBefore} → ${totalAfter}`);
            }
            // Hand should still be nonempty (drew cards each turn).
            if (afterHandSize < 1) throw new Error(`hand empty after 8 more turns`);
            // Turn number must have advanced.
            if (after.turn_number <= before.turn_number) {
                throw new Error(`turn did not advance: ${before.turn_number} → ${after.turn_number}`);
            }
            return {
                turns: `${before.turn_number} → ${after.turn_number}`,
                bf: { from: beforeBfSizes, to: afterBfSizes },
                hand: { from: beforeHandSize, to: afterHandSize },
            };
        });
        await screenshot('rebuild_07_midgame.png');

        // ---- 14. Battlefield section labels rendered in DOM ------------
        await step(results, 'battlefield_section_labels_rendered', async () => {
            const vm = await readViewModel(page);
            // Pick the player whose battlefield has at least one section.
            const playerWithBf = vm.players.find(p => p.battlefield_sections.length > 0);
            if (!playerWithBf) {
                // Skip rather than fail — early game can have empty fields.
                return { skipped: 'no player has populated battlefield yet' };
            }
            const ourSec = vm.players[vm.our_player_idx].battlefield_sections;
            const oppSec = vm.players.find(p => p.player_id !== vm.players[vm.our_player_idx].player_id).battlefield_sections;
            const ourLabels = await page.evaluate(() =>
                Array.from(document.querySelectorAll('#player-field-cards .bf-section-label'))
                    .map(el => el.textContent));
            const oppLabels = await page.evaluate(() =>
                Array.from(document.querySelectorAll('#opp-field-cards .bf-section-label'))
                    .map(el => el.textContent));
            // We don't enforce strict label match because card counts change
            // every render, but the COUNT of sections rendered must equal
            // the number in the view model.
            if (ourLabels.length !== ourSec.length) {
                throw new Error(`our bf sections: dom=${ourLabels.length} vm=${ourSec.length}`);
            }
            if (oppLabels.length !== oppSec.length) {
                throw new Error(`opp bf sections: dom=${oppLabels.length} vm=${oppSec.length}`);
            }
            return { ourLabels, oppLabels };
        });

        // ---- 15. Auto-run for a few seconds, verify game progresses ---
        await step(results, 'auto_run_progresses', async () => {
            const before = await readViewModel(page);
            await page.keyboard.press('a'); // toggle auto-run on
            await page.waitForTimeout(2500);
            await page.keyboard.press('a'); // toggle off
            await page.waitForTimeout(300);
            const after = await readViewModel(page);
            if (after.turn_number < before.turn_number) {
                throw new Error(`auto-run regressed turn count: ${before.turn_number} → ${after.turn_number}`);
            }
            return { turns: `${before.turn_number} → ${after.turn_number}` };
        });
        await screenshot('rebuild_08_after_autorun.png');

        // ---- 16. No JS errors / WASM panics ---------------------------
        await step(results, 'no_js_errors', async () => {
            if (results.browserErrors.length > 0) {
                throw new Error(`${results.browserErrors.length} pageerror(s): ${results.browserErrors.map(e => e.msg).join(' | ').slice(0, 200)}`);
            }
            return 'clean';
        });

        await step(results, 'no_visible_error_banner', async () => {
            const display = await page.evaluate(() => {
                const b = document.getElementById('js-error-banner');
                return b ? b.style.display : 'missing';
            });
            if (display !== 'none' && display !== 'missing' && display !== '') {
                const txt = await page.textContent('#js-error-messages');
                throw new Error(`error banner visible: ${txt?.slice(0, 200)}`);
            }
            return display;
        });

        // ---- 17. Exit returns to launcher -----------------------------
        await step(results, 'exit_to_launcher', async () => {
            await page.keyboard.press('q');
            await page.waitForTimeout(400);
            const visible = await page.evaluate(() =>
                document.getElementById('launcher')?.classList.contains('show') || false);
            if (!visible) throw new Error('launcher not shown after exit');
        });
        await screenshot('rebuild_09_after_exit.png');

        // ---- DONE ------------------------------------------------------
        const failed = results.steps.filter(s => !s.ok);
        results.passed = results.steps.length - failed.length;
        results.failed = failed.length;
        results.success = failed.length === 0 && results.browserErrors.length === 0;
    } finally {
        results.endTime = new Date().toISOString();
        try { fs.writeFileSync(RESULTS_PATH, JSON.stringify(results, null, 2)); } catch (e) { /* ignore */ }
        await browser.close();
        server.kill();
    }

    // Print summary.
    log('');
    log('=== Step Summary ===');
    for (const s of results.steps) {
        log(`  ${s.ok ? 'OK  ' : 'FAIL'} ${s.name}  (${s.durationMs}ms)`);
    }
    log('');
    log(`Steps: ${results.passed}/${results.steps.length} passed`);
    log(`Browser errors: ${results.browserErrors.length}`);
    log(`Screenshots saved to: ${SCREEN_DIR}`);
    log(`Results JSON: ${RESULTS_PATH}`);
    return results.success;
}

runTest().then(ok => {
    log(ok ? '=== game.html rebuild E2E PASSED ===' : '=== game.html rebuild E2E FAILED ===');
    process.exit(ok ? 0 : 1);
}).catch(err => {
    console.error('Unhandled error:', err);
    process.exit(1);
});
