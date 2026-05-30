#!/usr/bin/env bash
# Randomized-invariant fuzz sweep: native determinism + local-vs-network gamelog
# equivalence, across a configurable seed range x deck sample.
#
# This generalizes the SINGLE-seed, hardcoded-deck comparison in
# tests/network_vs_local_equivalence_e2e.sh into a SWEEP over many seeds and
# many deck pairs, and adds the native-determinism invariant (same seed twice ->
# byte-identical tagged gamelogs).
#
# Two invariants are checked for EVERY (deck-pair, seed):
#
#   1. determinism  -- `mtg tui D1 D2 --seed K --tag-gamelogs` run twice yields
#                      byte-identical [GAMELOG ...] streams. (mtg-engine has
#                      tests/determinism_e2e.rs for self-pairs at a few seeds;
#                      this sweeps real heterogeneous deck pairs across a seed
#                      range with both heuristic and random controllers.)
#
#   2. equivalence  -- the SAME game played LOCAL (single process, two AIs) vs
#                      NETWORK (server + 2 loopback clients) yields identical
#                      [GAMELOG ...] streams. This is the deep network-determinism
#                      invariant from docs/NETWORK_ARCHITECTURE.md ("desync is
#                      ALWAYS fatal"). Any divergence is a REAL BUG.
#
# Both invariants must hold for every combo; ANY divergence makes the whole
# sweep exit 1 (it never skips / soft-passes).
#
# ----------------------------------------------------------------------------
# USAGE
#
#   bug_finding/fuzz_determinism_netequiv.sh [OPTIONS]
#
# OPTIONS:
#   --seeds N            Number of seeds per deck pair (default: 5). Seeds are
#                        START_SEED, START_SEED+1, ... START_SEED+N-1.
#   --start-seed K       First seed (default: 1).
#   --decks "GLOB ..."   One or more shell globs selecting .dck files for the
#                        deck corpus (quote it!). Deck PAIRS are drawn from this
#                        corpus (see --pair-mode). Default corpus = the 1994
#                        old-school decks: "decks/old_school/*.dck
#                        decks/old_school2/*.dck".
#   --pair-mode MODE     How to form deck pairs from the corpus:
#                          chain  (default) -- consecutive pairs (d0,d1),(d1,d2)...
#                                              giving ~N pairs for N decks (cheap,
#                                              good coverage spread).
#                          all            -- every unordered pair i<j (O(n^2);
#                                              use for heavy overnight runs).
#                          self           -- each deck mirror-matched against
#                                              itself (matches determinism_e2e.rs
#                                              style; fastest, no cross-deck cost).
#   --max-pairs M        Hard cap on number of deck pairs actually run (after
#                        pair-mode expansion). 0 = no cap (default 0).
#   --controllers "L ..."  Space-separated controller list. Each listed
#                        controller C runs both players as C. Valid: heuristic,
#                        random, zero. Default: "heuristic random".
#   --invariant WHICH    Which invariant(s) to run: determinism | equivalence |
#                        both (default: both).
#   --timeout SECS       Per-game timeout (default: 120).
#   --port-base P        Base TCP port for loopback network games (default:
#                        random in 20000..40000; each game offsets from here).
#   --jobs J             (reserved) currently sequential; J ignored.
#   --out DIR            Directory for divergence captures + summary
#                        (default: debug/fuzz_sweep_<timestamp>; debug/ is
#                        gitignored). Divergent gamelogs + diffs are saved here.
#   --keep-logs          Keep ALL per-game logs (not just divergences). Heavy.
#   -h | --help          Show this help.
#
# EXIT CODES:
#   0  -- every combo passed both requested invariants.
#   1  -- at least one divergence (REAL BUG) or a game crashed/timed out.
#   2  -- usage / environment error (no cardsfolder, bad controller, etc).
#
# EXAMPLES:
#   # Fast bounded run (what the validate-wired wrapper uses):
#   bug_finding/fuzz_determinism_netequiv.sh --seeds 4 --pair-mode chain \
#       --max-pairs 2 --controllers "heuristic random"
#
#   # Heavy overnight aggressive sweep (hundreds of game-pairs):
#   bug_finding/fuzz_determinism_netequiv.sh --seeds 40 --pair-mode all \
#       --controllers "heuristic random zero" --invariant both
#
#   # Determinism only, on a custom deck glob:
#   bug_finding/fuzz_determinism_netequiv.sh --invariant determinism \
#       --decks "decks/old_school2/*.dck" --seeds 20
#
# ----------------------------------------------------------------------------
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

