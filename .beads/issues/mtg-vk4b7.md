---
title: 'Network desync: rogerbrand seed=3 P2 state-hash mismatch at choice_seq=216 (Demonic Tutor / library-search shadow sync)'
status: in_progress
priority: 2
issue_type: bug
created_at: 2026-05-28T16:39:35.293229311+00:00
updated_at: 2026-05-28T20:12:31.537667530+00:00
---

# Description

## Summary

validate-network-e2e-step (web/test_network_multideck.js --quick: monored s13, rogerbrand s3) P2 state-hash DESYNCS. PRE-EXISTING. Hidden-info shadow-sync class (mtg-212 / mtg-259 family).

## CRITICAL FINDING: the GATE tests the WASM browser client, not native

test_network_gui_e2e.js = native server + ONE native client (P1) + ONE BROWSER WASM client (P2). The desync is ALWAYS "P2" = the WASM client. The mtg-vk4b7 native repro (two `mtg connect` clients) exercises a DIFFERENT code path and desyncs at a DIFFERENT point than the gate. Also: `--deck X` only sets the NATIVE P1 deck; the browser P2 uses its dropdown DEFAULT (an old-school Moxen/Scrubland deck), so the gate matchup is monored-vs-default, NOT a mirror, and is not fully deterministic across runs.

## PARTIAL FIXES (branch fix-desync-vk4b7)

### Commit a78c7b9e — core engine (fixes the NATIVE-path desyncs; native 40/40 clean)
1. SearchLibrary tutors (Demonic Tutor) routed through choose_from_library_with_hook (was execute_effect picking library_cards[0] / None on shadow). + placeholder player resolution.
2. Forced fixed-count DiscardCards select by LOWEST CardId (information-independent) instead of CMC heuristic (which needs hidden card identity).
NATIVE repro (two `mtg connect` random clients): monored s13 20/20 CLEAN, rogerbrand s3 20/20 CLEAN. Pre-fix both DESYNC'd (rogerbrand at choice_seq=220).

### Uncommitted/next — WASM client LibraryReordered (real WASM-only bug)
The WASM client's ServerMessage::LibraryReordered handler was a NO-OP (client.rs ~712) while the NATIVE client APPLIES it (sync_callback: `zones.library.cards = new_order.into_iter().rev().collect()`). Fixed by queuing reorders in WasmNetworkClient and applying them BEFORE reveals in all 3 WASM sync_callbacks (ai_harness.rs + fancy_tui.rs replay & normal paths). This is correct but NOT sufficient for the gate.

## REMAINING DESYNC (gate still RED) — precise divergence for next pass

`node web/test_network_gui_e2e.js --deck decks/monored.dck --seed 13` (P2=browser default old-school deck) still desyncs:
  FATAL P2 state hash mismatch at choice_seq=40 action_count=227, Turn 4 "Upkeep".
  SERVER: Life [20,19] Hands [5,3] Libs [52,52] Battlefield 7 Graveyards [0,0] (P2 hand CardIds [116,118,119]).
  COUNTS MATCH between server and client — the divergence is a CARD-PROPERTY or zone-ORDER difference (e.g. Mox tapped state, land/permanent order, or a draw-reveal-timing issue) NOT a count mismatch. The client-side (browser console) state dump is needed to pinpoint — capture WasmNetworkClient debug logs.
  Likely class: WASM shadow reveal-timing (mtg-263) or permanent-order/tap-state divergence with the Moxen/dual-land old-school deck. NOT the library-search or forced-discard class already fixed.

## Next steps
1. Capture the WASM client's shadow state at the divergence (browser console / WasmNetworkClient debug) to identify the exact diverging card/property.
2. Compare native-vs-WASM shadow handling for the failing matchup (the native client of the SAME decks does NOT desync at turn 4 — diff the two client implementations' reveal/sync paths).
3. Consider making the gate deterministic: have the browser P2 load the `--deck` arg too (harness fix, sibling-agent territory).

Related: mtg-212, mtg-259, mtg-229, mtg-263 (WASM reveal timing), mtg-429, mtg-jqkm3 (Hypnotic Specter discard rules gap).
