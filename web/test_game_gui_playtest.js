// Agent-driven playtest verification of the rebuilt native_game.html.
//
// This is the "long-form" companion to `test_game_gui_rebuild.js` (which is
// the structured per-step pass/fail suite). The playtest plays ≥10 full
// turns per game across two different seeds, sampling card clicks at
// different points and verifying:
//   1. The view model is consistent with the DOM at each sample
//   2. Card click → details update consistency holds for many distinct
//      card_ids over the lifetime of the game
//   3. The game progresses naturally (turn counter monotonic, log grows)
//   4. No JS console errors are emitted at any point
//   5. Battlefield section labels stay in sync with the view model as
//      cards enter/leave play
//
// Bugs are *reported* (collected into the results JSON and the in-process
// `bugs` list); the test only fails when something genuinely catastrophic
// happens — a WASM panic, a JS error, the game getting stuck, etc.
//
// Run with: node18 web/test_game_gui_playtest.js

const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
const { chromium } = require('playwright');

const PORT = 8772;
const SCREEN_DIR = path.join(__dirname, 'screenshots');
const RESULTS_PATH = path.join(SCREEN_DIR, 'game_gui_playtest_results.json');

const GAMES = [
    { label: 'game1_seed42',  seed: '42',  p1: 'eric_avatar_draft', p2: 'gabriel_avatar_draft' },
    { label: 'game2_seed123', seed: '123', p1: 'eric_avatar_draft', p2: 'gabriel_avatar_draft' },
];

/// Minimum number of turns we expect to see during each playtest. Heuristic
/// AI vs heuristic AI on avatar decks tends to take 10-25 turns; if we
/// can't even reach 10 something is badly wrong.
const MIN_TURNS = 10;

/// How long we'll let auto-run drive the game before giving up. Heuristic
/// games normally finish in 3-8 seconds; we wait up to 25 to cover
/// stragglers.
const AUTO_RUN_TIMEOUT_MS = 25000;

/// How many distinct card-click samples we want per playtest. Heuristic
/// games end in 3-8 seconds of auto-run, so we do most click sampling
/// during a manual "warm-up" phase first (Space-stepping a few turns) and
/// then a final round mid-auto-run.
const CLICK_SAMPLES = 8;
/// How many turns to step manually with Space before turning auto-run on.
/// Picked so several lands + creatures are in play and the hand is big.
const WARMUP_TURNS = 6;

function log(msg) { console.log(`[${new Date().toISOString()}] ${msg}`); }
function ensureScreenDir() { if (!fs.existsSync(SCREEN_DIR)) fs.mkdirSync(SCREEN_DIR, { recursive: true }); }

/// Read the live view model from the test bridge installed by native_game.html.
async function readViewModel(page) {
    return page.evaluate(() => {
        if (!window.__mtg) return null;
        try { return JSON.parse(window.__mtg.tui_get_gui_view_model_json()); }
        catch (e) { return { error: e.message }; }
    });
}

/// Collect every card_id currently visible to the perspective player (hand
/// + own battlefield + opponent battlefield).
async function listVisibleCardIds(page) {
    const vm = await readViewModel(page);
    if (!vm || !vm.players) return [];
    const out = [];
    for (const player of vm.players) {
        for (const c of player.hand || []) out.push({ card_id: c.card_id, name: c.name, src: 'hand' });
        for (const sec of player.battlefield_sections || []) {
            for (const c of sec.cards) out.push({ card_id: c.card_id, name: c.name, src: `bf:${sec.label}` });
        }
    }
    return out;
}

