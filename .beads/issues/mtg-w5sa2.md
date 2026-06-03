---
title: 'monored seed13 turn-18 rewind drift: cards[N].keywords (Rockface haste) + mana_state_version not restored across rewinds'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-03T01:20:26.813516624+00:00
updated_at: 2026-06-03T01:20:26.813516624+00:00
---

# Description

Pre-existing (NOT introduced by mtg-610 A2 combat fix; reproduces independent of combat). Pre-merge blocker for the netarch branch (full make validate / multideck gate). Regression-adjacent to closed mtg-379 (mana_state_version in replay hash) + mtg-586 (monored desync).

REPRO: cd web && node test_network_gui_e2e.js --deck decks/monored.dck --seed 13  (fails ~1/3). FATAL: 'turn-start state hash for turn 18 changed across rewinds'.

EXACT DRIFT (from the WASM replay verifier [VERIFIER FIELD DIFF] dump):
- cards[46].keywords  — card 46 is a creature granted HASTE-until-EOT by Rockface Village (116). Its stored keywords KeywordSet differs between two rewinds to turn-18-start.
- mana_state_version  — the mana-state counter differs across rewinds (mtg-379 territory; likely reset_turn_state mana_pool.clear() not undo-logged -> version counter drifts, same A1 'reset_turn_state transients not undo-logged' class).

WHY THE EXISTING FIX IS INSUFFICIENT: undo.rs rewind_to_turn_start ALREADY blanket-clears temp_keywords_until_eot for every card (added for the earlier 'turn-12 haste drift', comment at undo.rs:~1576). Yet cards[46].keywords STILL drifts at turn-18-start. So the bit in  is NOT being removed by clear_temp_keywords_until_eot — meaning either (a) a keyword is inserted into card.keywords OUTSIDE grant_keyword_until_eot (so temp_keywords_until_eot doesn't track it, clear can't remove it), or (b) keywords are recomputed from a continuous/static effect between the clear and the turn-start snapshot, re-adding a board-state-dependent bit. Grant sites: actions/mod.rs:3647/3698/3849 (all use grant_keyword_until_eot). NEXT: instrument card 46's keywords through the forward grant + both rewinds to see which bit survives the clear and where it's (re)added; then make the grant undo-logged (mirror SetTempBaseStats/restore_temp_base_stats) AND ensure mana_state_version is restored on rewind (or excluded from the hash per mtg-379's approach). Two distinct fixes likely needed.

Both fields are in the reset_turn_state / undo-log-completeness class the rewind vision targets — belongs in the netarch branch, not deferred.
