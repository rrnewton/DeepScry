---
title: WASM SMART damage assignment can return NeedInput from synchronous combat code
status: open
priority: 3
issue_type: bug
created_at: 2026-05-15T14:24:37.832025245+00:00
updated_at: 2026-05-15T14:24:37.832025245+00:00
---

# Description

Discovered while fixing mtg-e05f9c. After implementing
choose_blocker_for_lethal_damage / choose_blocker_for_remaining_damage in
WasmNetworkLocalController, those overrides correctly return NeedInput when
the matching ChoiceRequest from the server hasn't yet arrived in the WASM
client's queue.

But the call site (smart_damage_assignment in mtg-engine/src/game/actions/combat.rs
lines 343/386) treats NeedInput as fatal — it does not have re-entry support
because the surrounding apply_combat_damage / smart_damage_assignment call
chain is fully synchronous.

When this race triggers, the ai_harness reports:
  Game loop error: InvalidAction("NeedInput returned in synchronous game loop")

Observed FLAKY (~1/10 in --quick mixed fuzz) — game eventually completes
because subsequent step_harness invocations skip already-executed combat
damage via the combat_damage_dealt_turn guard, so this is non-fatal in
practice. But it's still a violation of the deterministic-sequential-
simulation principle and should be fixed.

Possible fix paths:
1. Make smart_damage_assignment / apply_combat_damage re-entrant.
2. Pre-drain server messages in the harness before running combat damage.
3. Block in the WASM harness by polling for the ChoiceRequest before
   running combat damage.

Reproducer: bug_finding/network_fuzz_test.py --quick --client mixed --parallel 2
