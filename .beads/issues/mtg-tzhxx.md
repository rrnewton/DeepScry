---
title: Add browser-e2e gate for WASM AI harness turn-1 EndCombat re-entry (mtg-gfr2a follow-up)
status: open
priority: 3
issue_type: task
created_at: 2026-06-09T21:17:48.204102972+00:00
updated_at: 2026-06-09T21:18:05.635717821+00:00
---

# Description

## Summary

Follow-up from the mtg-gfr2a fix review (WASM AI harness migrated to rewind+replay resume, landed on integration @ 5b3e5aa1). The mtg-gfr2a-fixed code path currently has NO direct browser-e2e gate, so a regression in the turn-1 EndCombat re-entry behavior would not be caught by `make validate` at the browser level.

## What to add

1. **Automated browser-e2e for the WASM AI harness path.** Drive `web/wasm_ai_harness.html` via its `run_network_ai_step` entrypoint to play a WASM-vs-WASM random game that exercises a **turn-1 EndCombat re-entry** (the exact scenario mtg-gfr2a regressed on). Assert no state-hash desync. Wire this new test into `make validate` (the validate step matrix in `scripts/validate.py`) so the harness path is gated on every run.

2. **Symmetric early-desync detection in step_replay.** Consider adding `fancy_tui`'s post-replay `run_replay_verification()` to the `ai_harness` `step_replay` path. fancy_tui runs a verification pass after replay that catches early divergence; the AI harness replay path lacks the equivalent symmetric check, so an early (turn-1) desync introduced during replay-resume could go undetected in the harness even though fancy_tui would flag it.

## Why

The mtg-gfr2a fix (rewind+replay resume migration in `mtg-engine/src/wasm/network/ai_harness.rs`) is currently protected only by the Rust-level oracle e2e test (`mtg-engine/tests/rewind_replay_oracle_e2e.rs`) and the native validate sweep — not by a real browser-driven run of the harness HTML page. A direct browser gate plus symmetric replay verification close the coverage gap for this desync class.

## References

- mtg-gfr2a (closed) — the desync this follows up on.
- Landed fix: integration @ 5b3e5aa1 ("fix(netarch): migrate WASM AI harness to rewind+replay resume").
- Touch points: `web/wasm_ai_harness.html` (`run_network_ai_step`), `mtg-engine/src/wasm/network/ai_harness.rs` (`step_replay`), fancy_tui `run_replay_verification()`, `scripts/validate.py` (validate step matrix).
