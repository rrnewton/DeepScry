---
title: 'Web/WASM: demote per-action log spam below non-DEBUG (Priority check / apply_state_sync / WASM Draw-Played); keep non-debug clean'
status: open
priority: 3
issue_type: task
created_at: 2026-06-05T13:52:32.720913006+00:00
updated_at: 2026-06-05T13:52:32.720913006+00:00
---

# Description

Demote per-action web/WASM log spam below non-DEBUG; non-debug games should be CLEAN.

REPORTED (user playtest 2026-06-05): even WITHOUT 'Debug logging (TRACE)' checked, full-speed random/random games spew per-action console output. Examples:
- 'Priority check: player N has 0 available abilities, action_count=..'
- 'WasmRemoteController: Opponent chose indices [..] (seq=.., P2 choice #..)'
- 'WasmNetworkLocalController: ChoiceRequest seq=.. ready (last_submitted=..)'
- 'WasmNetworkLocalController: Auto-pass with 0 abilities (..submitting immediately)'
- 'WasmNetworkClient: Submitted choice seq=.., waiting for ack...'
- 'apply_state_sync: reveal ac=.. owner=.. card=..'
- 'WASM Draw/Played: <card> (id=..) card_already_known=..'

ASK: audit ALL of this and move it to debug/TRACE level (or delete where it has no debugging value). Goal: a non-DEBUG game shows ONLY essential info + errors. Sources: WASM client controllers + apply_state_sync (mtg-engine/src/wasm/network/).

SEPARATE perf note: random/random at 'full speed' is still pretty slow despite the near-instant Random controller - needs a perf look. Related: mtg-188 (reduce idle WASM render freq), mtg-726 (network-test perf deep-dive).
