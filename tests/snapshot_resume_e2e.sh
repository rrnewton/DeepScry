#!/usr/bin/env bash
# E2E test for `mtg resume` snapshot/restore capability.
#
# Verifies that:
#   1. `mtg tui --stop-on-choice N --snapshot-output S` writes a usable snapshot.
#   2. `mtg resume S` can pick up from that snapshot and run to completion
#      WITHOUT panicking, in BOTH bincode (default) and JSON snapshot formats.
#   3. The end-of-game outcome of a stop-and-resume run is semantically
#      identical to a single uninterrupted run with the same seed/decks/
#      controllers (deep state comparison via `scripts/diff_gamestate.py`,
#      ignoring non-semantic bookkeeping like the undo log and the
#      mana-cache version counter).
#   4. Resuming with `--override-p1` / `--override-p2` swaps in a different
#      controller without panicking. (Outcome is allowed to diverge here —
#      we just check the engine doesn't crash.)
#
# This test was added after discovering a `Cache exists after rebuild`
# panic during resume — the `mana_caches` field on `GameState` is
# `#[serde(skip)]` so it deserializes empty, and the next call into
# `ManaEngine` panicked. See `GameState::ensure_mana_cache` /
# `ensure_mana_caches_for_all_players`.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/test_helpers.sh"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

echo "=== mtg resume snapshot/restore e2e test ==="
echo

ensure_mtg_binary

cd "$WORKSPACE_ROOT"

# Working directory for snapshot/gamestate artifacts. Use mktemp so parallel
# invocations of `make validate` don't clobber each other.
WORK_DIR="$(mktemp -d -t mtg_resume_e2e_XXXXXX)"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

DECK="$WORKSPACE_ROOT/decks/grizzly_bears.dck"
SEED=42

if [ ! -f "$DECK" ]; then
    echo -e "${RED}Error: required test deck not found at $DECK${NC}"
    exit 1
fi

OVERALL_PASS=0
OVERALL_FAIL=0
FAILED_CASES=()

mark_pass() {
    OVERALL_PASS=$((OVERALL_PASS + 1))
    echo -e "${GREEN}  ✓ $1${NC}"
}
mark_fail() {
    OVERALL_FAIL=$((OVERALL_FAIL + 1))
    FAILED_CASES+=("$1")
    echo -e "${RED}  ✗ $1${NC}"
}

# --- Phase 1: Baseline (single uninterrupted game, JSON final state) -------
BASELINE_FINAL="$WORK_DIR/baseline.gamestate"
BASELINE_LOG="$WORK_DIR/baseline.log"
echo "[Phase 1] Baseline: heuristic vs heuristic, seed=$SEED, full game ..."
if ! run_mtg_with_timeout 60 tui \
        "$DECK" "$DECK" \
        --p1 heuristic --p2 heuristic \
        --seed "$SEED" \
        --tag-gamelogs \
        --save-final-gamestate "$BASELINE_FINAL" \
        --json \
        > "$BASELINE_LOG" 2>&1; then
    echo -e "${RED}Baseline game failed:${NC}"
    tail -50 "$BASELINE_LOG"
    exit 1
fi
if [ ! -s "$BASELINE_FINAL" ]; then
    echo -e "${RED}Baseline final gamestate not written to $BASELINE_FINAL${NC}"
    tail -30 "$BASELINE_LOG"
    exit 1
fi
BASELINE_TURNS_LINE="$(grep -E "Turns played:" "$BASELINE_LOG" | head -1 || echo "")"
BASELINE_TURNS="$(echo "$BASELINE_TURNS_LINE" | grep -oE '[0-9]+' | head -1)"
echo "       baseline: $BASELINE_TURNS_LINE"
echo "       final gamestate JSON: $(wc -c < "$BASELINE_FINAL") bytes"

