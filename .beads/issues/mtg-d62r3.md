---
title: 'Network desync (native): Bazaar draw-then-discard shadow-sync ordering (mtg-u3dwj deeper part)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-05T02:46:11.921815139+00:00
updated_at: 2026-06-05T02:46:11.921815139+00:00
---

# Description

## Network desync (NATIVE, deterministic): Bazaar of Baghdad "draw 2, discard 3" — shadow discard decided BEFORE drawn cards materialised

## Status: FIXED (branch fix-allhallows-eve, on netarch base @44d7919a)

Deterministic local-vs-network gamelog divergence on the rogerbrand mirror,
seed 3, HEURISTIC controller (the DEEPER part of mtg-u3dwj, separate from the
already-fixed Bazaar reveal-timing race). Repro:

    bug_finding/fuzz_determinism_netequiv.sh --invariant equivalence \
      --decks decks/old_school/01_rogue_rogerbrand.dck --pair-mode self \
      --start-seed 3 --seeds 1 --controllers heuristic --keep-logs

Before fix: local-vs-network gamelog diff = 377 lines. After fix: 5/5 GREEN,
plus seeds 1-6 heuristic+random all GREEN.

## ROOT CAUSE (first divergent state-field)

First divergence: Turn 8 M2, Gabriel's Bazaar of Baghdad discard.
- LOCAL (full state): discards the two just-drawn lands Scrubland + Badlands first.
- NETWORK (client shadow): discards Animate Dead x2 instead, KEEPING the lands.

Gabriel is a network CLIENT; its heuristic decides the discard on its SHADOW
state. The heuristic `choose_cards_to_discard` sorts the hand by value (lands=0
=> discarded first). On the shadow, the two cards just drawn by Bazaar (in the
SAME atomic ability resolution) were NOT yet materialised, so they were dropped
from the candidate set (release-mode `debug_assert!(false)` on an unresolvable
own card is a NO-OP, identical to the old `filter_map`) and the heuristic
discarded the wrong cards — an information-independence violation
(docs/NETWORK_ARCHITECTURE.md: controllers MUST decide identically on full vs
shadow state; desync is ALWAYS fatal). No fatal hash mismatch fires because the
server obeys the client's (wrong) choice, so server and shadow stay hash-equal
while diverging from the full-state local game.

Why the drawn cards were missing: the discard ChoiceRequest's catch-up buffer
(assemble_choice_buffer) DOES carry the two draw reveals at their own acs (server
log confirms "collected 2 reveals ... action_count=295", cards 109/108). But the
DiscardCards handler (game_loop/priority.rs ~1899) called `sync_to_action()`
BEFORE the controller received that request, so the buffer reveals were not yet
in the shadow's reveal_log when the sync ran (reveals apply bounded by
max_received_choice_ac ∧ reveal_log.frontier — both below the draw acs until the
request arrives). NetworkLocalController.choose_cards_to_discard then receives the
request (get_choice_info) but does NO sync afterward, so the heuristic decides on
the un-materialised shadow. This INVERTS the proven priority-loop order (prepare
=> sync => decide, priority.rs ~551-575 "NETWORK SYNC PROTOCOL").

Confirmed empirically (client2 shadow log): the Bazaar draw reveals apply at the
NEXT ChoiceRequest's sync (game_action=302), AFTER the discard already executed.
The matching Turn-6 Bazaar did NOT diverge only because that turn's drawn cards
would have been kept anyway (drawn cards not among the discards).

## FIX

game_loop/priority.rs: in BOTH structurally identical draw-then-discard sites,
RECEIVE the discard ChoiceRequest (whose buffer carries the just-drawn cards'
reveals) BEFORE syncing the shadow — `controller.prepare_for_priority_choice()`
then `self.sync_to_action()` then decide, mirroring the priority-loop "NETWORK
SYNC PROTOCOL" order:
  1. ACTIVATED/triggered-ability discard handler (~1950): Bazaar of Baghdad,
     Jalum Tome, etc.
  2. SPELL-resolution discard handler `resolve_top_spell_with_discard_hook`
     (~3247): Careful Study, Frantic Search, Thirst for Knowledge, Compulsive
     Research, Blast of Genius, Ancient Excavation, Artificer's Epiphany, ...
     This path had the IDENTICAL bug (synced before receiving the request) and
     was MISSED by the first cut of this fix.

