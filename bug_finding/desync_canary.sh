#!/usr/bin/env bash
# ============================================================================
# COMPREHENSIVE DESYNC REGRESSION CANARY  (opt-in heavy; NOT in `make validate`)
# ============================================================================
#
# WHY THIS EXISTS
#   A predecessor network-architecture agent validated a new design against a
#   PARTIAL oracle (a lockstep test with NO cycling cards) and declared it
#   "native green". The FULL local-vs-network equivalence sweep later revealed a
#   REAL pre-existing native desync on a cycling -> library-search -> shuffle
#   path (the mtg-420 class). LESSON: partial coverage hides desyncs.
#
#   This canary is the BROAD net the default `make validate` is deliberately
#   NOT. Default validate runs exactly ONE deck pair (the avatar-draft pair) at
#   ONE pinned seed (3) across the three controllers -- fast and stable, but
#   narrow. This canary sweeps the historically-dangerous mechanics broadly so
#   that any future network architecture (e.g. the netarch minimal-protocol
#   prototype) -- or a regression that lands on `integration` -- is caught
#   before it is trusted.
#
# WHAT IT ASSERTS  (the canonical desync oracle, reused -- not reinvented)
#   For every (deck-pair, seed, controller) it runs the SAME game twice:
#     LOCAL   = one process, two AIs.
#     NETWORK = `mtg server` + two `mtg connect` clients over loopback,
#               with --network-debug (strict reveal validation + state hashing).
#   The server's authoritative [GAMELOG ...] stream MUST be byte-identical to
#   the local game's. ANY divergence is a desync / information-leak (a
#   controller that decides differently on full state vs shadow state) and is
#   ALWAYS FATAL per docs/NETWORK_ARCHITECTURE.md. All of this is implemented by
#   bug_finding/fuzz_determinism_netequiv.sh --invariant equivalence; this
#   script is a THIN corpus+policy wrapper around it (DRY -- no second copy of
#   the server/client orchestration or the gamelog comparison).
#
# COVERAGE vs the default validate equiv legs (we EXTEND, not duplicate):
#   default validate  : avatar pair, seed 3, {heuristic,random,zero}      (3 games)
#   this canary GREEN  : avatar pair + monored mirror + counterspells mirror
#                        (all three controllers) + rogerbrand mirror HEURISTIC
#                        (combat two-choice + All Hallow's Eve mass-resurrection;
#                        the mtg-u3dwj/mtg-d62r3 fix), BROAD seed ranges.
#                        (cycling/search/shuffle, burn/combat-damage,
#                         counter/stack-interaction, in-resolution draw-then-discard)
#   this canary KNOWN-RED: rogerbrand mirror random/zero only (mtg-586 load-flaky
#                        network-server nondeterminism). See the KNOWN_RED note below.
#
# THE GREEN GATE vs THE KNOWN-RED TIER  (honest; no faked green)
#   GREEN corpus  -> drives the exit code. If ANY green leg diverges, the canary
#                    FAILS (exit 1) and prints the captured reproducer. The
#                    green corpus is restricted to combos that are
#                    DETERMINISTICALLY green in isolation, matching the project
#                    policy that gating legs must be reliably reproducible.
#   KNOWN_RED tier -> run, captured, and reported LOUDLY every time, but does NOT
#                    drive the exit code. These are PRE-EXISTING, ALREADY-TRACKED
#                    desyncs (see issue refs in the table). This is XFAIL, NOT a
#                    silent exclusion: the legs run, their divergences are
#                    printed and captured under debug/, and if a known-red leg
#                    ever unexpectedly PASSES the canary says so (the bug may be
#                    fixed -> promote it into the green corpus + update the
#                    baseline). The default validate gate likewise excludes the
#                    randomized rogerbrand equivalence sweep precisely because it
#                    is not deterministically green (mtg-586 history); we surface
#                    it here instead of pretending it is covered.
#
# BASELINE (fix-allhallows-eve on integration, 2026-06-04): the GREEN corpus is
#   all-green, NOW INCLUDING the rogerbrand-mirror HEURISTIC leg. That leg used
#   to diverge DETERMINISTICALLY (377-line local-vs-network gamelog diff, 3/3 in
#   isolation, Turn 8 M2): the Bazaar-of-Baghdad "draw two, then discard three"
#   in-resolution discard was decided on the network client's shadow BEFORE the
#   just-drawn cards' reveals (carried in the discard ChoiceRequest's buffer) were
#   materialised, so the heuristic discarded the wrong cards — an
#   information-independence desync. FIXED (mtg-u3dwj / mtg-d62r3): the DiscardCards
#   handler now receives the ChoiceRequest before syncing (prepare -> sync ->
#   decide), so rogerbrand heuristic seeds 1-4 are now 5/5-deterministically green
#   and promoted into the gate above. The rogerbrand random/zero legs remain
#   KNOWN_RED (mtg-586-class load-flaky network-server nondeterminism; cf. the
#   WASM-shadow expressions mtg-589 / mtg-609). The GREEN corpus MUST NOT regress.
#
# USAGE
#   bug_finding/desync_canary.sh             # full canary (green gate + known-red report)
#   bug_finding/desync_canary.sh --green-only  # only the green gate (skip known-red report)
#   bug_finding/desync_canary.sh --quick       # ~halved seed ranges (smoke)
#   make validate-desync-canary              # = full canary via the Makefile
#
#   Run it under per-run cgroup isolation when other validates/agents share the
#   host (recommended on this dev harness):
#       systemd-run --user --scope -- make validate-desync-canary
#
# EXIT CODES
#   0  -- every GREEN leg passed (known-red legs are informational).
#   1  -- at least one GREEN leg diverged (a regression -> see captured repro).
#   2  -- environment/usage error (no binary, no cardsfolder, bad flag).
#
# OUTPUT
#   Per-entry sweep logs + any divergence captures land under
#   debug/desync_canary_<timestamp>/ (debug/ is gitignored). Each divergence
#   capture carries a REPRODUCER.txt with the exact single-combo command.
# ============================================================================
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