# Helper: run one stop+resume cycle and check determinism.
#
# Args:
#   $1 - label (for error messages)
#   $2 - stop-on-choice argument (e.g. "3" or "5:p1")
#   $3 - snapshot format flag ("" for bincode, "--json" for JSON)
#   $4 - "deep" or "smoke":
#        deep  = save final gamestate as JSON and run diff_gamestate.py
#        smoke = just check that resume runs to completion + turn count matches
run_stop_resume_case() {
    local label="$1"
    local stop_at="$2"
    local fmt_flag="$3"
    local check_mode="$4"

    local sanitized
    sanitized="$(echo "$label" | tr -c 'A-Za-z0-9_' '_')"

    local snap_ext="snapshot"
    [ -n "$fmt_flag" ] && snap_ext="snapshot.json"

    local snap="$WORK_DIR/${sanitized}.${snap_ext}"
    local seg1_log="$WORK_DIR/${sanitized}.seg1.log"
    local seg2_log="$WORK_DIR/${sanitized}.seg2.log"

    # Segment 1: play, stop, snapshot
    local seg1_args=(tui "$DECK" "$DECK"
        --p1 heuristic --p2 heuristic
        --seed "$SEED"
        --tag-gamelogs
        --stop-on-choice "$stop_at"
        --snapshot-output "$snap")
    [ -n "$fmt_flag" ] && seg1_args+=("$fmt_flag")

    if ! run_mtg_with_timeout 60 "${seg1_args[@]}" > "$seg1_log" 2>&1; then
        mark_fail "$label (segment 1: stop-on-choice)"
        echo "      see $seg1_log"
        tail -30 "$seg1_log"
        return
    fi
    if [ ! -s "$snap" ]; then
        mark_fail "$label (segment 1: snapshot file empty/missing)"
        echo "      see $seg1_log"
        return
    fi

    # Segment 2: resume + run to game end
    local seg2_args=(resume "$snap" --tag-gamelogs)
    local resume_final=""
    if [ "$check_mode" = "deep" ]; then
        resume_final="$WORK_DIR/${sanitized}.final.gamestate"
        seg2_args+=(--save-final-gamestate "$resume_final" --json)
    elif [ -n "$fmt_flag" ]; then
        # smoke + json: passing --json is fine but we won't compare the file
        seg2_args+=("$fmt_flag")
    fi

    if ! run_mtg_with_timeout 60 "${seg2_args[@]}" > "$seg2_log" 2>&1; then
        mark_fail "$label (segment 2: resume crashed/failed)"
        echo "      see $seg2_log"
        tail -30 "$seg2_log"
        return
    fi
    local resume_turns_line resume_turns
    resume_turns_line="$(grep -E "Turns played:" "$seg2_log" | head -1 || echo "")"
    resume_turns="$(echo "$resume_turns_line" | grep -oE '[0-9]+' | head -1)"
    if [ -z "$resume_turns" ]; then
        mark_fail "$label (segment 2: game did not finish)"
        echo "      see $seg2_log"
        tail -30 "$seg2_log"
        return
    fi
    if [ "$resume_turns" != "$BASELINE_TURNS" ]; then
        mark_fail "$label (turn-count mismatch: baseline=$BASELINE_TURNS resume=$resume_turns)"
        return
    fi

    # Optional deep state check
    if [ "$check_mode" = "deep" ]; then
        if [ ! -s "$resume_final" ]; then
            mark_fail "$label (resume final gamestate not written)"
            return
        fi
        local diff_log="$WORK_DIR/${sanitized}.diff.log"
        if ! python3 "$WORKSPACE_ROOT/scripts/diff_gamestate.py" \
                "$BASELINE_FINAL" "$resume_final" > "$diff_log" 2>&1; then
            mark_fail "$label (final game state diverges from baseline)"
            echo "      diff_gamestate.py output:"
            sed 's/^/        /' "$diff_log" | head -40
            return
        fi
    fi

    mark_pass "$label"
}

# --- Phase 2: stop-and-resume at multiple stop points (JSON, deep check) ---
echo
echo "[Phase 2] Stop-and-resume determinism (JSON snapshots, deep state diff)"
for stop_at in 3 8 25; do
    run_stop_resume_case "json/stop@${stop_at}" "$stop_at" "--json" "deep"
done

# --- Phase 3: same coverage but with the binary (bincode) snapshot format --
# We can't trivially deep-diff bincode final states (they aren't JSON), so
# this phase is a smoke test: resume must run to completion and reach the
# same number of turns as the baseline. The JSON-format Phase 2 already
# proves deep determinism; here we just guard against bincode-specific
# regressions in serialization.
echo
echo "[Phase 3] Stop-and-resume in default bincode snapshot format (smoke)"
for stop_at in 3 8 25; do
    run_stop_resume_case "bincode/stop@${stop_at}" "$stop_at" "" "smoke"
done

# --- Phase 4: resume with controller override -----------------------------
# Resume the snapshot but swap controllers. The outcome can legitimately
# differ from the baseline, so we only assert the resume runs to a normal
# game-over (no panic, game terminates).
echo
echo "[Phase 4] Resume with --override-p2 (controller swap, smoke only)"
SNAP_OVERRIDE="$WORK_DIR/override.snapshot.json"
SEG1_OV_LOG="$WORK_DIR/override.seg1.log"
if ! run_mtg_with_timeout 60 tui \
        "$DECK" "$DECK" \
        --p1 heuristic --p2 heuristic \
        --seed "$SEED" \
        --tag-gamelogs \
        --stop-on-choice 8 \
        --snapshot-output "$SNAP_OVERRIDE" \
        --json \
        > "$SEG1_OV_LOG" 2>&1; then
    mark_fail "override: failed to write snapshot"
    tail -30 "$SEG1_OV_LOG"
else
    SEG2_OV_LOG="$WORK_DIR/override.seg2.log"
    if ! run_mtg_with_timeout 60 resume "$SNAP_OVERRIDE" \
            --override-p2 random \
            --override-seed-p2 12345 \
            --tag-gamelogs \
            --json \
            > "$SEG2_OV_LOG" 2>&1; then
        mark_fail "override: resume with --override-p2 random crashed/failed"
        tail -30 "$SEG2_OV_LOG"
    else
        if grep -qE "Game Over|wins!" "$SEG2_OV_LOG"; then
            mark_pass "override: resume with --override-p2 random reached game over"
        else
            mark_fail "override: resume completed but did not reach game over"
            tail -10 "$SEG2_OV_LOG"
        fi
    fi
fi

# --- Summary ---------------------------------------------------------------
echo
echo "=== Test Results Summary ==="
echo "  Passed: $OVERALL_PASS"
echo "  Failed: $OVERALL_FAIL"
if [ $OVERALL_FAIL -gt 0 ]; then
    echo -e "${RED}Failed cases:${NC}"
    for c in "${FAILED_CASES[@]}"; do
        echo "  - $c"
    done
    exit 1
fi
echo -e "${GREEN}✓ All snapshot/resume e2e cases passed!${NC}"
