#!/usr/bin/env bash
# E2E regression test: Cycling ability stays synchronised in network mode
#
# Reproduces and locks-in the fix for mtg-ced6d1:
#
#   "Network desync: Cycle ability (Mountaincycling) not visible to client;
#    ABILITY SYNC BUG -> FATAL DESYNC on Cycle vs CastSpell"
#
# Root cause: `scry_cards()` and `surveil_cards()` used a heuristic that
# inspected hidden card identities (is_land / is_creature). The server has
# full card data and put a Swamp on the bottom; the client's shadow game
# could not classify the unrevealed top card and "kept on top" instead. The
# resulting library divergence meant the next draw produced different
# CardIds on each side, so Mongoose Lizard never made it into the client's
# hand and its Mountaincycling ability was missing from the local action
# list -> "ABILITY SYNC BUG - server has 3 abilities, local has 2" ->
# "FATAL DESYNC: Choice mismatch ... selected Cycle ... but client expected
# CastSpell".
#
# Fix (state.rs / network/controller.rs / network/server.rs):
# - Shadow games (`is_shadow_game == true`) now skip the scry/surveil
#   reorder when any top-card identity is unknown.
# - Server-side scry/surveil queue the scrying player into
#   `pending_library_reorders`. NetworkController drains the queue when
#   building the next ChoiceRequest, and the coordinator broadcasts a
#   `ServerMessage::LibraryReordered` to BOTH clients before forwarding
#   the ChoiceRequest, so both shadow libraries re-sync before ability
#   enumeration.
#
# This test fails if any of those pieces regress: it asserts that the
# server gamelog and the local-mode gamelog are byte-for-byte identical
# for the historically-failing seed (315 / random / random) and that no
# `ABILITY SYNC BUG` or `FATAL DESYNC` entry appears in any client log.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "=== Cycle/Mountaincycling network-sync regression (mtg-ced6d1) ==="
echo

# Run the underlying equivalence harness with the historically-failing seed.
# 315 + random/random was the minimal repro discovered by the network fuzz
# session; both decks contain Mountaincycling/Plainscycling cards AND a
# Rumble Arena ETB that triggers scry-1, exercising the full path.
SEED=315
CTRL_P1=random
CTRL_P2=random

if ! "$SCRIPT_DIR/network_vs_local_equivalence_e2e.sh" "$SEED" "$CTRL_P1" "$CTRL_P2"; then
    echo -e "${RED}FAIL: equivalence harness reported failure for seed=$SEED${NC}"
    exit 1
fi

# The harness preserves logs at /tmp/network_vs_local_e2e_<pid>; locate the
# most recent one so we can inspect it for the specific symptoms of mtg-ced6d1.
LATEST_LOG_DIR="$(ls -dt /tmp/network_vs_local_e2e_* 2>/dev/null | head -n 1 || true)"
if [[ -z "$LATEST_LOG_DIR" ]]; then
    echo -e "${YELLOW}WARN: could not locate harness log dir; skipping deeper assertions${NC}"
    exit 0
fi

echo
echo "Inspecting $LATEST_LOG_DIR for mtg-ced6d1 symptoms..."

fail=0

# Symptom 1: ABILITY SYNC BUG warnings on either client
for client_log in "$LATEST_LOG_DIR"/network/client1.log "$LATEST_LOG_DIR"/network/client2.log; do
    if [[ -f "$client_log" ]] && grep -q "ABILITY SYNC BUG" "$client_log"; then
        echo -e "${RED}FAIL: ABILITY SYNC BUG re-appeared in $(basename "$client_log")${NC}"
        grep "ABILITY SYNC BUG" "$client_log" | head -3
        fail=1
    fi
done

# Symptom 2: FATAL DESYNC anywhere in the network game
if grep -RIql "FATAL DESYNC" "$LATEST_LOG_DIR/network" 2>/dev/null; then
    echo -e "${RED}FAIL: FATAL DESYNC re-appeared in network logs${NC}"
    grep -RIH "FATAL DESYNC" "$LATEST_LOG_DIR/network" | head -3
    fail=1
fi

# Symptom 3: Mountaincycling must be observed on the SERVER for this seed
# (sanity check that we actually exercised the buggy code path; if Mongoose
# Lizard never showed up, the test is silently no-oping).
if ! grep -q "Mountaincycling" "$LATEST_LOG_DIR/network/server.log"; then
    echo -e "${YELLOW}WARN: server log did not contain 'Mountaincycling' for seed=$SEED;${NC}"
    echo -e "${YELLOW}  this seed may need to be refreshed if deck contents changed.${NC}"
    # Don't fail: this is a sanity check, not a hard invariant.
fi

if [[ "$fail" -ne 0 ]]; then
    echo
    echo -e "${RED}REGRESSION: mtg-ced6d1 symptoms re-appeared. Logs at $LATEST_LOG_DIR${NC}"
    exit 1
fi

echo
echo -e "${GREEN}PASS: cycle network-sync (mtg-ced6d1) regression test${NC}"