SWEEP="$REPO_ROOT/bug_finding/fuzz_determinism_netequiv.sh"
if [ ! -x "$SWEEP" ]; then
    echo "ERROR: heavy sweep not found/executable at $SWEEP" >&2
    exit 2
fi

# ----- Options --------------------------------------------------------------
GREEN_ONLY=0
QUICK=0
ALL_CONTROLLERS="heuristic random zero"
while [ $# -gt 0 ]; do
    case "$1" in
        --green-only) GREEN_ONLY=1; shift;;
        --quick) QUICK=1; shift;;
        -h|--help) sed -n '2,90p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0;;
        *) echo "ERROR: unknown option '$1' (try --help)" >&2; exit 2;;
    esac
done

# Halve the seed ranges in --quick mode (smoke). The known-divergent seeds
# (monored 13, counterspells 5, rogerbrand 3) stay inside every range below.
qseeds() { if [ "$QUICK" -eq 1 ]; then echo $(( ($1 + 1) / 2 )); else echo "$1"; fi; }

TS="$(date +%Y%m%d_%H%M%S)"
OUT_ROOT="$REPO_ROOT/debug/desync_canary_$TS"
mkdir -p "$OUT_ROOT"

# ----- Corpus tables --------------------------------------------------------
# Each entry: LABEL | DECK_GLOB | PAIR_MODE | START_SEED | N_SEEDS | CONTROLLERS
#
# GREEN corpus: deterministically-green dangerous mechanics. Drives exit code.
#   - avatar pair        : cycling / typecycling -> library search -> shuffle
#                          (the mtg-420 class the partial oracle missed).
#   - monored mirror     : fast burn + combat-damage assignment (the historical
#                          monored seed=13 case; range covers it).
#   - counterspells mirror: counter / stack-interaction, long control games
#                          (historical counterspells seed=5 case; EXPENSIVE
#                          ~50s/combo so its range is intentionally shorter).
GREEN_CORPUS=(
  "avatar-cycling|decks/booster_draft/avatar/ryan_avatar_draft.dck decks/booster_draft/avatar/gabriel_avatar_draft.dck|all|1|$(qseeds 6)|$ALL_CONTROLLERS"
  "monored-burn|decks/monored.dck|self|11|$(qseeds 6)|$ALL_CONTROLLERS"
  "counterspells-stack|decks/counterspells.dck|self|3|$(qseeds 4)|$ALL_CONTROLLERS"
  "rogerbrand-allhallows-heuristic|decks/old_school/01_rogue_rogerbrand.dck|self|1|$(qseeds 4)|heuristic"
)

