---
title: 'Defense-in-depth: assert <=1 empty-name Searched reveal per (action_count, searcher) in reveal_log'
status: open
priority: 4
issue_type: task
created_at: 2026-06-05T21:29:20.699978934+00:00
updated_at: 2026-06-05T21:29:20.699978934+00:00
---

# Description

The seed-7 fix (mtg-ho2r8, landed @bc4b0e29) resolves an OPPONENT's hidden-library search on the observer via searched_card_for(searcher, target_action) — scans reveal_log for the dummy empty-name Searched reveal with greatest action_count <= target. Correct BECAUSE each fetched card gets its own ChoicePoint at a DISTINCT action_count, hence its own dummy reveal. INVARIANT: at most one empty-name Searched reveal per (action_count, searcher). Today NO card violates this (grep cardsfolder/: zero ChangeNum 2+/X Library->Hand searches; Dig-keep loop emits a per-iteration ChoicePoint so each fetch lands at a distinct ac) -> UNREACHABLE today. RISK (latent): a future atomic multi-fetch emitting ONE ChoicePoint for two opponent fetches at the SAME ac would make searched_card_for return the same single CardId twice -> drop the 2nd -> hand-size desync. GUARD (both native client.rs::searched_card_for AND wasm/network/client.rs::searched_card_for): assert <=1 empty-name Searched reveal per (ac, searcher), fatal on a 2nd (consistent with the reorder_log/reveal_log 'distinct same-class delta per ac is fatal' invariant). Found by adversarial desync-review of bc4b0e29 (CLEAR; this the single tracked theoretical concern). Related: mtg-ho2r8, mtg-725.