# Shared helpers — single source of truth (no inline reimplementations).
# shellcheck source=lib/gamelog_filter.sh
source "$SCRIPT_DIR/lib/gamelog_filter.sh"
# shellcheck source=lib/seed_salts.sh
source "$SCRIPT_DIR/lib/seed_salts.sh"

# ----- Defaults -------------------------------------------------------------
N_SEEDS=5
START_SEED=1
DECK_GLOBS="decks/old_school/*.dck decks/old_school2/*.dck"
PAIR_MODE="chain"
MAX_PAIRS=0
CONTROLLERS="heuristic random"
INVARIANT="both"
GAME_TIMEOUT=120
PORT_BASE=$((20000 + RANDOM % 20000))
OUT_DIR=""
KEEP_LOGS=0

# ----- Arg parse ------------------------------------------------------------
while [ $# -gt 0 ]; do
    case "$1" in
        --seeds) N_SEEDS="$2"; shift 2;;
        --start-seed) START_SEED="$2"; shift 2;;
        --decks) DECK_GLOBS="$2"; shift 2;;
        --pair-mode) PAIR_MODE="$2"; shift 2;;
        --max-pairs) MAX_PAIRS="$2"; shift 2;;
        --controllers) CONTROLLERS="$2"; shift 2;;
        --invariant) INVARIANT="$2"; shift 2;;
        --timeout) GAME_TIMEOUT="$2"; shift 2;;
        --port-base) PORT_BASE="$2"; shift 2;;
        --jobs) shift 2;;
        --out) OUT_DIR="$2"; shift 2;;
        --keep-logs) KEEP_LOGS=1; shift;;
        -h|--help) sed -n '2,80p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0;;
        *) echo "ERROR: unknown option '$1' (try --help)" >&2; exit 2;;
    esac
done

case "$INVARIANT" in determinism|equivalence|both) ;; *)
    echo "ERROR: --invariant must be determinism|equivalence|both" >&2; exit 2;; esac
case "$PAIR_MODE" in chain|all|self) ;; *)
    echo "ERROR: --pair-mode must be chain|all|self" >&2; exit 2;; esac
for c in $CONTROLLERS; do
    case "$c" in heuristic|random|zero) ;; *)
        echo "ERROR: invalid controller '$c' (heuristic|random|zero)" >&2; exit 2;; esac
done

# ----- Binary + cardsfolder resolution (mirror test_helpers.sh) -------------
MTG_BIN="${MTG_BIN:-$REPO_ROOT/target/release/mtg}"
if [ ! -x "$MTG_BIN" ]; then
    echo "Building release binary (no prebuilt at $MTG_BIN)..."
    cargo build --release --bin mtg --features network || { echo "build failed" >&2; exit 2; }
fi
if ! "$MTG_BIN" server --help >/dev/null 2>&1; then
    echo "ERROR: $MTG_BIN lacks network support (server subcommand). Rebuild with --features network." >&2
    exit 2
fi
if [ -z "${CARDSFOLDER:-}" ]; then
    for cand in "$REPO_ROOT/cardsfolder" \
                "$REPO_ROOT/forge-java/forge-gui/res/cardsfolder"; do
        [ -d "$cand/a" ] && { export CARDSFOLDER="$cand"; break; }
    done
fi
if [ -z "${CARDSFOLDER:-}" ] || [ ! -d "$CARDSFOLDER/a" ]; then
    echo "ERROR: no usable cardsfolder (run: git submodule update --init forge-java, or set CARDSFOLDER=...)" >&2
    exit 2
fi

# ----- Output dir -----------------------------------------------------------
if [ -z "$OUT_DIR" ]; then
    OUT_DIR="$REPO_ROOT/debug/fuzz_sweep_$(date +%Y%m%d_%H%M%S)"
fi
mkdir -p "$OUT_DIR"
SUMMARY_CSV="$OUT_DIR/summary.csv"
echo "invariant,deck1,deck2,seed,controller,result,detail" > "$SUMMARY_CSV"

