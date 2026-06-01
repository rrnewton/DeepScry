---
title: 'TRACK: finish netarch rewind/replay (PRIMARY — land first) — action-log source of truth + delete TurnStructure guards'
status: open
priority: 1
issue_type: task
created_at: 2026-06-01T00:58:29.466320424+00:00
updated_at: 2026-06-01T00:58:29.466320424+00:00
---

# Description

NETARCH PRIMARY GOAL (user 2026-05-31, AFK autonomous): finish the network rearchitecture so web-based games use PROPER intra-turn rewind/replay for "blocking" instead of re-run-with-guards. This is the #1 priority to LAND FIRST (ahead of lobby/launcher). Lobby+server-protocol+launcher+deck-editor work proceeds in PARALLEL where it is SEPARABLE from netarch, but netarch agents get merge priority.

DECISION: do NOT rebase PR#11 (wasm-rewind-replay, 3 ahead / 153 behind, touches the most-churned engine files combat/phase/steps/mod/state/undo/ai_harness). Reimplement FRESH on current integration, guided by the mtg-610 design + PR#11's documented learnings. PR#12 already closed (its ActionLog<T> code + docs already merged).

THE GOAL STATE (from mtg-610 + action_log.rs invariants + NETWORK_ARCHITECTURE.md):
- The monotonic, append-only, NON-DESTRUCTIVE action_log (ActionLog<T>, already on integration) is the SINGLE source of truth for all network-fed info to controllers AND to shadow-state updates.
- WASM harness BLOCKS by rewind-to-checkpoint + replay-forward (re-evolving internal state, suppressing ONLY external effects: dup logging / network sends) — the SAME mechanism snapshot/resume already uses and passes. NOT re-run-from-top-of-step.
- Delete the ~13 TurnStructure *_turn guard fields (draw_step_executed_turn, turn_state_reset_turn, attackers_declared_turn, blockers_declared_turn, combat_first_strike_damage_dealt_turn, combat_first_strike_priority_done_turn, combat_damage_dealt_turn, upkeep_triggers_checked_turn, end_step_triggers_checked_turn, draw_triggers_checked_turn, main1_delayed_fired_turn, main2_delayed_fired_turn) — they only exist to paper over re-run-without-rewind.
- Same fix resolves mtg-559 (robots42 Copy Artifact in-stack-resolution re-entry) → robots42 re-joins the network gate.

THE BLOCKER PR#11 hit (must solve): destructively-consumed network inputs are not replayable on turn>=2 — controller.rs:309 view.take_pending_library_reorders() DRAINS; opponent side-channels. FIX: route these through the non-destructive action_log so replay reconstructs them (the action_log already declares CardRevealed/LibraryReordered as its StateSyncEntry payloads — wire the consumption to read-by-action_count, not drain).

SEQUENCING (netarch decomposed into landable steps; each validate-gated + skeptic-gated, native byte-identical, STRICT native-vs-WASM DIVERGED:0):
- N1: make library-reorders + opponent side-channel inputs flow through the non-destructive action_log (read-by-action_count, stop draining). Prove replay reconstructs them. (Unblocks the turn>=2 gap.)
- N2: verify/extend undo.rs rewind for arbitrary MID-RESOLUTION / in-stack state (mtg-559 linchpin); add debug-assert hash round-trip invariant (rewind+replay returns to exact start hash).
- N3: switch ai_harness step_harness to rewind+replay blocking (resume not re-run); add the debug hash-check.
- N4: delete the TurnStructure guard family; re-enable robots42 in the network gate; confirm net SIMPLER (line count down).
Each step can be its own commit/merge if it stands alone + validates; otherwise one branch netarch-rewind-replay.

PARALLEL (separable, non-netarch): lobby-server-protocol agent (RUNNING: Register/unique-name, waiting-game heartbeat/eviction mtg-dw9j3, deck+ready, reconnect tokens, bug-report infallibility, --help rewrite). Launcher/3-page web (mtg-khy7x) + deck editor — mostly JS/HTML, separable EXCEPT net_game_driver.js (Phase 3 native renderer) which DEPENDS on netarch landing first. Interleave merges; netarch first when conflicting.

Refs: mtg-610 (arch, in_progress), mtg-559 (robots42), mtg-589 (desync family), mtg-614 (closed: old PR#11 attempt), action_log.rs, undo.rs, ai_harness.rs, snapshot_architecture.md, NETWORK_ARCHITECTURE.md, NETWORK_ACTION_LOG.md.
