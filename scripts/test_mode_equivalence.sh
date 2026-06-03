#!/usr/bin/env bash
# Run the agent_play.py mode-equivalence test suite.
#
# This script wraps `pytest agentplay/test_mode_equivalence.py` so it can be
# called as a Makefile step or as a standalone command, and additionally
# emits a human-readable side-by-side comparison of the recorded action
# streams and game logs from each driver.
#
# Usage:
#   ./scripts/test_mode_equivalence.sh                # native drivers only
#   AGENTPLAY_TEST_WASM=1 ./scripts/test_mode_equivalence.sh   # also WASM driver
#
# Environment variables:
#   AGENTPLAY_TEST_WASM=1              -- enable the WASM driver E2E case
#   AGENTPLAY_TEST_WASM_DECK=foo.dck   -- override the WASM-exported deck
#                                         (default: decks/old_school2/ur_burn.dck)
#   CARDSFOLDER=/path/to/cardsfolder   -- override card definitions path

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

# Resolve a CARDSFOLDER if the user didn't set one. The persistent worktree
# may have a broken `cardsfolder` symlink (forge-java not initialised);
# fall back to a sibling checkout's cardsfolder if available.
if [ -z "${CARDSFOLDER:-}" ]; then
    for candidate in \
        "$REPO_ROOT/cardsfolder" \
        "$REPO_ROOT/forge-java/forge-gui/res/cardsfolder"; do
        if [ -d "$candidate/a" ]; then
            export CARDSFOLDER="$candidate"
            break
        fi
    done
fi

if [ -z "${CARDSFOLDER:-}" ]; then
    echo "Error: no usable CARDSFOLDER found." >&2
    echo "Run 'git submodule update --init forge-java' or set CARDSFOLDER=...
    " >&2
    exit 1
fi

echo "=== agent_play.py mode-equivalence tests ==="
echo "  CARDSFOLDER=$CARDSFOLDER"
echo "  AGENTPLAY_TEST_WASM=${AGENTPLAY_TEST_WASM:-0}"
echo

# Run the pytest suite first (this is the authoritative pass/fail gate).
python3 -m pytest agentplay/test_mode_equivalence.py -v

# Generate a human-readable comparison artefact for ad-hoc debugging.
# This isn't asserting anything — it just gives the developer a quick
# eyeball view of how stop-and-go vs persistent vs WASM compare on a
# concrete game.
COMPARE_DIR="$(mktemp -d -t agentplay-compare-XXXXXX)"
trap 'rm -rf "$COMPARE_DIR"' EXIT
echo
echo "=== ad-hoc comparison (game artefacts in $COMPARE_DIR) ==="

run_driver() {
    local driver="$1"
    local deck="$2"
    local game_dir="$COMPARE_DIR/$driver"
    echo "--- $driver ---"
    set +e
    python3 agentplay/agent_game.py \
        --mock --seed 42 --max-turns 4 \
        --game-dir="$game_dir" \
        --driver="$driver" \
        --mode=random-vs-random \
        -- "$deck" "$deck" >"$game_dir.log" 2>&1
    local rc=$?
    set -e
    if [ $rc -ne 0 ] && [ $rc -ne 2 ]; then
        echo "  driver $driver exited rc=$rc; tail log:"
        tail -10 "$game_dir.log"
        return $rc
    fi
    echo "  rc=$rc  p1_actions=$(grep -cv '^pass$' "$game_dir/p1_choices.txt" 2>/dev/null || echo 0)" \
         "p2_actions=$(grep -cv '^pass$' "$game_dir/p2_choices.txt" 2>/dev/null || echo 0)" \
         "log_lines=$(wc -l <"$game_dir/game.log" 2>/dev/null || echo 0)"
}

run_driver "stop-and-go" "decks/simple_bolt.dck"
run_driver "persistent" "decks/simple_bolt.dck"

if [ "${AGENTPLAY_TEST_WASM:-0}" = "1" ]; then
    wasm_deck="${AGENTPLAY_TEST_WASM_DECK:-decks/old_school2/ur_burn.dck}"
    run_driver "wasm" "$wasm_deck"
fi

echo
echo "✓ mode-equivalence script completed"
