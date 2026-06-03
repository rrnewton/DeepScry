---
title: 'monored seed13 turn-18 rewind drift: cards[N].keywords (Rockface haste) + mana_state_version not restored across rewinds'
status: in_progress
priority: 2
issue_type: bug
created_at: 2026-06-03T01:20:26.813516624+00:00
updated_at: 2026-06-03T03:21:34.609332285+00:00
---

# Description

Pre-existing (NOT introduced by mtg-610 A2 combat fix; reproduces independent of combat). Pre-merge blocker for the netarch branch (full make validate / multideck gate). Regression-adjacent to closed mtg-379 (mana_state_version in replay hash) + mtg-586 (monored desync).

REPRO: cd web && node test_network_gui_e2e.js --deck decks/monored.dck --seed 13  (fails ~1/3). FATAL: 'turn-start state hash for turn 18 changed across rewinds'.

EXACT DRIFT (from the WASM replay verifier [VERIFIER FIELD DIFF] dump):
- cards[46].keywords  — card 46 is a creature granted HASTE-until-EOT by Rockface Village (116). Its stored keywords KeywordSet differs between two rewinds to turn-18-start.
- mana_state_version  — the mana-state counter differs across rewinds (mtg-379 territory; likely reset_turn_state mana_pool.clear() not undo-logged -> version counter drifts, same A1 'reset_turn_state transients not undo-logged' class).

WHY THE EXISTING FIX IS INSUFFICIENT: undo.rs rewind_to_turn_start ALREADY blanket-clears temp_keywords_until_eot for every card (added for the earlier 'turn-12 haste drift', comment at undo.rs:~1576). Yet cards[46].keywords STILL drifts at turn-18-start. So the bit in  is NOT being removed by clear_temp_keywords_until_eot — meaning either (a) a keyword is inserted into card.keywords OUTSIDE grant_keyword_until_eot (so temp_keywords_until_eot doesn't track it, clear can't remove it), or (b) keywords are recomputed from a continuous/static effect between the clear and the turn-start snapshot, re-adding a board-state-dependent bit. Grant sites: actions/mod.rs:3647/3698/3849 (all use grant_keyword_until_eot). NEXT: instrument card 46's keywords through the forward grant + both rewinds to see which bit survives the clear and where it's (re)added; then make the grant undo-logged (mirror SetTempBaseStats/restore_temp_base_stats) AND ensure mana_state_version is restored on rewind (or excluded from the hash per mtg-379's approach). Two distinct fixes likely needed.

Both fields are in the reset_turn_state / undo-log-completeness class the rewind vision targets — belongs in the netarch branch, not deferred.

--- 2026-06-02 netarch-dev4: ROOT FOUND + FIXED (keyword half). The hypothesis was BACKWARDS. ---
The drifting bit is NOT an extra haste SURVIVING the clear — it is a PRINTED haste being STRIPPED. Reproduced monored seed13 (turn 14 this build): [VERIFIER FIELD DIFF] cards[49].keywords PRIOR={Haste}([16,0]) CURRENT={}([0]); card 49 = Screaming Nemesis, which has PRINTED K:Haste. EnumSet repr=array: word0 bit4 = Haste.

MECHANISM: Rockface Village pump (AB$ Pump | KW$ Haste) calls grant_keyword_until_eot(Haste) on a creature that ALREADY has printed Haste. The old impl inserted Haste into temp_keywords_until_eot unconditionally, so clear_temp_keywords_until_eot (forward EOT cleanup AND the rewind sweep) AND the PumpCreature undo all REMOVED the printed Haste. First obs of turn-14-start had printed Haste; a later rewind that popped the pump stripped it -> turn-start hash drift. This was ALSO a real forward-play bug (a pumped Screaming Nemesis loses printed Haste at EOT).

FIX (mirrors the AnimateTypeline granted-keyword guard): grant_keyword_until_eot now only tracks (and returns true for) keywords it NEWLY adds to the live set; PumpCreature/PumpCreatureVariable log only newly-granted keywords. Printed/other-source keywords are never tracked as temp -> never stripped. card.rs + actions/mod.rs. New unit regression test grant_keyword_until_eot_never_strips_printed_keyword.

RESULT: monored seed13 9/10 (was ~2/3). mana_state_version diff in the dump is INFORMATIONAL only — it is already excluded from EXCLUDED_FIELDS_REWIND_VERIFIER (line 79), so it does NOT affect the verifier hash; the keyword bit was the sole hash-changer.

REMAINING (separate root, ~1/10): a cross-machine compute_view_hash desync at combat (NOT the turn-start verifier). Run 4: fork ~action_count 1446 right after 'Hired Claw deals 2 damage to Emberheart Challenger (44) blocker', with a 'NetworkController: target_card_ids [44] not in remaining_blockers, falling back to index' WARN. Likely A2-adjacent multi-blocker/blocker-identity combat issue, NOT keyword. Tracking under continued netarch combat work.
