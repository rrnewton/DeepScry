#!/usr/bin/env bash
#
# validate_step.sh — terse, tagged, self-contained step runner for `make validate`.
#
# This is the log-hygiene + observability foundation for the validate overhaul
# (mtg-717). It wraps ONE validation step so that:
#
#   * the validate log stays TERSE: only a tagged START line and a tagged
#     PASS/FAIL line (with wall-clock duration) reach stdout by default;
#   * the step's DETAILED output (compiler spew, per-game logs, per-request
#     server logs, browser console) is streamed to a per-step file instead of
#     the shared validate log;
#   * on FAILURE the captured detail is dumped INTO the log (each line tagged)
#     so a failing run is self-contained — you never have to re-run to see why;
#   * every emitted line is prefixed `[jobGroup.jobId]` so that, even when
#     `make -j` interleaves many steps onto one stdout, you can grep one part
#     of the work-tree and ignore the rest while still seeing real completion
#     order.
#
# 3-LEVEL naming (jobGroup, jobId, testName): this wrapper owns the first two
# levels (group.job). The wrapped command owns testName-level detail in its own
# output (e.g. a nextest "mtg-engine game::foo::bar" line), which lands in the
# per-step detail file.
#
# Usage:
#   scripts/validate_step.sh <jobGroup> <jobId> "<description>" -- <command> [args...]
#
# Environment:
#   VALIDATE_VERBOSE=1        also stream the tagged detail live to stdout
#                             (default: detail only goes to the per-step file,
#                             and to stdout only on failure)
#   VALIDATE_VERBOSE_DIR=DIR  directory to persist every step's detail log,
#                             named "<group>.<job>.log". When unset, the detail
#                             goes to a temp file that is deleted on success.
#
# Exit status: the wrapped command's exit status (so `make` still fails the
# build on a failing step).

set -u

if [ "$#" -lt 4 ]; then
    echo "usage: $0 <jobGroup> <jobId> <description> -- <command...>" >&2
    exit 2
fi

GROUP="$1"; shift
JOB="$1"; shift
DESC="$1"; shift
if [ "$1" != "--" ]; then
    echo "$0: expected '--' separator before the command, got '$1'" >&2
    exit 2
fi
shift

TAG="${GROUP}.${JOB}"

# Resolve where the detail log lives.
DETAIL_KEEP=false
if [ -n "${VALIDATE_VERBOSE_DIR:-}" ]; then
    mkdir -p "$VALIDATE_VERBOSE_DIR"
    DETAIL_FILE="$VALIDATE_VERBOSE_DIR/${TAG}.log"
    DETAIL_KEEP=true
else
    DETAIL_FILE="$(mktemp "/tmp/validate_${TAG}.XXXXXX.log")"
fi

# Terse, tagged START line.
printf '[%s] ▶ START  %s\n' "$TAG" "$DESC"

START_EPOCH=$(date +%s)

# Run the command, capturing combined stdout+stderr to the detail file.
# In VERBOSE mode, ALSO stream the detail live to stdout with each line tagged.
if [ "${VALIDATE_VERBOSE:-0}" = "1" ]; then
    # tee to file, and pipe the live copy through a tagging sed to stdout.
    # pipefail so the wrapped command's status (not sed/tee) is what we read.
    set -o pipefail
    "$@" 2>&1 | tee "$DETAIL_FILE" | sed -u "s/^/[$TAG] /"
    STATUS=${PIPESTATUS[0]}
else
    "$@" >"$DETAIL_FILE" 2>&1
    STATUS=$?
fi

END_EPOCH=$(date +%s)
DURATION=$((END_EPOCH - START_EPOCH))

if [ "$STATUS" -eq 0 ]; then
    printf '[%s] ✓ PASS   %s (%ds)\n' "$TAG" "$DESC" "$DURATION"
    if [ "$DETAIL_KEEP" = false ]; then
        rm -f "$DETAIL_FILE"
    fi
else
    printf '[%s] ✗ FAIL   %s (%ds, exit %d)\n' "$TAG" "$DESC" "$DURATION" "$STATUS"
    # Self-contained failure: dump the captured detail INTO the log, tagged.
    # (In VERBOSE mode the live stream already showed it; dump anyway so the
    # failure block is contiguous and greppable by tag.)
    printf '[%s] ----- detail (%s) -----\n' "$TAG" "$DETAIL_FILE"
    sed "s/^/[$TAG] /" "$DETAIL_FILE"
    printf '[%s] ----- end detail -----\n' "$TAG"
    if [ "$DETAIL_KEEP" = false ]; then
        rm -f "$DETAIL_FILE"
    fi
fi

exit "$STATUS"
