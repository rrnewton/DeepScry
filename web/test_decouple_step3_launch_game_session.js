#!/usr/bin/env node
/**
 * E2E test for decouple-step3: `launch_game_session` works WITHOUT ratzilla.
 *
 * Loads `native_game.html` (so wasm-bindgen initializes the card database) and then
 * deliberately:
 *   1. REMOVES the `<div id="ratzilla-terminal">` element from the DOM, so any
 *      hidden ratzilla launcher would fail.
 *   2. Calls `launch_game_session(...)` directly via the WASM bindings instead
 *      of clicking the launch button (which would call `launch_fancy_tui`).
 *   3. Drives the game forward via `tui_run_turn()` / `tui_tick()` and reads
 *      `tui_get_gui_view_model_json()` to confirm state advances and the
 *      view-model JSON is well-formed.
 *
 * If ratzilla were still required, step 1 alone would break the session:
 * `launch_fancy_tui` calls `DomBackend::new_by_id("ratzilla-terminal")` which
 * errors when the element is missing. After decouple-step3,
 * `launch_game_session` deliberately doesn't touch that element.
 */
const { chromium } = require('playwright');
const { spawn } = require('child_process');
const path = require('path');
const { getRandomPorts } = require('./test_network_utils');

const projectRoot = path.join(__dirname, '..');

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

