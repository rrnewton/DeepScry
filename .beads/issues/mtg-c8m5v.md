---
title: 'Card Compatibility: Frostcliff Siege'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:32:56.676887021+00:00
updated_at: 2026-06-06T04:32:56.676887021+00:00
---

# Description

Test all behavioral aspects of Frostcliff Siege in MTG Forge-rs.

Card: cardsfolder/f/frostcliff_siege.txt
Set: TDM/Tarkir: Dragonstorm
Deck: 04 Henry Temur Otters (mtg-684) — sideboard

Card text:
  {1}{U}{R} Enchantment
  As this enchantment enters, choose Jeskai or Temur.
  • Jeskai — Whenever one or more creatures you control deal combat damage to a player, draw a card.
  • Temur — Creatures you control get +1/+0 and have trample and haste.

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{U}{R} Enchantment: WORKING — loads from card file without error
2. [BROKEN] ETBReplacement mode choice (Jeskai/Temur) not triggered: K:ETBReplacement:Other:SiegeChoice references SVar:SiegeChoice with DB$ GenericChoice. The engine checks etb_choose_player flag which requires ApiType::ChoosePlayer, but DB$ GenericChoice is ApiType::Unknown('GenericChoice'). The mode selection prompt never fires.
   Filed as bug in general ETBReplacement SiegeChoice handling (see mtg-xe2n7 for Roaring Furnace for related Room-mechanic issue; Frostcliff uses the same pattern).
3. [BROKEN] Static abilities (Jeskai draw trigger / Temur pump) never activate: since no mode is chosen on ETB, neither ChosenModeJeskai nor ChosenModeTemur is set, so neither S: block fires. Card is a 0-effect enchantment on the battlefield.
4. [BROKEN] Heuristic AI never casts card: with enough mana available, the AI consistently passes without casting Frostcliff Siege (multiple seeds tested). Suggests the heuristic AI does not know how to score this card.
5. [x] Zero controller can cast: confirmed — 'Zero1 casts Frostcliff Siege (18) (putting on stack) / Frostcliff Siege (18) resolves' — card enters battlefield but with no mode (BROKEN).

Reproducer:
```sh
./target/release/mtg tui --p1 zero --p2 zero --seed 42 --verbosity 3 --p1-draw 'Frostcliff Siege;Breeding Pool;Mountain;Island;Breeding Pool' --p2-draw 'Island;Island;Island;Island;Island;Island;Island' debug/frostcliff_test.dck debug/frostcliff_test.dck
```

Expected log evidence (BROKEN — mode choice missing):
```
Zero1 casts Frostcliff Siege (18) (putting on stack)
Frostcliff Siege (18) resolves
```
(No mode choice line appears; card enters with no Jeskai/Temur selection)

CARD STATUS: BROKEN — ETBReplacement SiegeChoice (DB$ GenericChoice) not handled; no mode selected on entry
