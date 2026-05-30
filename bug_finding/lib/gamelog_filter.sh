#!/usr/bin/env bash
# gamelog_filter.sh — THE single bash implementation of [GAMELOG ...]
# extraction + comparison-noise filtering for local-vs-network /
# determinism gamelog diffing. Source this; do not reinline the sed/grep
# pipeline.
#
# Before consolidation this exact pipeline was copy-pasted into
#   - bug_finding/fuzz_determinism_netequiv.sh  (extract_gamelog())
#   - tests/network_vs_local_equivalence_e2e.sh (two inline copies)
# Behaviour MUST match the Python counterpart
# bug_finding/network_test_lib.py::extract_gamelog (same ANSI-strip +
# [GAMELOG ...] keep + same known-noise filter).
#
# What it does, on stdin -> stdout:
#   1. strip ANSI colour escapes (local logs may be colourised)
#   2. keep only [GAMELOG ...] lines
#   3. drop known-noise lines that legitimately differ in timing between
#      server/client damage accounting:
#        - "Tap ... for {...}"   (mana tap formatting)
#        - bare "... resolves"   (resolution echo)
#        - "... takes ... damage ... life:"  (per-event life delta)
#        - "... deals  ... damage ... life:" (per-event life delta)
#
# Usage:
#   source "<repo>/bug_finding/lib/gamelog_filter.sh"
#   some_command 2>&1 | gamelog_filter > out.gamelog
#   # or, file-in/file-out:
#   gamelog_filter_file "$raw_log" "$out_gamelog"

# Read raw log on stdin; emit filtered, comparable [GAMELOG ...] lines on stdout.
gamelog_filter() {
    sed -E 's/\x1b\[[0-9;]*m//g' \
        | grep '\[GAMELOG' \
        | grep -v 'Tap.*for {' \
        | grep -v 'resolves$' \
        | grep -v 'takes.*damage.*life:' \
        | grep -v 'deals.*damage.*life:'
}

# Convenience: filter $1 (raw log file) into $2 (output file). Never fails the
# caller on an empty match (the comparison logic handles empty files itself).
gamelog_filter_file() {
    local src="$1" dst="$2"
    gamelog_filter < "$src" > "$dst" 2>/dev/null || true
}