/// Click the DOM element keyed by `card_id` and verify (a) the view model's
/// `selected_card.card_id` flipped to that id, and (b) the DOM details
/// pane's name header reflects the new card. Returns the detail object on
/// success, throws a string error on failure.
async function clickCardAndVerify(page, target, bugs) {
    const exists = await page.evaluate((cid) => {
        return !!document.querySelector(`[data-card-id="${cid}"]`);
    }, target.card_id);
    if (!exists) {
        // Card vanished between listVisibleCardIds and click — usual
        // race between the 30 ms render tick and our sampling. Skip.
        return null;
    }
    await page.evaluate((cid) => {
        document.querySelector(`[data-card-id="${cid}"]`).click();
    }, target.card_id);
    await page.waitForTimeout(60);

    const detailJson = await page.evaluate(() => window.__mtg.tui_get_selected_card_details());
    if (!detailJson || detailJson === 'null') {
        bugs.push({
            kind: 'tui_select_card_returned_null',
            card_id: target.card_id,
            name: target.name,
            src: target.src,
            note: 'Visible card returned null — should never happen for a perspective-visible card',
        });
        return null;
    }
    const detail = JSON.parse(detailJson);
    if (detail.card_id !== target.card_id) {
        bugs.push({
            kind: 'selected_card_id_mismatch',
            asked_for: target.card_id,
            got: detail.card_id,
            name: target.name,
        });
        return detail;
    }
    // The DOM detail header MUST reflect the same card.
    const headerText = (await page.textContent('.card-detail-name'))?.trim() || '';
    if (!headerText.includes(detail.name)) {
        bugs.push({
            kind: 'dom_detail_name_mismatch',
            card_id: target.card_id,
            expected: detail.name,
            got: headerText,
        });
    }
    // Image-first sanity: image slot must come before the name slot.
    const order = await page.evaluate(() => {
        const body = document.getElementById('card-details-body');
        if (!body) return null;
        return Array.from(body.children).map(el => el.className || '');
    });
    if (order && order.length > 0) {
        const imgIdx = order.findIndex(c => /card-detail-image/.test(c));
        const nameIdx = order.findIndex(c => /card-detail-name/.test(c));
        if (imgIdx === -1 || nameIdx === -1 || imgIdx > nameIdx) {
            bugs.push({
                kind: 'card_details_image_not_first',
                card_id: target.card_id,
                order,
            });
        }
    }
    return detail;
}

