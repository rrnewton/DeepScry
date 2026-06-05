---
title: 'PRINCIPLED: generalize rewind re-materialization to carry ALL per-instance state via reveal-actionlog (not per-field reconstruction)'
status: open
priority: 2
issue_type: task
created_at: 2026-06-05T17:36:16.670285632+00:00
updated_at: 2026-06-05T21:30:38.277155297+00:00
---

# Description

slot04 desync-review 2026-06-05 (no-band-aid direction). Root issue: re-materializing an opponent permanent on rewind uses the blank template and loses EVERY per-instance fact (tapped, controller, damage, counters, P/T bonus, summoning-sickness, attachments, chosen_color). Field-by-field reconstruction (tapped done, controller + tap-sites next) is whack-a-mole. Principled fix: carry the full per-instance state through the reveal-actionlog unification (mtg-o99ow) so NO field is lost — subsumes the controller + tap-site + unhashed-field gaps. Immediate prize blockers (7/11/19) may be fixed first, but this is the durable fix. Related: mtg-o99ow, mtg-677.

=== SEED-7 SEARCH-TO-HAND HALF: RESOLVED + MERGED 2026-06-05 (@6a708dda, commit bc4b0e29) ===
The observer/in-stack search-to-hand divergence (seed 7) is FIXED and on integration. Root cause (4th-framing, entry-probe-verified — the earlier actions/mod.rs:4777 try_get pin was a probe-after-early-return MISREAD): the observer resolves an opponent's hidden-library tutor via choose_from_library_with_hook's !is_network_mode branch; its valid_cards is only the MATERIALIZED subset (4 of server's 37), and the code fell back to the authoritative fetched CardId ONLY when valid_cards.is_empty() — a PARTIAL non-empty list indexed valid_cards[index] (server's index addresses ITS full 37-list) -> wrong card or None -> lost fetch -> hand 5 vs 6 -> fatal. FIX (2 parts): (1) network_choice.rs: for a Remote controller, ignore the partial index and use the authoritative rewind-surviving searched_card_lookup -> take_library_search_result; (2) network/client.rs: native searched_card_for (ac-keyed dummy-Searched reveal scan) + with_searched_card_lookup wiring (was WASM-only). GATED: independent robots re-verify (seed 7 PASS 281ch/21t; seeds 2/11 no-regression; 8 prior-green stay green), adversarial desync-review CLEAR, mtg-rules-review PASS, full hermetic make validate 34/34 green. Defense-in-depth follow-up: mtg-8y0zn (assert <=1 empty-name Searched reveal per ac+searcher). REMAINING for the prize: seed 19 (mtg-8ow9h man-land animation continuous-effect reconstruction) is the LAST blocker; the broader full-per-instance-state carry above is still the durable umbrella.