EMPTY-HAND GUARD (BLOCKER-1, regression the first cut introduced): the blocking
`prepare_for_priority_choice()` MUST sit BELOW the `if actual_count == 0
{ continue }` guard (activated path) / INSIDE the `if actual_count > 0` guard
(spell path). On an empty-hand targeted discard (e.g. a Mind-Rot-style "target
player discards N" vs a 0-card opponent) the server computes actual_count==0,
sends NO ChoiceRequest, and `continue`s. If the client blocked on prepare there
it would HANG forever, and because `take_local_choice` is a blind FIFO pop it
could instead pop the NEXT request → answer request N+1 to choice N → off-by-one
FATAL desync. Gating the block restores the invariant: a network block happens
IFF a request will be sent.

WHY SERVER/LOCAL ARE SAFE: `prepare_for_priority_choice()` is a no-op default for
non-network controllers AND for the authoritative server. The server is correct
NOT because of any sync hook (note: `with_pre_choice_hook` has ZERO call sites
repo-wide — it is dead code and is NOT relied on here) but because the server
runs the FULL authoritative game: its own hand already holds the just-drawn
cards, and it merely relays the client's chosen indices. The bug is purely on the
client SHADOW, whose drawn cards are unmaterialised until the buffer is applied.
`prepare` is idempotent (the subsequent choose_discard_with_hook reuses the
cached request via get_choice_info), so it is safe for the remote (opponent) seat
too. NOT applied to the Loot discard sites (discard-THEN-draw: no just-drawn
cards to miss).

No engine rule logic changed — this is network shadow-apply ORDERING.

## Relationship to mtg-609 (sibling) and mtg-o99ow

Same family as mtg-609 (WASM-shadow All Hallow's Eve / mass-effect sequencing),
which was already fixed/closed via a different mechanism (begin-of-upkeep
re-entry guard). This is the NATIVE-client analogue — distinct code path
(NetworkLocalController vs WASM controller), distinct fix site. The netarch
buffer rearchitecture (mtg-o99ow) fixed the EARLIER Bazaar reveal-timing race;
this fixes the residual in-resolution local-choice sync-ordering gap it left.

## MTG Rules Review — Verdict: PASS
1. Rule impl: N/A for rule semantics — network shadow-apply ordering only.
   Bazaar of Baghdad "{T}: Draw two cards, then discard three cards." (CR 120
   draw, CR 701 discard) unchanged. The discard remains the controller's choice.
2. Reveal ordering: STRENGTHENED — the controller now sees its just-drawn cards
   (its own reveals, applied before the decision) exactly as the full-state
   engine does. This IS the fix.
3. Information hiding: PRESERVED — only the deciding player's OWN drawn cards are
   materialised (their own entitled reveals); no opponent/library hidden info
   exposed. No new data sent.
4. Decision authority: PRESERVED — the discard is still chosen by the client's
   PlayerController; the server only requests + applies the client's choice.
5. Server/client sync: STRENGTHENED — removes an information-independence
   divergence; desync stays fatal (no silent recovery). Verified equivalence
   5/5 + seeds 1-6 heuristic/random GREEN; full make validate GREEN.
6. Workaround vs real fix: REAL FIX — corrects the sync ORDER at the root
   (prepare=>sync=>decide), no card-specific special-case, no skipped event.
7. Bug-class: class-level for the draw-then-discard family — fixes BOTH the
   activated/triggered-ability discard path (Bazaar, Jalum Tome) AND the
   spell-resolution discard path (Careful Study, Frantic Search, Thirst for
   Knowledge, ...). Explicitly NOT the Loot path (discard-THEN-draw — no
   just-drawn cards to materialise before the choice). The empty-hand guard
   makes the fix safe for targeted discards vs an empty-handed opponent.
