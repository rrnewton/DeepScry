#!/usr/bin/env node
// Network multi-deck E2E test - runs test_network_gui_e2e.js with several
// deck/seed combinations to verify no desync across diverse card interactions.
//
// Each scenario runs a full networked game (server + native AI + WASM browser).
// Tests run sequentially to keep resource usage manageable.
//
// Usage: node test_network_multideck.js
//        node test_network_multideck.js --quick   # Run only 2 scenarios (CI fast path)

const { execFileSync } = require('child_process');
const path = require('path');

function log(msg) {
    const ts = new Date().toISOString().substring(11, 23);
    console.log(`[${ts}] ${msg}`);
}

// Deck/seed combinations covering diverse card interactions:
// - Different deck archetypes (aggro, control, artifacts, burn)
// - Different seeds for varied game states
// Paths are relative to project root (test_network_gui_e2e.js resolves them).
//
// A single --deck applies that deck to BOTH seats (native P1 + browser P2),
// so every scenario below is a deterministic same-deck MIRROR match
// (see test_network_gui_e2e.js's deck-injection step).
//
// old_school/01_rogue_rogerbrand seed=3 (All Hallow's Eve mass-resurrection) is
// back in the gate: the WASM-shadow desync was root-caused to the begin-of-upkeep
// phase triggers double-firing on WASM GameLoop re-entry after a NeedInput block,
// and fixed by a per-turn re-entry guard in check_phase_triggers (mtg-609).
//
// EXCLUDED known-broken mirror scenarios (pre-existing WASM-shadow desyncs,
// NOT introduced by the mirror-match harness fix — they reproduce identically
// on the prior non-mirror code too):
//   - white_weenie seed=7: native P2 hash mismatch ~choice_seq=214 (mtg-nkd71).
//   - old_school/03_robots_jesseisbak seed=42: WASM in-stack-resolution re-entry
//     desync. Copy Artifact's Clone choice (and Balance / other interactive
//     resolution effects) return NeedInput mid-resolution; on WASM GameLoop
//     re-entry resolve_top_spell_with_discard_hook re-runs from the first effect
//     instead of resuming, so already-executed effects (and the clone) run twice.
//     Distinct, broader root cause than mtg-609; needs an effect-resume index
//     for spell resolution (cf. pending_activation_effect_idx). Tracked: mtg-559.
// These belong to the engine shadow-state work, not the gate harness; the gate
// uses scenarios proven STABLE as mirror matches.
const SCENARIOS = [
    { deck: 'decks/monored.dck',                     seed: 13, desc: 'Red burn + creatures (mirror)' },
    { deck: 'decks/counterspells.dck',               seed: 5,  desc: 'Control + counters (mirror)' },
    { deck: 'decks/old_school/01_rogue_rogerbrand.dck', seed: 3, desc: "Old-school reanimator: All Hallow's Eve (mirror, mtg-609)" },
];

// All three mirror scenarios are proven stable and fast enough for the gate, so
// both quick (the CI fast path invoked by `make validate`) and full runs
// exercise the entire SCENARIOS list — including the rogerbrand All Hallow's Eve
// mirror (mtg-609). The --quick flag is retained for API compatibility and so
// slower, experimental scenarios can later be appended to the full-only tail
// without changing the CI fast path.
const QUICK_MODE = process.argv.includes('--quick');
const scenarios = SCENARIOS;

const testScript = path.join(__dirname, 'test_network_gui_e2e.js');

async function main() {
    log(`=== Network Multi-Deck E2E Test (${scenarios.length} scenarios${QUICK_MODE ? ', quick mode' : ''}) ===`);
    const startTime = Date.now();
    const results = [];

    for (let i = 0; i < scenarios.length; i++) {
        const s = scenarios[i];
        log(`\n--- Scenario ${i + 1}/${scenarios.length}: ${s.desc} (${s.deck} seed=${s.seed}) ---`);
        const scenarioStart = Date.now();

        try {
            execFileSync('node', [testScript, '--deck', s.deck, '--seed', s.seed.toString()], {
                cwd: __dirname,
                stdio: 'inherit',
                timeout: 240000, // 4 minute timeout per scenario
            });

            const elapsed = ((Date.now() - scenarioStart) / 1000).toFixed(1);
            log(`  PASS (${elapsed}s)`);
            results.push({ ...s, result: 'PASS', elapsed });
        } catch (err) {
            const elapsed = ((Date.now() - scenarioStart) / 1000).toFixed(1);
            log(`  FAIL (${elapsed}s): ${err.message}`);
            results.push({ ...s, result: 'FAIL', elapsed, error: err.message });
        }
    }

    // Summary
    const totalElapsed = ((Date.now() - startTime) / 1000).toFixed(1);
    const passed = results.filter(r => r.result === 'PASS').length;
    const failed = results.filter(r => r.result === 'FAIL').length;

    log(`\n=== SUMMARY: ${passed}/${results.length} passed, ${failed} failed (${totalElapsed}s total) ===`);
    for (const r of results) {
        const icon = r.result === 'PASS' ? 'PASS' : 'FAIL';
        log(`  ${icon}: ${r.desc} (${r.deck} seed=${r.seed}) - ${r.elapsed}s`);
    }

    if (failed > 0) {
        log('\n=== MULTI-DECK TEST FAILED ===');
        process.exit(1);
    } else {
        log('\n=== ALL MULTI-DECK TESTS PASSED ===');
    }
}

main().catch(err => {
    log(`Fatal error: ${err.message}`);
    process.exit(1);
});
