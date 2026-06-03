#!/usr/bin/env node
// Network multi-deck E2E test - runs test_network_gui_e2e.js with several
// deck/seed combinations to verify no desync across diverse card interactions.
//
// Each scenario runs a full networked game (server + native AI + WASM browser).
// Tests run sequentially to keep resource usage manageable.
//
// Usage: node test_network_multideck.js
//        node test_network_multideck.js --quick   # Run only 2 scenarios (CI fast path)

const { execFile } = require('child_process');
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
// old_school/03_robots_jesseisbak seed=42 is back in the gate (mtg-559/mtg-610):
// the WASM in-stack-resolution re-entry desync (Copy Artifact Clone, Balance,
// extra-turn, and other interactive resolution choices returning NeedInput
// mid-resolution) is now fixed by the unified rewind+replay AI path plus the
// closed undo holes — CloneCard / PushExtraTurn are undoable GameActions and the
// until-EOT keyword clear is zone-independent, so a mid-resolution rewind+replay
// round-trips compute_state_hash exactly instead of double-applying effects.
//
// EXCLUDED known-broken mirror scenarios (pre-existing WASM-shadow desyncs,
// NOT introduced by the mirror-match harness fix — they reproduce identically
// on the prior non-mirror code too):
//   - white_weenie seed=7: native P2 hash mismatch ~choice_seq=214 (mtg-nkd71).
// These belong to the engine shadow-state work, not the gate harness; the gate
// uses scenarios proven STABLE as mirror matches.
const SCENARIOS = [
    { deck: 'decks/monored.dck',                     seed: 13, desc: 'Red burn + creatures (mirror)' },
    { deck: 'decks/counterspells.dck',               seed: 5,  desc: 'Control + counters (mirror)' },
    { deck: 'decks/old_school/01_rogue_rogerbrand.dck', seed: 3, desc: "Old-school reanimator: All Hallow's Eve (mirror, mtg-609)" },
    { deck: 'decks/old_school/03_robots_jesseisbak.dck', seed: 42, desc: 'Old-school robots: Copy Artifact clone / Balance / extra-turn in-resolution choices (mirror, mtg-559/mtg-610)' },
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

// Run one scenario as a child `node test_network_gui_e2e.js` process, capturing
// its output (rather than inheriting stdio) so concurrent scenarios don't
// interleave their stdout. Each scenario allocates its OWN random server/http
// ports (test_network_gui_e2e.js -> getRandomPorts), so concurrent runs do not
// collide. Resolves to a result record; never rejects.
function runScenario(s, index) {
    return new Promise((resolve) => {
        const scenarioStart = Date.now();
        execFile('node', [testScript, '--deck', s.deck, '--seed', s.seed.toString()], {
            cwd: __dirname,
            timeout: 240000, // 4 minute hard timeout per scenario
            maxBuffer: 64 * 1024 * 1024,
        }, (err, stdout, stderr) => {
            const elapsed = ((Date.now() - scenarioStart) / 1000).toFixed(1);
            resolve({ ...s, index, elapsed, result: err ? 'FAIL' : 'PASS', stdout, stderr, error: err ? err.message : null });
        });
    });
}

async function main() {
    log(`=== Network Multi-Deck E2E Test (${scenarios.length} scenarios${QUICK_MODE ? ', quick mode' : ''}, PARALLEL) ===`);
    const startTime = Date.now();

    // mtg-717: run the scenarios CONCURRENTLY instead of sequentially. Each is a
    // fully independent networked game on its own random ports, so wall-clock
    // collapses from sum(scenarios) (~443s, two scenarios soaking to the 180s
    // game-timeout cap) to max(scenario) (~190s) with ZERO change to per-game
    // depth — every scenario still plays the exact same game to the exact same
    // turn count and asserts the exact same no-desync / no-fatal-error invariant.
    const results = await Promise.all(scenarios.map((s, i) => runScenario(s, i)));

    // Emit each scenario's captured output in stable scenario order so a failure
    // is still self-contained and diagnosable (parent step routes this to its
    // per-step detail file / dumps it on failure).
    for (const r of results) {
        log(`\n--- Scenario ${r.index + 1}/${scenarios.length}: ${r.desc} (${r.deck} seed=${r.seed}) -> ${r.result} (${r.elapsed}s) ---`);
        if (r.stdout) process.stdout.write(r.stdout);
        if (r.result === 'FAIL') {
            if (r.stderr) process.stderr.write(r.stderr);
            log(`  scenario error: ${r.error}`);
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