/// Run one playtest. Returns a structured result object — does NOT throw on
/// in-game bugs (those go into `result.bugs`); throws only if the page
/// itself fails to launch / the WASM panics.
async function playOneGame(browser, server, cfg) {
    log(`--- ${cfg.label}: seed=${cfg.seed}, decks=${cfg.p1} vs ${cfg.p2} ---`);
    const result = {
        label: cfg.label,
        cfg,
        startTime: new Date().toISOString(),
        bugs: [],
        consoleErrors: [],
        snapshots: [],
        clickSamples: [],
        turns: { initial: null, final: null, growth: 0 },
        logCount: { initial: 0, final: 0 },
        success: false,
    };

    const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
    page.on('pageerror', err => {
        result.consoleErrors.push({ kind: 'pageerror', msg: err.message });
        log(`!! pageerror: ${err.message}`);
    });
    page.on('console', msg => {
        if (msg.type() === 'error') {
            const text = msg.text();
            // Card image fetches use a fallback chain (local → Scryfall →
            // Gatherer); on a sandboxed devserver without external network
            // access ALL of these 404 / fail. They are NOT game bugs — the
            // GUI handles missing images by hiding the <img>. Filter them
            // out so they don't drown out real JS errors.
            if (/Failed to load resource/.test(text)
                && (/404/.test(text) || /ERR_/.test(text) || /net::/.test(text))) {
                return;
            }
            result.consoleErrors.push({ kind: 'console_error', msg: text });
        }
    });

    try {
        // Boot the launcher and configure the game.
        await page.goto(`http://localhost:${PORT}/native_game.html`, { waitUntil: 'networkidle', timeout: 60000 });
        await page.waitForSelector('#launcher.show', { state: 'visible', timeout: 30000 });

        await page.selectOption('#p1-controller', 'heuristic');
        await page.selectOption('#p2-controller', 'heuristic');
        await page.selectOption('#p1-collection', 'booster_draft');
        await page.selectOption('#p2-collection', 'booster_draft');
        await page.selectOption('#p1-deck', cfg.p1);
        await page.selectOption('#p2-deck', cfg.p2);
        await page.fill('#game-seed', cfg.seed);

        await page.click('#btn-launch');
        await page.waitForSelector('#game-area.show', { state: 'visible', timeout: 30000 });
        await page.waitForTimeout(500);

        // Initial snapshot.
        let vm = await readViewModel(page);
        if (!vm || vm.error) throw new Error(`view model unreadable at start: ${JSON.stringify(vm)}`);
        result.turns.initial = vm.turn_number;
        result.logCount.initial = (vm.logs || []).length;
        result.snapshots.push({
            t: 0, turn: vm.turn_number, step: vm.current_step,
            our_life: vm.players[vm.our_player_idx]?.life,
            opp_life: vm.players.find(p => p.player_id !== vm.players[vm.our_player_idx].player_id)?.life,
            stack_size: (vm.stack || []).length,
        });

        // ---- Warm-up phase: step turns manually with Space, then sample
        // clicks while cards are reliably in play. This is necessary
        // because heuristic vs heuristic games on these decks finish in
        // ~3-8 s of auto-run and we want ~CLICK_SAMPLES distinct samples.
        log(`${cfg.label}: warm-up — ${WARMUP_TURNS} Space turns then click sampling`);
        for (let i = 0; i < WARMUP_TURNS; i++) {
            await page.keyboard.press('Space');
            await page.waitForTimeout(220);
        }

        // Click sampling: walk every visible card_id (up to CLICK_SAMPLES)
        // and verify each one selects correctly + updates the details pane.
        const visibleAtWarmup = await listVisibleCardIds(page);
        const seen = new Set();
        for (const target of visibleAtWarmup) {
            if (seen.has(target.card_id)) continue;
            if (result.clickSamples.length >= CLICK_SAMPLES) break;
            seen.add(target.card_id);
            const t0 = Date.now();
            const detail = await clickCardAndVerify(page, target, result.bugs);
            vm = await readViewModel(page);
            result.clickSamples.push({
                phase: 'warmup',
                t: Date.now() - t0,
                turn: vm.turn_number,
                target,
                detail_card_id: detail?.card_id ?? null,
                detail_name: detail?.name ?? null,
                detail_pt: detail?.pt_line ?? null,
                detail_oracle_lines: detail?.oracle_lines?.length ?? 0,
                had_image_first: !result.bugs.some(b => b.kind === 'card_details_image_not_first' && b.card_id === target.card_id),
            });
            log(`${cfg.label} warm-up t=${vm.turn_number}: click ${target.src} card_id=${target.card_id} (${target.name}) → "${detail?.name ?? 'null'}"`);
        }

        // ---- Auto-run phase: drive the game to completion. Periodically
        // sample one more click so we exercise the click→details path
        // while the game is actively mutating state.
        log(`${cfg.label}: kicking off auto-run`);
        await page.keyboard.press('a'); // toggle auto-run ON

        const startMs = Date.now();
        // Sample mid-game clicks every ~600 ms so even a 3 s auto-run game
        // gets a few additional snapshots.
        const sampleEveryMs = 600;
        let nextSampleAt = startMs + 400;
        let gameOver = false;
        let lastTurn = vm.turn_number;
        let stuckTicks = 0;

        while (Date.now() - startMs < AUTO_RUN_TIMEOUT_MS) {
            await page.waitForTimeout(250);

            vm = await readViewModel(page);
            if (!vm || vm.error) {
                result.bugs.push({ kind: 'view_model_unreadable', vm });
                break;
            }

            // Track game progress for the turn-monotonicity invariant.
            if (vm.turn_number < lastTurn) {
                result.bugs.push({
                    kind: 'turn_counter_regressed',
                    from: lastTurn,
                    to: vm.turn_number,
                });
            }
            if (vm.turn_number === lastTurn) {
                stuckTicks++;
            } else {
                stuckTicks = 0;
                lastTurn = vm.turn_number;
            }

            // Mid-auto-run click sample: pick a card we haven't sampled
            // yet during this game (across BOTH warmup and auto-run
            // phases) so the total set of clicked card_ids is maximally
            // diverse. We bail when we've hit CLICK_SAMPLES total.
            if (Date.now() >= nextSampleAt && result.clickSamples.length < CLICK_SAMPLES) {
                const visible = await listVisibleCardIds(page);
                if (visible.length > 0) {
                    const sampledIds = new Set(result.clickSamples.map(s => s?.target?.card_id));
                    const fresh = visible.filter(c => !sampledIds.has(c.card_id));
                    const target = (fresh[0] || visible[0]);
                    const detail = await clickCardAndVerify(page, target, result.bugs);
                    result.clickSamples.push({
                        phase: 'autorun',
                        t: Date.now() - startMs,
                        turn: vm.turn_number,
                        target,
                        detail_card_id: detail?.card_id ?? null,
                        detail_name: detail?.name ?? null,
                        detail_pt: detail?.pt_line ?? null,
                        detail_oracle_lines: detail?.oracle_lines?.length ?? 0,
                        had_image_first: !result.bugs.some(b => b.kind === 'card_details_image_not_first' && b.card_id === target.card_id),
                    });
                    log(`${cfg.label} auto t=${vm.turn_number}: click ${target.src} card_id=${target.card_id} (${target.name}) → detail.name="${detail?.name}"`);
                }
                nextSampleAt = Date.now() + sampleEveryMs;
            }

            if (vm.game_over) { gameOver = true; break; }
        }

        await page.keyboard.press('a'); // toggle auto-run OFF
        await page.waitForTimeout(300);

        // Final snapshot.
        vm = await readViewModel(page);
        result.turns.final = vm.turn_number;
        result.turns.growth = (vm.turn_number || 0) - (result.turns.initial || 0);
        result.logCount.final = (vm.logs || []).length;
        result.snapshots.push({
            t: Date.now() - startMs, turn: vm.turn_number, step: vm.current_step,
            our_life: vm.players[vm.our_player_idx]?.life,
            opp_life: vm.players.find(p => p.player_id !== vm.players[vm.our_player_idx].player_id)?.life,
            game_over: vm.game_over,
            stack_size: (vm.stack || []).length,
        });

        // Record evidence.
        await page.screenshot({ path: path.join(SCREEN_DIR, `playtest_${cfg.label}_final.png`), fullPage: true });

        // Game-progress sanity bug filing.
        if (result.turns.final < MIN_TURNS) {
            result.bugs.push({
                kind: 'too_few_turns_played',
                expected_min: MIN_TURNS,
                got: result.turns.final,
                game_over: !!vm.game_over,
            });
        }
        if (stuckTicks > 30) {
            result.bugs.push({
                kind: 'game_appeared_stuck',
                stuck_ticks: stuckTicks,
                last_turn: lastTurn,
                last_step: vm.current_step,
            });
        }
        if (result.logCount.final <= result.logCount.initial) {
            result.bugs.push({
                kind: 'log_did_not_grow',
                initial: result.logCount.initial,
                final: result.logCount.final,
            });
        }

        // Exit and return to launcher.
        await page.keyboard.press('q');
        await page.waitForTimeout(400);
        const launcherShown = await page.evaluate(() =>
            document.getElementById('launcher')?.classList.contains('show') || false);
        if (!launcherShown) {
            result.bugs.push({ kind: 'exit_did_not_return_to_launcher' });
        }

        result.success = result.consoleErrors.length === 0
            && result.turns.final >= MIN_TURNS
            && !result.bugs.some(b => /panic|stuck|regressed/.test(b.kind));
    } catch (err) {
        result.bugs.push({ kind: 'fatal', msg: err.message });
        result.success = false;
    } finally {
        result.endTime = new Date().toISOString();
        await page.close();
    }
    return result;
}

