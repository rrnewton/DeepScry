---
title: 'Card Compatibility: Enduring Vitality'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:32:28.087775596+00:00
updated_at: 2026-06-06T04:32:28.087775596+00:00
---

# Description

Test all behavioral aspects of Enduring Vitality in MTG Forge-rs.

Card: cardsfolder/e/enduring_vitality.txt
Set: DSK/Duskmourn: House of Horror
Deck: 04 Henry Temur Otters (mtg-684)

Card text:
  {1}{G}{G} Enchantment Creature — Elk Glimmer (3/3)
  Vigilance
  Creatures you control have '{T}: Add one mana of any color.'
  When Enduring Vitality dies, if it was a creature, return it to the battlefield under its owner's control. It's an enchantment. (It's not a creature.)

Findings (2026-06-06_#3008(50175e06), agent slot04):

1. [x] Parses as {1}{G}{G} 3/3 Enchantment Creature with Vigilance: WORKING
2. [x] Resolves and enters battlefield: WORKING — 'Enduring Vitality (56) resolves / Enduring Vitality (56) enters the battlefield as a 3/3 creature' (seed 42 heuristic game)
3. [x] Attacks as 3/3: WORKING — 'Enduring Vitality (56) deals 3 damage to AI-Heuristic2 (life: 17)' (seed 42)
4. [unverified] Static ability (creatures gain mana tap): 'Creatures you control have {T}: Add one mana of any color' — the SVar:AnyMana with AB$ Mana | Produced$ Any is scripted but not observed firing in logs
5. [unverified] Death trigger (return as pure enchantment): S:Mode$ Continuous with AddType$ Enchantment + RemoveCardTypes$ True on return — not triggered in observed games (Vitality survived)

Reproducer:
```sh
./target/release/mtg tui --p1 heuristic --p2 heuristic --seed 42 --verbosity 2 decks/championship/2025/04_henry_temur_otters.dck decks/championship/2025/01_manfield_izzet_lessons.dck
```

Expected log evidence:
```
Enduring Vitality (56) resolves
Enduring Vitality (56) enters the battlefield as a 3/3 creature
Enduring Vitality (56) deals 3 damage to AI-Heuristic2 (life: 17)
```

CARD STATUS: PARTIAL — basic creature behavior WORKING; mana-tap static and death-return-as-enchantment not verified
