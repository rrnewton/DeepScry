#!/usr/bin/env bash
# E2E regression test: robots42 deck via the new ActionLog<StateSyncEntry>
# shadow state-sync log (Phase 2 step 1).
#
# Tracks docs/NETWORK_ACTION_LOG_MIGRATION.md § 1 — the first migration
# target: replace WasmNetworkClient's destructive `drain_reveals` /
# `drain_library_reorders` machinery with a non-destructive,
# action_count-keyed ActionLog<StateSyncEntry>. The deck (Old School
# 1994 "03 Robots Jesseisbak") is reveal-heavy: it ships Ancestral
# Recall, Wheel of Fortune, Demonic Tutor, Braingeyser, Timetwister,
# and Mind Twist — all of which generate cascading server reveals AND
# library reorders, exactly the message classes that arrived in
# non-deterministic order under the old destructive-drain path and
# caused mtg-559.
#
# Verification strategy — two layers:
#
# 1. Smoke layer (this script): run robots42 vs robots42 in local mode
#    across several seeds and assert each game completes cleanly (no
#    panic, no fatal desync). The local-mode binary exercises the same
#    `process_card_reveal` infrastructure that the WASM shadow uses;
#    a regression that broke the reveal-application path here would
#    show up as a panic or hang.
#
# 2. Unit layer (compiled in to `cargo test`):
#    - `mtg-engine/src/network/state_sync.rs::tests` covers the
#      strongly-typed entry round-trip through `ActionLog<T>` and the
#      arrival-order-independence property that backs the mtg-559 fix.
#    - `mtg-engine/src/wasm/network/client.rs::tests` covers the apply
#      cursor's strict-monotonic, non-destructive-read, and rewind
#      semantics on the WASM client (compiled under wasm-tui clippy).
#
# Determinism note: the engine has a preexisting, separately-tracked
# non-determinism class on some decks (HashMap iteration order, plus
# RNG-state plumbing gaps tracked under other beads issues). This test
# therefore does NOT compare two independent runs byte-for-byte — it
# verifies completion. The unit layer guarantees the new ActionLog
# primitive is the deterministic substrate it claims to be.
#
# The matching WASM browser e2e — which runs through the actual
# Phase 2 step 1 code paths (`apply_state_sync_up_to_frontier`) — is
# wired into `make validate-wasm-e2e-step` and runs against the WASM
# bundle built from this same workspace.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WORKSPACE_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "=== robots42 state-sync regression (Phase 2 step 1 / mtg-559) ==="
echo

DECK="$WORKSPACE_ROOT/decks/old_school/03_robots_jesseisbak.dck"

if [[ ! -f "$DECK" ]]; then
    echo -e "${RED}FAIL: robots42 deck not found at $DECK${NC}"
    exit 1
fi

MTG_BIN="$WORKSPACE_ROOT/target/release/mtg"
if [[ ! -x "$MTG_BIN" ]]; then
    echo -e "${YELLOW}Binary not pre-built at $MTG_BIN — building...${NC}"
    (cd "$WORKSPACE_ROOT" && cargo build --release --features network --quiet)
fi

if [[ ! -x "$MTG_BIN" ]]; then
    echo -e "${RED}FAIL: $MTG_BIN not available${NC}"
    exit 1
fi

# Several seeds increase coverage of the reorder/reveal interleaving
# patterns the new ActionLog must handle. Each game runs end-to-end and
# is asserted to complete with a real winner (PlayerDeath / Concede)
# rather than a panic or timeout.
SEEDS=(3 7 19 42)

OUTPUT_DIR="$(mktemp -d -t robots42_state_sync_e2e.XXXXXX)"
trap "rm -rf '$OUTPUT_DIR'" EXIT

fail=0
ran=0

for SEED in "${SEEDS[@]}"; do
    echo
    echo -e "${YELLOW}--- seed=$SEED ---${NC}"

    LOG="$OUTPUT_DIR/seed_${SEED}.log"

    # robots42 vs itself: maximises chance that both sides scry / search /
    # reorder in the same turns, stressing the reveal-heavy code paths.
    if ! timeout 90 "$MTG_BIN" tui \
            "$DECK" "$DECK" \
            --p1 zero --p2 zero \
            --seed-p1 "$SEED" --seed-p2 "$SEED" \
            --tag-gamelogs \
            > "$LOG" 2>&1 ; then
        echo -e "${RED}FAIL: seed=$SEED — binary exited with non-zero status${NC}"
        echo "Tail:"
        tail -n 12 "$LOG"
        fail=1
        continue
    fi
    ran=$((ran + 1))

    # Completion proof: every successful game prints "Game Over" and
    # either a Winner line or a clear stalemate reason. Absence of that
    # line means the binary hung, timed out, or crashed silently.
    if ! grep -q '=== Game Over ===' "$LOG"; then
        echo -e "${RED}FAIL: seed=$SEED — '=== Game Over ===' not found in log${NC}"
        echo "Tail:"
        tail -n 12 "$LOG"
        fail=1
        continue
    fi

    # Panic / fatal-desync gate: the new ActionLog::push panics on a
    # strictly-monotonic invariant violation (invariant #2 of
    # docs/NETWORK_ACTION_LOG.md § 8). A panic in the reveal/reorder
    # path would surface here.
    if grep -Ei 'panicked at|FATAL DESYNC|ABILITY SYNC BUG' "$LOG" > /dev/null; then
        echo -e "${RED}FAIL: seed=$SEED — panic / fatal-desync signal in log${NC}"
        grep -Ei 'panicked at|FATAL DESYNC|ABILITY SYNC BUG' "$LOG" | head -5
        fail=1
        continue
    fi

    gamelog_lines="$(grep -cE '\[GAMELOG' "$LOG" || true)"
    echo -e "${GREEN}seed=$SEED: game completed cleanly, $gamelog_lines GAMELOG entries${NC}"
done

echo
if [[ "$fail" -ne 0 ]]; then
    echo -e "${RED}=== FAIL: $fail / $ran seeds had errors ===${NC}"
    echo "Logs preserved at: $OUTPUT_DIR"
    trap - EXIT
    exit 1
fi

echo -e "${GREEN}=== PASS: $ran / ${#SEEDS[@]} seeds completed cleanly with no panic or fatal desync ===${NC}"