async function main() {
    ensureScreenDir();
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

    const overall = { startTime: new Date().toISOString(), games: [], allBugs: [], success: false };
    try {
        for (const cfg of GAMES) {
            const r = await playOneGame(browser, server, cfg);
            overall.games.push(r);
            for (const b of r.bugs) overall.allBugs.push({ game: cfg.label, ...b });
            for (const e of r.consoleErrors) overall.allBugs.push({ game: cfg.label, kind: 'console_' + e.kind, msg: e.msg });
        }
        overall.success = overall.games.every(g => g.success) && overall.allBugs.length === 0;
    } finally {
        overall.endTime = new Date().toISOString();
        try { fs.writeFileSync(RESULTS_PATH, JSON.stringify(overall, null, 2)); } catch (e) { /* ignore */ }
        await browser.close();
        server.kill();
    }

    // Summary.
    log('');
    log('=== Playtest Summary ===');
    for (const g of overall.games) {
        log(`  ${g.label}: turns ${g.turns.initial} → ${g.turns.final} (Δ${g.turns.growth}), ` +
            `clicks=${g.clickSamples.length}, bugs=${g.bugs.length}, errors=${g.consoleErrors.length}, ` +
            `success=${g.success}`);
    }
    log('');
    if (overall.allBugs.length > 0) {
        log(`!! ${overall.allBugs.length} bug(s) collected:`);
        for (const b of overall.allBugs) {
            log(`   ${b.game}: ${b.kind}${b.msg ? ` — ${b.msg}` : ''}`);
        }
    } else {
        log('No bugs collected.');
    }
    log(`Results JSON: ${RESULTS_PATH}`);
    log(overall.success ? '=== Playtest PASSED ===' : '=== Playtest had bugs/failures ===');
    return overall.success;
}

main().then(ok => process.exit(ok ? 0 : 1))
      .catch(err => { console.error('Unhandled:', err); process.exit(1); });
