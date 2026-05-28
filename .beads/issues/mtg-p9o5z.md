---
title: 'Bug: validate-network-e2e-step flaky/red on integration (rogerbrand+monored random games desync)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T14:00:38.951917168+00:00
updated_at: 2026-05-28T14:00:38.951917168+00:00
---

# Description

make validate's `validate-network-e2e-step` (web/test_network_gui_e2e.js, run in CI per .github/workflows/ci.yml:225) is currently RED on the `integration` base commit, independent of any feature work.

EVIDENCE (2026-05-28): Reproduced on clean integration commit 897881c9 (no local changes) in worktree fix-allhallows-eve after `make build-network wasm-network`:

  make validate-network-e2e-step
  --- Scenario 1/2: monored.dck seed=13 ---   PASS (24.2s)
  --- Scenario 2/2: old_school/01_rogue_rogerbrand.dck seed=3 ---  FAIL
    FATAL: P2 state hash mismatch! server=7bc17eb6... client=dc9e5bc4... at choice_seq=250 action_count=1352
  === MULTI-DECK TEST FAILED ===

The mismatch choice_seq/action_count VARIES run-to-run (observed 41, 149, 250 across runs) and which scenario fails varies (monored passed on one run, failed on others) — i.e. a NONDETERMINISTIC / timing-sensitive desync, not a fixed-seed deterministic one. This is the same bug class as mtg-1jtoy (reveal-ordering desync in random network games, mid-game P2 state hash mismatch) and mtg-b2tqp (shadow-state divergence). A concurrent agent's validate run happened to PASS both scenarios (got lucky on timing), confirming flakiness.

IMPACT: `make validate` cannot complete green on integration until this is fixed, because the network-e2e step is mandatory. This blocks every agent from producing a fully-green `validate_<sha>.log` via the normal `make validate` path. Until fixed, validation proof for unrelated feature branches must rely on (a) the full nextest suite passing (1232/1232) plus (b) the specific feature's e2e/unit tests, with the network-e2e flake explicitly called out.

NEXT STEPS: investigate the reveal-ordering / shadow-state divergence per mtg-1jtoy's root-cause analysis; add a deterministic seed-pinned regression once the race is understood. Consider whether the WASM browser timing under load contributes (the failures cluster when CPU-contended).

Reproducer (needs a graphical/headless-chromium env that make validate sets up):
  make build-network wasm-network
  make validate-network-e2e-step
