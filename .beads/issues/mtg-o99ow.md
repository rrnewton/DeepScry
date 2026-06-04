---
title: 'NETARCH: reveal-as-choice unification — key reveals by game action_count, revert the action_count exclusion'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T03:13:00.957496754+00:00
updated_at: 2026-06-04T03:13:26.765803492+00:00
---

# Description

NETARCH rearchitecture (branch `netarch-reveal-actionlog-unify`, slot01 + handoff chain). USER-DECIDED direction (2026-06-03, AFK autonomous). Principled successor to the action_count exclusion (state_hash.rs) — this issue's CLOSING commit REVERTS that exclusion and restores action_count as a cross-replica invariant.

Full design basis: ai_docs/REVEAL_ACTIONLOG_UNIFICATION_DESIGN_20260603.md. Under tracker mtg-677 (finish netarch rewind/replay). Cross-links: mtg-610 (effective-ac arch this deletes), mtg-589 (reorder-before-reveal two-pass this deletes), mtg-559 / mtg-mb668 / mtg-zfq7x (rewind / in-stack / hidden-zone prereqs), mtg-254 (WASM client arch), migration step 5 (docs/NETWORK_ACTION_LOG_MIGRATION.md §5).

## GOAL (user's framing)
A card REVEAL is structurally a remote CHOICE: a monotone, information-increasing entry in the ONE replica-computed action log; the server fills the missing bit keyed by the GAME action_count ("at K, revealed card = X"); consumed via the SAME block/NeedsInput/rewind gate as choices. Sync stays TARGETED (reveals only to who should see). DONE = action logs ALWAYS ALIGNED, identical modulo reveal-name visibility. Monotone info applied ASAP (never hurts). The pre-draw proactive send becomes pure EFFICIENCY (block-on-missing handles correctness). Eliminate the SYNTHETIC-keyed side channel; RESTORE action_count as a cross-replica invariant.

## HARD PREREQUISITE (do NOT skip sequencing)
Keying reveals at the DRAW action_count requires the GameLoop to reliably rewind/block at the draw step. mtg-677 N4 shows native+live-WASM draw-step rewind/block completeness is NOT done (draw_step_executed_turn guard removal → P2 hash mismatch @Turn2 Draw ac=86). DO NOT attempt draw-ac keying (analysis steps 4-7) before that prerequisite lands.

## SEQUENCE (overnight chain)
1. [DONE] File this issue (durable resume contract).
2. Assess the mtg-677 native/WASM draw-step rewind/block gap precisely.
3. CHEAP, prereq-INDEPENDENT win: server per-reveal action_count stamping — emit each draw-reveal stamped with its OWN forward_idx (controller.rs:515) instead of bundling all into the next choice's ac. Data already computed; bankable alone.
4. [GATED on prereq] Delete effective-ac map (wasm/network/client.rs:194 + family); key apply on target_action (not greedy up_to_frontier); wire native wait_for_state_sync_frontier (client.rs:643-667, currently no non-test caller) into the draw path (steps.rs:415) + priority path (priority.rs:575).
5. [GATED] Collapse late-binding: draw_card instantiates inline; delete undo.rs:1305-1311 is_late_binding + async cards.insert for Draw/OpeningHand; KEEP dummy/masked path (is_dummy_reveal).
6. [GATED] Fold reorder emission into shuffle_library (state.rs:745) at the ShuffleLibrary action's own ac (residual #1: shuffle_library emits no LibraryReordered); ADD action_count:u64 to LibraryReordered (protocol.rs:658); delete reorder-before-reveal two-pass (mtg-589).
7. [GATED, CLOSING commit] Revert the action_count exclusion (state_hash.rs); flip its RED test to assert alignment.

## PROOF GATE / canaries (every step)
Full `make validate` → cite validate_logs/validate_<sha>.log. Sharpest desync canaries: web/test_network_multideck.js All Hallow's Eve seed=3; native-vs-WASM DIVERGED:0 mirror; robots42 family.

## DISCIPLINE
desync is ALWAYS fatal: END STATE is genuine alignment (action_count back in hash, exclusion reverted), NEVER suppression. Incremental commits at every green sub-step. Keep this issue's NEXT-STEP current — the handoff chain reads it.

## NEXT STEP (resume here)
Step 3 in progress: add `action_count: u64` to CardRevealInfo (controller.rs:28), populate from forward_idx in collect_reveals_since_last_choice (controller.rs:556), use reveal_info.action_count at the bundled-reveal emission site server.rs:2870 (the choice_request.reveals loop ONLY — leave OpponentChoice/ChoiceAccepted/library-search reveal stamps at the choice ac; those reveals ARE the choice's effect). Then full validate + canaries. This changes draw-reveal apply timing EARLIER (draw-ac vs choice-ac) but is hash-neutral while exclusion stands (step 7 restores it). If validate red, do NOT commit — surface.
