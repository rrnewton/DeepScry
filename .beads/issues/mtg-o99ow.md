---
title: 'NETARCH: reveal-as-choice unification — key reveals by game action_count, revert the action_count exclusion'
status: open
priority: 2
issue_type: task
created_at: 2026-06-04T03:13:00.957496754+00:00
updated_at: 2026-06-04T03:22:07.770091804+00:00
---

# Description

NETARCH rearchitecture (branch `netarch-reveal-actionlog-unify`, slot01 + handoff chain). USER-DECIDED direction (2026-06-03, AFK autonomous). Principled successor to the action_count exclusion (state_hash.rs) — this issue's CLOSING commit REVERTS that exclusion and restores action_count as a cross-replica invariant.

Full design basis: ai_docs/REVEAL_ACTIONLOG_UNIFICATION_DESIGN_20260603.md. Under tracker mtg-677 (finish netarch rewind/replay). Cross-links: mtg-610 (effective-ac arch this deletes), mtg-589 (reorder-before-reveal two-pass this deletes), mtg-559 / mtg-mb668 / mtg-zfq7x (rewind / in-stack / hidden-zone prereqs), mtg-254 (WASM client arch), migration step 5 (docs/NETWORK_ACTION_LOG_MIGRATION.md §5).

## GOAL (user's framing)
A card REVEAL is structurally a remote CHOICE: a monotone, information-increasing entry in the ONE replica-computed action log; the server fills the missing bit keyed by the GAME action_count ("at K, revealed card = X"); consumed via the SAME block/NeedsInput/rewind gate as choices. Sync stays TARGETED (reveals only to who should see). DONE = action logs ALWAYS ALIGNED, identical modulo reveal-name visibility. Monotone info applied ASAP (never hurts). The pre-draw proactive send becomes pure EFFICIENCY (block-on-missing handles correctness). Eliminate the SYNTHETIC-keyed side channel; RESTORE action_count as a cross-replica invariant.

## PREREQUISITE STATUS (reassessed 2026-06-03 — draw step LARGELY SATISFIED)
Keying reveals at the DRAW action_count requires the GameLoop to reliably rewind/block at the draw step. ORIGINAL premise (mtg-677 N4): draw-step rewind NOT done. REASSESSED: commit 26c5a460 (mtg-610 WIP, ancestor of integration) already DELETED the 9-guard TurnStructure re-entry family (incl. draw_step_executed_turn) and both net paths resume via undo-log rewind+replay (steps.rs:411, phase.rs:167). So draw-ac keying is UNBLOCKED for the draw step. The N4 text in mtg-677 is STALE (dated update appended there). The ONLY remaining gap is the in-stack-resolution class — see SUBSUMPTION.

## SEQUENCE (overnight chain)
1. [DONE] File this issue (durable resume contract).
2. Assess the mtg-677 native/WASM draw-step rewind/block gap precisely.
3. CHEAP, prereq-INDEPENDENT win: server per-reveal action_count stamping — emit each draw-reveal stamped with its OWN forward_idx (controller.rs:515) instead of bundling all into the next choice's ac. Data already computed; bankable alone.
4. [UNBLOCKED for draw step] Delete effective-ac map (wasm/network/client.rs:194 + family); key apply on target_action (not greedy up_to_frontier); wire native wait_for_state_sync_frontier (client.rs:643-667, currently no non-test caller) into the draw path (steps.rs:415) + priority path (priority.rs:575). Stage so in-stack-resolution cases align as you go.
5. [GATED] Collapse late-binding: draw_card instantiates inline; delete undo.rs:1305-1311 is_late_binding + async cards.insert for Draw/OpeningHand; KEEP dummy/masked path (is_dummy_reveal).
6. [GATED] Fold reorder emission into shuffle_library (state.rs:745) at the ShuffleLibrary action's own ac (residual #1: shuffle_library emits no LibraryReordered); ADD action_count:u64 to LibraryReordered (protocol.rs:658); delete reorder-before-reveal two-pass (mtg-589).
7. [GATED, CLOSING commit] Revert the action_count exclusion (state_hash.rs); flip its RED test to assert alignment.

## PROOF GATE / canaries (every step)
Full `make validate` → cite validate_logs/validate_<sha>.log. Sharpest desync canaries: web/test_network_multideck.js All Hallow's Eve seed=3; native-vs-WASM DIVERGED:0 mirror; robots42 family.

## DISCIPLINE
desync is ALWAYS fatal: END STATE is genuine alignment (action_count back in hash, exclusion reverted), NEVER suppression. Incremental commits at every green sub-step. Keep this issue's NEXT-STEP current — the handoff chain reads it.

## SUBSUMPTION (team-lead 2026-06-03) — ONE effort, not gated on an external blocker
The ONLY remaining rewind-completeness gap is the IN-STACK-RESOLUTION class: robots42 deep-ac desync (Copy Artifact Clone / Balance in-stack at depth ~1616), still EXCLUDED from the gate. That in-stack class IS the mtg-mb668 class-A residual (seed-2 turn-16 post-shuffle, seed-5, deep-ac) and is SUBSUMED by THIS unification — those desyncs are exactly "reveal/reorder info not aligned at the right action_count." Do NOT treat robots42 as external to wait on; un-excluded-green robots42 is the ACCEPTANCE PRIZE, made green by the aligned-log model. mtg-yexvc residual findings are direct input. The action_count drift the exclusion masks (seed-2 Timetwister: client 947 vs server 950 actions, identical observable state, state_hash.rs:415-427) is the same root: the client doesn't log every server action — the unification makes every reveal/reorder a logged action at the SAME ac on both replicas, so counts realign and action_count returns to the hash.

## NEXT STEP (resume here)
Step 3 DONE + committed @72b8607e (CardRevealInfo.action_count = forward_idx; server.rs choice_request.reveals loop stamps reveal_info.action_count; OpponentChoice/ChoiceAccepted/library-search reveals left at choice ac — verified isolated from mtg-mb668 searched_card_for, which only matches empty-name Searched reveals). Full `make validate` running (bg) for the validate_<sha>.log proof + canaries. IF GREEN: push branch (orchestrator diff-gates + ff-merges), then proceed to step 4 — draw-ac keying is UNBLOCKED for the draw step: delete effective-ac map (wasm/network/client.rs:194 + push_state_sync_stamped / stamp_pending_state_sync / effective_ac_of family), key apply on target_action instead of greedy up_to_frontier (apply_state_sync_up_to_frontier ~1286), wire native wait_for_state_sync_frontier (client.rs:656) into the draw path (steps.rs:415) + priority path. Stage so in-stack-resolution cases align as you go; robots42 is the canary. IF VALIDATE RED: stash the 2-file change to attribute (integration is orchestrator-maintained green → a regression is mine), fix forward, do NOT push half-done.
