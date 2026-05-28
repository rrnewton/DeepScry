---
title: 'Network determinism: server-authoritative winner + mana-cache consistency on mass tap/untap (fix-network-desync)'
status: open
priority: 2
issue_type: bug
created_at: 2026-05-28T16:47:30.979583956+00:00
updated_at: 2026-05-28T16:47:30.979583956+00:00
---

# Description

## Summary

Two independent network-determinism fixes from the fix-network-desync branch (replaces the now-gone hash issue mtg-p9o5z referenced at dispatch).

### 1. Flaky 'Clients disagree on winner' (in make validate)

The in-validate websocket test `network_e2e::websocket_integration::test_run_game_with_random_controllers` intermittently failed: 'Clients disagree on winner: Some(1) vs None'. With `with_deferred_game_end`, BOTH clients learn the winner only from the server's `GameEnded` message, but `run_game` aborted the reader task and read `server_winner` immediately after the GameLoop returned. The client whose loop returned first could read `server_winner == None` (reader had not yet processed GameEnded) and report a different winner than its peer.

Fix (network/client.rs): make the winner server-authoritative AND await the GameEnded notification before aborting the reader. Added `SharedNetworkState::server_winner`, `set_server_winner` (notifies waiters via a tokio Notify), and `wait_for_server_winner(timeout)`. `run_game` now awaits the server verdict (10s safety timeout) BEFORE `reader_handle.abort()`, then prefers it over any locally-derived winner. Both ExitGame and natural-completion paths return the same value.

Stability: 30/30 websocket runs, 0 fails.

### 2. Mana-cache staleness on mass tap/untap (latent network desync)

`Effect::TapAll` / `Effect::UntapAll` (actions/mod.rs), the Waterbend cost taps, and `GameState::untap_all` set `card.tapped` DIRECTLY, bypassing `tap_permanent`/`untap_permanent`. That left the `ManaSourceCache` precomputed `untapped_*` totals (and `mana_state_version`) stale, so `ManaEngine` over-reported available mana and could offer an UNAFFORDABLE spell as a legal play -> server full-state vs client shadow-state divergence (fatal network desync). It also skipped undo logging for those tap changes.

Fix: route all of these through `tap_permanent`/`untap_permanent` (canonical path: logs TapCard undo, updates the cache via on_tap/on_untap, bumps mana_state_version). Added a `#[cfg(debug_assertions)]` invariant in `ManaEngine::read_from_cache` asserting cached untapped totals == live single-mana source tap state, to catch this regression class immediately.

Regression test: `mana_engine::tests::test_mass_tap_untap_keeps_mana_cache_consistent` — populates the EAGER cache, taps via TapAll, asserts ManaEngine reports 0 green; confirmed it FAILS with the old direct-mutation code and PASSES with the fix.

NOTE: the avatar decks used by the equivalence e2e do not contain TapAll/UntapAll/Waterbend cards, so this fix is verified by the deterministic unit test, not the equivalence sweep.

### Also: wired equivalence test into make validate

Added `tests/network_vs_local_equivalence_e2e.sh 3 random` + `3 zero` to `validate-network-e2e-step` (Makefile) to guard this class going forward (mtg-380). Sweep: 30/30 (10 seeds x random/heuristic/zero) deterministic-identical.

### Out of scope (separate pre-existing bug)

rogerbrand seed=3 P2 state-hash mismatch (Demonic Tutor / library-search shadow sync) filed as mtg-vk4b7; confirmed pre-existing on baseline, NOT in make validate.

Related: mtg-380, mtg-273, mtg-vk4b7.