(async () => {
    let httpServer, browser;
    const { httpPort: HTTP_PORT } = await getRandomPorts();
    let failures = [];

    function check(name, ok, detail) {
        if (ok) {
            log(`PASS: ${name} — ${detail}`);
        } else {
            log(`FAIL: ${name} — ${detail}`);
            failures.push(`${name}: ${detail}`);
        }
    }

    try {
        httpServer = spawn('python3', ['-m', 'http.server', HTTP_PORT.toString()], {
            cwd: path.join(projectRoot, 'web'),
            stdio: ['ignore', 'pipe', 'pipe'],
        });
        await new Promise(r => setTimeout(r, 1500));

        browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });

        const page = await browser.newPage();
        await page.setViewportSize({ width: 1280, height: 720 });

        // Capture browser console errors so we surface WASM panics into the
        // test output (rather than silently failing).
        const browserErrors = [];
        page.on('pageerror', err => browserErrors.push(err.message));
        page.on('console', msg => {
            if (msg.type() === 'error') browserErrors.push(`console.error: ${msg.text()}`);
        });

        await page.goto(`http://localhost:${HTTP_PORT}/native_game.html`, {
            waitUntil: 'networkidle',
            timeout: 30000,
        });
        await page.waitForFunction(() => {
            const s = document.getElementById('p1-deck');
            return s && s.options.length > 0;
        }, { timeout: 30000 });
        log('WASM module loaded, decks ready');

        // ===== Step 1: Confirm there is no #ratzilla-terminal in the DOM =====
        //
        // Pre decouple-step4 (mtg-81ed52), `web/native_game.html` shipped with a
        // hidden `<div id="ratzilla-terminal" style="display:none">` that
        // `launch_fancy_tui` populated with a ratzilla `DomBackend`. This
        // test deliberately asserted the div *was* there and removed it,
        // so any leftover `launch_fancy_tui` call would error out at
        // `DomBackend::new_by_id`.
        //
        // Post step 4, `web/native_game.html` no longer contains the div at all
        // (verified here) AND no longer calls `launch_fancy_tui` — it
        // uses `launch_game_session` directly. As a defence-in-depth
        // measure we ALSO try to remove the element if some future
        // refactor accidentally re-adds it.
        const initiallyPresent = await page.evaluate(() => {
            const el = document.getElementById('ratzilla-terminal');
            if (el) {
                el.parentNode.removeChild(el);
                return true;
            }
            return false;
        });
        check('ratzilla-terminal element absent from native_game.html (decouple-step4)',
              !initiallyPresent,
              initiallyPresent
                ? 'div was present (and was just removed for the test)'
                : 'div was already absent — native_game.html ships without it');

        // Sanity: confirm there's no #ratzilla-terminal anywhere now.
        const stillThere = await page.evaluate(() => !!document.getElementById('ratzilla-terminal'));
        check('ratzilla-terminal element confirmed absent before launch',
              !stillThere,
              `getElementById returned ${stillThere ? 'present' : 'null'}`);

        // ===== Step 2: Call launch_game_session via the JS export =====
        // The page exposes the WASM binding via the same `window.__wasm`
        // namespace as the existing tests use? Looking at native_game.html, it doesn't
        // explicitly hang exports on window, so we have to reach into the
        // ESM module via dynamic import. Easiest: just import directly.
        const launchResult = await page.evaluate(async () => {
            try {
                // Use the same module path native_game.html itself imports from
                // (`./pkg/mtg_forge_rs.js` — relative to the served root,
                // which is `web/`, so the absolute URL is `/pkg/...`).
                const mod = await import('/pkg/mtg_forge_rs.js');
                if (!mod.default) {
                    return { ok: false, error: 'No default init export' };
                }
                if (typeof mod.launch_game_session !== 'function') {
                    return { ok: false, error: `launch_game_session not exported (typeof=${typeof mod.launch_game_session})` };
                }
                if (typeof mod.tui_tick !== 'function') {
                    return { ok: false, error: `tui_tick not exported` };
                }

                // Init wasm (idempotent — native_game.html already initialised).
                if (!mod.__inited) {
                    await mod.default();
                    mod.__inited = true;
                }

                const cardDb = new mod.WasmCardDatabase();

                // Load deck index (deck names + metadata).
                const decksResp = await fetch('./data/decks.bin');
                if (!decksResp.ok) {
                    return { ok: false, error: `decks.bin fetch failed: ${decksResp.status}` };
                }
                cardDb.load_decks(new Uint8Array(await decksResp.arrayBuffer()));

                // Load full card definitions so any deck works.
                // (native_game.html lazily loads per-deck packs, but for this isolated
                // smoke test it's simpler to grab the whole cards.bin once.)
                const cardsResp = await fetch('./data/cards.bin');
                if (!cardsResp.ok) {
                    return { ok: false, error: `cards.bin fetch failed: ${cardsResp.status}` };
                }
                cardDb.load_cards(new Uint8Array(await cardsResp.arrayBuffer()));

                // Tokens too — some decks (avatar set) create clue/food
                // tokens at run-time and the game crashes without these.
                try {
                    const tokensResp = await fetch('./data/tokens.bin');
                    if (tokensResp.ok) {
                        cardDb.load_tokens(new Uint8Array(await tokensResp.arrayBuffer()));
                    }
                } catch (_) { /* optional */ }

                const names = JSON.parse(cardDb.get_deck_names_json());
                if (!names.length) {
                    return { ok: false, error: 'no decks loaded' };
                }
                // Pick the first deck whose cards are all in the database, in
                // case load_cards.bin doesn't actually cover everything.
                let deck = names[0];
                for (const candidate of names) {
                    if (cardDb.get_missing_cards_for_deck(candidate).length === 0) {
                        deck = candidate;
                        break;
                    }
                }

                // Heuristic vs Heuristic with seed 42 — fully self-driving so
                // we can advance via tui_run_turn() without human input.
                mod.launch_game_session(
                    cardDb,
                    deck,
                    deck,
                    20,
                    42n,
                    mod.WasmControllerType.Heuristic,
                    mod.WasmControllerType.Heuristic,
                );

                // Capture initial state from the view model.
                const initialJson = mod.tui_get_gui_view_model_json();
                let initial;
                try { initial = JSON.parse(initialJson); }
                catch (e) { return { ok: false, error: `view model JSON parse: ${e.message}\n${initialJson.slice(0, 200)}` }; }

                // Drive 30 ticks of run_turn — heuristic-vs-heuristic should
                // happily advance phases / turns without any UI tick.
                for (let i = 0; i < 30; i++) {
                    mod.tui_run_turn();
                }

                const finalJson = mod.tui_get_gui_view_model_json();
                let finalVm;
                try { finalVm = JSON.parse(finalJson); }
                catch (e) { return { ok: false, error: `final view model JSON parse: ${e.message}` }; }

                // Also exercise tui_tick() — toggle auto-run on, tick, and
                // confirm the boolean return value plumbing works.
                mod.tui_toggle_auto();
                const tick1 = mod.tui_tick();
                const tick2 = mod.tui_tick();

                return {
                    ok: true,
                    initialTurn: initial?.turn_number,
                    finalTurn: finalVm?.turn_number,
                    initialPhase: initial?.phase,
                    finalPhase: finalVm?.phase,
                    statusText: finalVm?.status_text,
                    gameOver: finalVm?.game_over,
                    tick1,
                    tick2,
                    tickType: typeof tick1,
                };
            } catch (e) {
                // wasm-bindgen exceptions may be plain JsValue without
                // standard Error properties — stringify defensively.
                let errStr;
                try { errStr = String(e); } catch (_) { errStr = '<unstringifiable>'; }
                return {
                    ok: false,
                    error: `${e?.name || 'Error'}: ${e?.message || errStr}\n${e?.stack || ''}`,
                };
            }
        });

        check('launch_game_session callable + drives game without ratzilla',
              launchResult.ok,
              launchResult.ok ? `initialTurn=${launchResult.initialTurn} → finalTurn=${launchResult.finalTurn}, phase ${launchResult.initialPhase} → ${launchResult.finalPhase}` : launchResult.error);

        if (launchResult.ok) {
            check('view model JSON contained turn_number',
                  Number.isInteger(launchResult.finalTurn),
                  `final turn_number = ${launchResult.finalTurn}`);

            check('view model JSON contained status_text',
                  typeof launchResult.statusText === 'string' && launchResult.statusText.length > 0,
                  `final status_text = ${launchResult.statusText?.slice(0, 80)}`);

            check('tui_tick returns a boolean',
                  launchResult.tickType === 'boolean',
                  `typeof tui_tick() = ${launchResult.tickType}, sample values: ${launchResult.tick1}, ${launchResult.tick2}`);

            check('game state advanced after run_turn calls (turn number grew)',
                  launchResult.finalTurn > launchResult.initialTurn,
                  `turn ${launchResult.initialTurn} → ${launchResult.finalTurn}`);
        }

        check('no browser pageerrors / console errors during the run',
              browserErrors.length === 0,
              browserErrors.length === 0 ? 'clean' : browserErrors.slice(0, 3).join(' | '));

    } finally {
        if (browser) await browser.close();
        if (httpServer) httpServer.kill();
    }

    if (failures.length === 0) {
        log('=== ALL TESTS PASSED ===');
        process.exit(0);
    } else {
        log(`=== FAILURES (${failures.length}) ===`);
        failures.forEach(f => log(`  - ${f}`));
        process.exit(1);
    }
})();