# KNOWN_RED tier: PRE-EXISTING, tracked desyncs. Reported + captured, NOT gating.
#   - rogerbrand mirror random/zero : combat two-choice + All Hallow's Eve
#                          mass-resurrection. The deterministic HEURISTIC seed=3
#                          divergence (the Bazaar-of-Baghdad in-resolution
#                          draw-then-discard shadow-sync-ordering desync, mtg-u3dwj
#                          / mtg-d62r3) was FIXED 2026-06-04 and PROMOTED into the
#                          GREEN corpus above. The remaining random/zero legs on
#                          rogerbrand stay here: they are mtg-586-class load-flaky
#                          (network-server nondeterminism, NOT a same-game desync)
#                          and must NOT be promoted into any gate until that is
#                          root-caused. Related WASM-shadow expressions: mtg-589 /
#                          mtg-609.
KNOWN_RED_CORPUS=(
  "rogerbrand-allhallows-rand-zero|decks/old_school/01_rogue_rogerbrand.dck|self|1|$(qseeds 4)|random zero"
)

# ----- Runner ---------------------------------------------------------------
# Run one corpus entry through the heavy equivalence sweep; returns its exit
# code (0 = all combos passed, 1 = >=1 divergence, 2 = env error). Tees a
# per-entry log under OUT_ROOT and points the sweep's own captures there too.
run_entry() {
    local label="$1" glob="$2" mode="$3" start="$4" nseeds="$5" ctrls="$6"
    local entry_out="$OUT_ROOT/$label"
    mkdir -p "$entry_out"
    echo "------------------------------------------------------------"
    echo ">>> $label : decks='$glob' pair=$mode seeds=$start..$((start+nseeds-1)) ctrls='$ctrls'"
    echo "------------------------------------------------------------"
    "$SWEEP" --invariant equivalence \
        --decks "$glob" --pair-mode "$mode" \
        --start-seed "$start" --seeds "$nseeds" \
        --controllers "$ctrls" \
        --out "$entry_out" 2>&1 | tee "$entry_out/sweep.log"
    return "${PIPESTATUS[0]}"
}

echo "============================================================"
echo " DESYNC REGRESSION CANARY"
echo "============================================================"
echo "  repo        : $REPO_ROOT"
echo "  commit      : $(git rev-parse --short HEAD 2>/dev/null || echo '?')"
echo "  quick mode  : $QUICK    green-only: $GREEN_ONLY"
echo "  out         : $OUT_ROOT"
echo "  controllers : $ALL_CONTROLLERS"
echo "============================================================"

CANARY_START=$(date +%s)
GREEN_FAILS=()
for entry in "${GREEN_CORPUS[@]}"; do
    IFS='|' read -r label glob mode start nseeds ctrls <<< "$entry"
    if ! run_entry "$label" "$glob" "$mode" "$start" "$nseeds" "$ctrls"; then
        GREEN_FAILS+=("$label")
    fi
done

KNOWN_RED_RESULTS=()
if [ "$GREEN_ONLY" -eq 0 ]; then
    for entry in "${KNOWN_RED_CORPUS[@]}"; do
        IFS='|' read -r label glob mode start nseeds ctrls <<< "$entry"
        echo
        echo "### KNOWN-RED (informational, XFAIL -- pre-existing tracked desync) ###"
        if run_entry "$label" "$glob" "$mode" "$start" "$nseeds" "$ctrls"; then
            KNOWN_RED_RESULTS+=("$label: UNEXPECTEDLY GREEN -- the tracked desync may be FIXED; promote to green corpus + update baseline")
        else
            KNOWN_RED_RESULTS+=("$label: diverged as expected (pre-existing tracked desync; see $OUT_ROOT/$label/)")
        fi
    done
fi

CANARY_END=$(date +%s)
echo
echo "============================================================"
echo " CANARY SUMMARY  (elapsed $((CANARY_END - CANARY_START))s)"
echo "============================================================"
echo "  GREEN corpus entries : ${#GREEN_CORPUS[@]}"
if [ "${#GREEN_FAILS[@]}" -eq 0 ]; then
    echo "  GREEN gate           : ALL GREEN ✓"
else
    echo "  GREEN gate           : ${#GREEN_FAILS[@]} REGRESSION(S) ✗ -> ${GREEN_FAILS[*]}"
    echo "                         captures + REPRODUCER.txt under $OUT_ROOT/<label>/FAIL_*"
fi
if [ "$GREEN_ONLY" -eq 0 ]; then
    echo "  KNOWN-RED (XFAIL)    :"
    for r in "${KNOWN_RED_RESULTS[@]}"; do echo "    - $r"; done
fi
echo "  per-entry sweep csv  : $OUT_ROOT/<label>/summary.csv"
echo "============================================================"

if [ "${#GREEN_FAILS[@]}" -gt 0 ]; then
    echo "RESULT: FAIL -- a deterministically-green desync canary leg regressed."
    exit 1
fi
echo "RESULT: PASS -- green corpus is desync-free (known-red legs are informational)."
exit 0
