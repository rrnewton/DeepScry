---
title: 'robots42 seed=42 intermittent WASM rewind+replay desync: pending_cast resume double-resolves a draw spell'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-02T19:39:54.432003632+00:00
updated_at: 2026-06-03T16:39:48.913823088+00:00
---

# Description

robots42 seed=42 intermittent WASM rewind+replay desync (netarch STEP-3).

========================================================================
STATUS 2026-06-03 (slot01 — sig-1 LIBRARY-SEARCH FIX LANDED; robots42 7/10 → working sigs 2/3):

sig-1 (opponent-shadow hidden-info library search) is FIXED via the reveal-history-buffer vehicle.

ROOT CAUSE (confirmed by robots42 dumps): on P_viewer's shadow, when the OPPONENT tutors (e.g. Demonic Tutor → Copy Artifact), the server cannot reveal WHICH card (hidden info) so it sends a single DUMMY `Searched` reveal: empty name, but carrying the AUTHORITATIVE fetched card_id, owner=searcher, stamped with the search choice's action_count (server.rs ~2933). The pre-fix `choose_from_library_with_hook` recorded the fetch from the RACED `take_library_search_result` (OpponentChoice.library_search_result), which is absent at the FIRST resolution → recorded `LibrarySearch(None)` → replayed forever → opponent library count NOT decremented → compute_view_hash desync.

FIX (4 files):
- game_loop/mod.rs: new `SearchedCardLookup` GameLoop hook + `with_searched_card_lookup` builder + `searched_card_lookup()` accessor.
- game_loop/network_choice.rs: in `choose_from_library_with_hook` non-network (WASM AI) branch, when valid_cards is empty (shadow opponent search) source the fetched CardId from the rewind-surviving lookup FIRST, falling back to the raced `take_library_search_result`. A genuine fail-to-find (CR 701.19c) has no Searched reveal → lookup None → correctly records the decline.
- wasm/network/client.rs: `searched_card_for(searcher, target_action)` reads the reveal-history buffer for the EMPTY-NAME (opponent dummy) `Searched` reveal owned by searcher with greatest effective_ac ≤ target_action. (Our own search gets MULTIPLE named candidate reveals + non-empty valid_cards, so the lookup is not consulted for it.)
- wasm/fancy_tui.rs: `make_ai_searched_card_lookup` wired on BOTH `run_network_ai_forward` and `run_network_ai_replay` GameLoops.

The shadow uses late-binding reserved CardID slots (game_init.rs): the opponent's library holds the REAL card_ids (uninstantiated), so recording `LibrarySearch(Some(real_id))` + move_card(id, Library, Hand) correctly decrements the opponent library count while identity stays hidden.

NATIVE MULTI-REWIND REPRODUCER (RED-proven): game_loop/mod.rs `#[cfg(test)] mod tests`:
- `opponent_library_search_fetch_lost_when_only_raced_source` — negative guard: records None, replay returns None forever.
- `opponent_library_search_fetch_survives_multi_rewind_via_lookup` — with lookup: records Some(fetched), survives 5 replay cycles. PROVEN RED when the production lookup is bypassed (left=None, right=Some(3)).

GATE: robots42 seed=42 ×10 = 7 PASS / 3 FAIL (was ~3-6/10). 3 remaining fails = sig-2 (mass-draw Timetwister/Wheel content divergence) + sig-3 (available-actions drift, downstream). Same hidden-info-replay class. NEXT: sig-2 mass-draw/shuffle RNG-state capture. Then mtg-t233k mana_pool gap. Then rebase onto origin/integration (advanced: rename→deepscry + beads renumber), full make validate, rules-review, re-enable robots42 in web/test_network_multideck.js.

Commit: sig-1 fix on netarch-undo-holes (slot01). Build green (release+network, wasm-network), clippy clean, fmt clean, native oracle/network_e2e/reproducer all green.
========================================================================