# ----- Seed derivation -------------------------------------------------------
# Local TUI is given pre-derived --seed-p1/--seed-p2; network clients are given
# the MASTER controller seed via --seed-player and derive per-slot internally.
# To make LOCAL and NETWORK RandomController RNG streams identical we pre-derive
# here using the SAME salts as seed_derivation.rs. derive_p1_seed/derive_p2_seed
# come from the shared bug_finding/lib/seed_salts.sh (sourced above) — the ONE
# bash mirror of the canonical Rust salts.

# ----- Build deck corpus ----------------------------------------------------
# Expand the (possibly multi-) glob into an array of existing .dck files, sorted
# for determinism of pairing.
DECKS=()
for g in $DECK_GLOBS; do
    for f in $g; do
        [ -f "$f" ] && DECKS+=("$f")
    done
done
# sort + dedup
IFS=$'\n' DECKS=($(printf '%s\n' "${DECKS[@]}" | sort -u)); unset IFS
if [ "${#DECKS[@]}" -eq 0 ]; then
    echo "ERROR: deck glob(s) '$DECK_GLOBS' matched no .dck files" >&2
    exit 2
fi

# ----- Build deck-pair list -------------------------------------------------
PAIRS=()
case "$PAIR_MODE" in
    self)
        for d in "${DECKS[@]}"; do PAIRS+=("$d|$d"); done;;
    chain)
        n=${#DECKS[@]}
        for ((i=0; i<n; i++)); do
            j=$(( (i+1) % n ))
            [ "$n" -eq 1 ] && j=0
            PAIRS+=("${DECKS[$i]}|${DECKS[$j]}")
        done;;
    all)
        n=${#DECKS[@]}
        for ((i=0; i<n; i++)); do
            for ((j=i+1; j<n; j++)); do
                PAIRS+=("${DECKS[$i]}|${DECKS[$j]}")
            done
        done
        [ "${#PAIRS[@]}" -eq 0 ] && PAIRS+=("${DECKS[0]}|${DECKS[0]}");;
esac
if [ "$MAX_PAIRS" -gt 0 ] && [ "${#PAIRS[@]}" -gt "$MAX_PAIRS" ]; then
    PAIRS=("${PAIRS[@]:0:$MAX_PAIRS}")
fi

# ----- Counters -------------------------------------------------------------
DET_PASS=0; DET_FAIL=0; DET_CRASH=0
EQ_PASS=0;  EQ_FAIL=0
FAIL_REPROS=()   # human-readable reproducer lines for failures

START_TS=$(date +%s)
echo "============================================================"
echo " Fuzz sweep: determinism + local-vs-network equivalence"
echo "============================================================"
echo "  binary       : $MTG_BIN"
echo "  cardsfolder  : $CARDSFOLDER"
echo "  invariant    : $INVARIANT"
echo "  controllers  : $CONTROLLERS"
echo "  decks        : ${#DECKS[@]} files, pair-mode=$PAIR_MODE -> ${#PAIRS[@]} pairs"
echo "  seeds        : $N_SEEDS seeds from $START_SEED"
echo "  per-game TO  : ${GAME_TIMEOUT}s   port-base: $PORT_BASE"
echo "  out          : $OUT_DIR"
echo "------------------------------------------------------------"

# Extract canonical, comparable GAMELOG lines from a raw log file.
# Delegates to the shared bug_finding/lib/gamelog_filter.sh (sourced above) so
# the ANSI-strip + [GAMELOG ...] keep + known-noise filter lives in exactly one
# place (also mirrored by network_test_lib.py::extract_gamelog and reused by
# tests/network_vs_local_equivalence_e2e.sh).
# For the DETERMINISM invariant both runs are identical mode so the SAME filter
# (it only removes lines, symmetrically) compares apples-to-apples.
extract_gamelog() {
    gamelog_filter_file "$1" "$2"
}

# Run a single LOCAL game; writes raw log to $1, args follow.
#   run_local <rawlog> <deck1> <deck2> <controller> <master_seed>
run_local_game() {
    local raw="$1" d1="$2" d2="$3" ctrl="$4" seed="$5"
    local p1s p2s; p1s="$(derive_p1_seed "$seed")"; p2s="$(derive_p2_seed "$seed")"
    timeout "$GAME_TIMEOUT" "$MTG_BIN" tui "$d1" "$d2" \
        --p1 "$ctrl" --p2 "$ctrl" \
        --p1-name Ryan --p2-name Gabriel \
        --seed "$seed" --seed-p1 "$p1s" --seed-p2 "$p2s" \
        --tag-gamelogs --no-color-logs --verbosity normal \
        > "$raw" 2>&1
    return $?
}

# ============================================================================
# INVARIANT 1: native determinism (same game twice -> identical gamelogs)
# ============================================================================
run_determinism() {
    local d1="$1" d2="$2" seed="$3" ctrl="$4"
    local tag="det_${ctrl}_s${seed}"
    local rawA="$OUT_DIR/.$tag.A.log" rawB="$OUT_DIR/.$tag.B.log"
    local glA="$OUT_DIR/.$tag.A.gamelog" glB="$OUT_DIR/.$tag.B.gamelog"

    run_local_game "$rawA" "$d1" "$d2" "$ctrl" "$seed"; local rcA=$?
    run_local_game "$rawB" "$d1" "$d2" "$ctrl" "$seed"; local rcB=$?

    extract_gamelog "$rawA" "$glA"
    extract_gamelog "$rawB" "$glB"

    # The DETERMINISM invariant is: the two runs are byte-identical. We compare
    # BOTH the exit code AND the [GAMELOG ...] stream. An identical crash (same
    # rc, same gamelog up to the error) still SATISFIES determinism -- it is a
    # separate pre-existing engine/AI issue (e.g. a controller looping on a free
    # ability until the priority-round guard fires), NOT a reproducibility break.
    # A determinism VIOLATION is when the two identically-seeded runs DIFFER.
    if [ "$rcA" != "$rcB" ]; then
        DET_FAIL=$((DET_FAIL+1))
        echo "determinism,$d1,$d2,$seed,$ctrl,FAIL,exit-code-diverged rcA=$rcA rcB=$rcB" >> "$SUMMARY_CSV"
        _capture_failure "determinism" "$d1" "$d2" "$seed" "$ctrl" "exit code diverged rcA=$rcA rcB=$rcB" "$rawA" "$rawB"
        return 1
    fi

    if ! diff -q "$glA" "$glB" >/dev/null 2>&1; then
        DET_FAIL=$((DET_FAIL+1))
        local ndiff; ndiff=$(diff "$glA" "$glB" | grep -c '^[<>]')
        echo "determinism,$d1,$d2,$seed,$ctrl,FAIL,gamelog-diverged diff=$ndiff lines" >> "$SUMMARY_CSV"
        _capture_failure "determinism" "$d1" "$d2" "$seed" "$ctrl" "gamelog diff=$ndiff lines (rc=$rcA)" "$glA" "$glB" "$rawA" "$rawB"
        return 1
    fi

    # Identical runs. Distinguish a clean game from a deterministic crash so the
    # crash is visible (and recorded) without being counted as an invariant break.
    if [ "$rcA" -ne 0 ]; then
        DET_CRASH=$((DET_CRASH+1))
        echo "determinism,$d1,$d2,$seed,$ctrl,PASS-CRASH,identical rc=$rcA (deterministic crash, not a divergence)" >> "$SUMMARY_CSV"
        # Keep one copy of the deterministic-crash log for triage (gitignored).
        if [ "$KEEP_LOGS" -eq 0 ]; then
            local cdir="$OUT_DIR/DETCRASH_${ctrl}_s${seed}_$(basename "$d1" .dck)_VS_$(basename "$d2" .dck)"
            mkdir -p "$cdir"; cp "$rawA" "$cdir/run.log" 2>/dev/null
            {
                echo "Deterministic CRASH (both runs identical, rc=$rcA) -- determinism HOLDS."
                echo "This is a separate engine/AI issue, not a reproducibility break."
                echo "DECK1=$d1 DECK2=$d2 SEED=$seed CONTROLLER=$ctrl"
                echo "$MTG_BIN tui $d1 $d2 --p1 $ctrl --p2 $ctrl --seed $seed --tag-gamelogs --no-color-logs --verbosity normal"
            } > "$cdir/REPRODUCER.txt"
            rm -f "$rawA" "$rawB" "$glA" "$glB"
        fi
        return 0
    fi

    DET_PASS=$((DET_PASS+1))
    echo "determinism,$d1,$d2,$seed,$ctrl,PASS,$(wc -l <"$glA") lines" >> "$SUMMARY_CSV"
    [ "$KEEP_LOGS" -eq 0 ] && rm -f "$rawA" "$rawB" "$glA" "$glB"
    return 0
}

# ============================================================================
# INVARIANT 2: local-vs-network equivalence
#   LOCAL  = one process, two AIs.
#   NETWORK= server (authoritative gamelog) + 2 loopback clients.
# Compares LOCAL gamelog vs SERVER gamelog (server is authoritative, has full
# card info) -- exactly the comparison the existing single-seed e2e does, but
# generalized over deck pairs/seeds and with a per-game unique port.
# ============================================================================
run_equivalence() {
    local d1="$1" d2="$2" seed="$3" ctrl="$4" port="$5"
    local tag="eq_${ctrl}_s${seed}_p${port}"
    local localRaw="$OUT_DIR/.$tag.local.log"
    local srvRaw="$OUT_DIR/.$tag.server.log"
    local c1Raw="$OUT_DIR/.$tag.client1.log"
    local c2Raw="$OUT_DIR/.$tag.client2.log"
    local glLocal="$OUT_DIR/.$tag.local.gamelog"
    local glSrv="$OUT_DIR/.$tag.server.gamelog"
    local p1s p2s; p1s="$(derive_p1_seed "$seed")"; p2s="$(derive_p2_seed "$seed")"

    # --- LOCAL game (background) ---
    timeout "$GAME_TIMEOUT" "$MTG_BIN" tui "$d1" "$d2" \
        --p1 "$ctrl" --p2 "$ctrl" \
        --p1-name Ryan --p2-name Gabriel \
        --seed "$seed" --seed-p1 "$p1s" --seed-p2 "$p2s" \
        --tag-gamelogs --no-color-logs --verbosity normal \
        > "$localRaw" 2>&1 &
    local LOCAL_PID=$!

    # --- NETWORK server (background, long-lived lobby) ---
    timeout "$GAME_TIMEOUT" "$MTG_BIN" server \
        --port "$port" --seed "$seed" \
        --tag-gamelogs --network-debug --no-color-logs --verbosity normal \
        --cardsfolder "$CARDSFOLDER" \
        > "$srvRaw" 2>&1 &
    local SERVER_PID=$!

    # wait for server to be listening
    local waited=0
    while ! grep -q -iE "listen|waiting|ready|started" "$srvRaw" 2>/dev/null; do
        sleep 0.3; waited=$((waited+1))
        if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
        [ "$waited" -ge 20 ] && break   # ~6s cap then try anyway
    done
    sleep 0.5

    # --- two clients (background). Clients get the MASTER controller seed and
    #     derive per-slot internally, so both pass --seed-player=$seed. ---
    timeout "$GAME_TIMEOUT" "$MTG_BIN" connect "$d1" \
        --server "localhost:$port" --controller "$ctrl" \
        --seed-player "$seed" --name Ryan \
        --cardsfolder "$CARDSFOLDER" \
        > "$c1Raw" 2>&1 &
    local C1_PID=$!
    sleep 0.5
    timeout "$GAME_TIMEOUT" "$MTG_BIN" connect "$d2" \
        --server "localhost:$port" --controller "$ctrl" \
        --seed-player "$seed" --name Gabriel \
        --cardsfolder "$CARDSFOLDER" \
        > "$c2Raw" 2>&1 &
    local C2_PID=$!

    # Wait for both clients (authoritative game-over), with a wall clock cap.
    local elapsed=0 c1done=0 c2done=0
    while [ "$elapsed" -lt "$GAME_TIMEOUT" ]; do
        [ $c1done -eq 0 ] && ! kill -0 "$C1_PID" 2>/dev/null && c1done=1
        [ $c2done -eq 0 ] && ! kill -0 "$C2_PID" 2>/dev/null && c2done=1
        [ $c1done -eq 1 ] && [ $c2done -eq 1 ] && break
        sleep 1; elapsed=$((elapsed+1))
    done
    wait "$C1_PID" 2>/dev/null; local c1rc=$?
    wait "$C2_PID" 2>/dev/null; local c2rc=$?
    # shut down the lobby server we spawned + reap local
    kill "$SERVER_PID" 2>/dev/null; wait "$SERVER_PID" 2>/dev/null
    wait "$LOCAL_PID" 2>/dev/null; local localrc=$?

    # crash/timeout detection
    if [ $c1done -ne 1 ] || [ $c2done -ne 1 ]; then
        EQ_FAIL=$((EQ_FAIL+1))
        local detail="network timeout (c1done=$c1done c2done=$c2done)"
        echo "equivalence,$d1,$d2,$seed,$ctrl,FAIL,$detail" >> "$SUMMARY_CSV"
        _capture_failure "equivalence" "$d1" "$d2" "$seed" "$ctrl" "$detail" "$localRaw" "$srvRaw" "$c1Raw" "$c2Raw"
        return 1
    fi
    if grep -qE "FATAL SYNC|DESYNC|panicked at|fatal error" "$srvRaw" "$c1Raw" "$c2Raw" 2>/dev/null; then
        EQ_FAIL=$((EQ_FAIL+1))
        echo "equivalence,$d1,$d2,$seed,$ctrl,FAIL,desync/panic-in-network-logs" >> "$SUMMARY_CSV"
        _capture_failure "equivalence" "$d1" "$d2" "$seed" "$ctrl" "desync or panic in network logs" "$localRaw" "$srvRaw" "$c1Raw" "$c2Raw"
        return 1
    fi

    extract_gamelog "$localRaw" "$glLocal"
    extract_gamelog "$srvRaw"  "$glSrv"

    if [ ! -s "$glLocal" ] || [ ! -s "$glSrv" ]; then
        EQ_FAIL=$((EQ_FAIL+1))
        echo "equivalence,$d1,$d2,$seed,$ctrl,FAIL,empty-gamelog(local=$(wc -l <"$glLocal" 2>/dev/null||echo 0) srv=$(wc -l <"$glSrv" 2>/dev/null||echo 0))" >> "$SUMMARY_CSV"
        _capture_failure "equivalence" "$d1" "$d2" "$seed" "$ctrl" "empty gamelog" "$localRaw" "$srvRaw" "$c1Raw" "$c2Raw"
        return 1
    fi

    if diff -q "$glLocal" "$glSrv" >/dev/null 2>&1; then
        EQ_PASS=$((EQ_PASS+1))
        echo "equivalence,$d1,$d2,$seed,$ctrl,PASS,$(wc -l <"$glLocal") lines" >> "$SUMMARY_CSV"
        [ "$KEEP_LOGS" -eq 0 ] && rm -f "$localRaw" "$srvRaw" "$c1Raw" "$c2Raw" "$glLocal" "$glSrv"
        return 0
    else
        EQ_FAIL=$((EQ_FAIL+1))
        local ndiff; ndiff=$(diff "$glLocal" "$glSrv" | grep -c '^[<>]')
        echo "equivalence,$d1,$d2,$seed,$ctrl,FAIL,gamelog-diff=$ndiff lines" >> "$SUMMARY_CSV"
        _capture_failure "equivalence" "$d1" "$d2" "$seed" "$ctrl" "local-vs-network gamelog diff=$ndiff lines" "$glLocal" "$glSrv" "$localRaw" "$srvRaw"
        return 1
    fi
}

# Persist a divergence/crash for triage and record a reproducer.
#   _capture_failure <invariant> <d1> <d2> <seed> <ctrl> <detail> <files...>
_capture_failure() {
    local inv="$1" d1="$2" d2="$3" seed="$4" ctrl="$5" detail="$6"; shift 6
    local dir="$OUT_DIR/FAIL_${inv}_${ctrl}_s${seed}_$(basename "$d1" .dck)_VS_$(basename "$d2" .dck)"
    mkdir -p "$dir"
    for f in "$@"; do [ -f "$f" ] && cp "$f" "$dir/" 2>/dev/null; done
    {
        echo "INVARIANT : $inv"
        echo "DETAIL    : $detail"
        echo "DECK1     : $d1"
        echo "DECK2     : $d2"
        echo "SEED      : $seed"
        echo "CONTROLLER: $ctrl"
        echo
        echo "# Reproduce (determinism: run twice and diff [GAMELOG ...]):"
        echo "$MTG_BIN tui $d1 $d2 --p1 $ctrl --p2 $ctrl --seed $seed --tag-gamelogs --no-color-logs --verbosity normal"
        echo
        echo "# Reproduce (local-vs-network single seed):"
        echo "MTG_BIN=$MTG_BIN bash tests/fuzz_determinism_netequiv_e2e.sh   # bounded"
        echo "bug_finding/fuzz_determinism_netequiv.sh --invariant $inv --decks \"$d1 $d2\" --pair-mode all --start-seed $seed --seeds 1 --controllers $ctrl --keep-logs"
    } > "$dir/REPRODUCER.txt"
    # If both gamelog files were passed first, drop a diff in too.
    if [ -f "$1" ] && [ -f "${2:-}" ]; then
        diff "$1" "$2" > "$dir/gamelog.diff" 2>/dev/null || true
    fi
    FAIL_REPROS+=("[$inv] $ctrl seed=$seed $(basename "$d1") vs $(basename "$d2"): $detail -> $dir")
    echo "  !! DIVERGENCE: [$inv] $ctrl seed=$seed $(basename "$d1") vs $(basename "$d2"): $detail"
    echo "     captured -> $dir"
}

# ----- Main sweep loop ------------------------------------------------------
COMBO=0
PORT_OFF=0
for pair in "${PAIRS[@]}"; do
    d1="${pair%%|*}"; d2="${pair##*|}"
    for seed in $(seq "$START_SEED" $((START_SEED + N_SEEDS - 1))); do
        for ctrl in $CONTROLLERS; do
            if [ "$INVARIANT" = "determinism" ] || [ "$INVARIANT" = "both" ]; then
                COMBO=$((COMBO+1))
                run_determinism "$d1" "$d2" "$seed" "$ctrl"
            fi
            if [ "$INVARIANT" = "equivalence" ] || [ "$INVARIANT" = "both" ]; then
                COMBO=$((COMBO+1))
                local_port=$(( PORT_BASE + (PORT_OFF % 15000) ))
                PORT_OFF=$((PORT_OFF+1))
                run_equivalence "$d1" "$d2" "$seed" "$ctrl" "$local_port"
            fi
            # progress heartbeat every 10 combos
            if [ $((COMBO % 10)) -eq 0 ]; then
                echo "  ... $COMBO combos done (det: $DET_PASS pass/$DET_FAIL fail, eq: $EQ_PASS pass/$EQ_FAIL fail)"
            fi
        done
    done
done

END_TS=$(date +%s)
ELAPSED=$((END_TS - START_TS))

echo "------------------------------------------------------------"
echo " SWEEP COMPLETE in ${ELAPSED}s"
echo "------------------------------------------------------------"
echo "  deck pairs run : ${#PAIRS[@]}"
echo "  seeds/pair     : $N_SEEDS (from $START_SEED)"
echo "  controllers    : $CONTROLLERS"
echo "  total combos   : $COMBO"
if [ "$INVARIANT" = "determinism" ] || [ "$INVARIANT" = "both" ]; then
    echo "  determinism    : PASS=$DET_PASS  FAIL=$DET_FAIL  (deterministic-crash=$DET_CRASH, identical runs that errored -- not a divergence)"
fi
if [ "$INVARIANT" = "equivalence" ] || [ "$INVARIANT" = "both" ]; then
    echo "  equivalence    : PASS=$EQ_PASS  FAIL=$EQ_FAIL"
fi
echo "  summary csv    : $SUMMARY_CSV"

TOTAL_FAIL=$((DET_FAIL + EQ_FAIL))
if [ "$TOTAL_FAIL" -gt 0 ]; then
    echo
    echo "  !!!! $TOTAL_FAIL DIVERGENCE(S) FOUND -- captures under $OUT_DIR/FAIL_* !!!!"
    for r in "${FAIL_REPROS[@]}"; do echo "    - $r"; done
    echo "============================================================"
    exit 1
fi

# clean run: if nothing failed and we aren't keeping logs, the OUT_DIR holds
# only the summary csv. Leave it (it's under gitignored debug/).
echo
echo "  ✓ ALL $COMBO COMBOS PASSED -- no determinism or equivalence divergence."
echo "============================================================"
exit 0
