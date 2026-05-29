---
title: lightning_bolt e2e leaves orphaned spinning mtg tui (timeout doesn't kill process group; tui spins on stdin EOF)
status: open
priority: 2
issue_type: task
created_at: 2026-05-29T02:13:32.482230523+00:00
updated_at: 2026-05-29T02:13:32.482230523+00:00
---

# Description

BUG (flaky validate + CPU waste): tests/lightning_bolt_targets_opponent_player_e2e.sh leaves ORPHANED `mtg tui` processes spinning at ~100% CPU.

Line ~99: `printf '1\n1\n0\n' | run_mtg_with_timeout 30 tui --start-state $PUZZLE --p1 tui --p2 zero --seed 42 ... || true`.

Two coupled defects:
1. run_mtg_with_timeout (tests/lib/test_helpers.sh) does not kill the PROCESS GROUP — when the 30s timeout fires it kills the immediate child/wrapper, but the actual `mtg` grandchild is orphaned (reparented to init, ppid=1) and keeps running.
2. `mtg tui` with --p1 tui BUSY-LOOPS at ~100% CPU on stdin EOF: after consuming the piped `1\n1\n0\n` it hits EOF and (apparently) loops re-reading EOF instead of exiting/erroring. So the orphan spins a full core indefinitely.

Observed TWICE: a leftover `bolt_only_player_target.pzl --p1 tui --p2 zero` orphan spinning ~12 min at 94.9% CPU (killed by PID by the coordinator) after both the step-2 (mtg-609) validate and the crate-rename (mtg-601) validate. Wastes a core, inflates load (→ false timeout-flakes in concurrent/subsequent tests), and pollutes the box.

Fixes:
- run_mtg_with_timeout: use `timeout --kill-after` + run the child in its own process group and kill the whole group (setsid + kill -- -PGID), so no orphan survives.
- mtg tui controller: on stdin EOF, exit cleanly (or return an error) instead of busy-looping — a TUI/human controller reading EOF should terminate, not spin.
- Consider: this test drives the interactive --p1 tui controller with piped fixed input just to capture the target menu; a fixed-input/script controller (--p1 fixed:...) would be more robust than --p1 tui for an automated test.

Relates to validate stability (the green+stable priority) + the flakiness system (mtg-593): a spinning orphan manufactures timeout-under-load false flakes.
