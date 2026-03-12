---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-12T02:04:32.634560831+00:00
---

# Description

Track implementation of MTG game features including keywords, abilities, card effects, and the card script parsing infrastructure.

## Active Issues

**Card Parsing & Selectors:**
- mtg-147: Unhandled Affected$ selectors tracking (270 Unknown Affected$ remaining)

**ETB & Replacement Effects:**
- mtg-zeuy0: Thriving Grove doesn't enter tapped or prompt for color choice (affects all Thriving lands)

**Keywords:**
- ✅ Living Weapon keyword parsing fixed (38 cards)
- ✅ For Mirrodin keyword parsing fixed (30 cards)
- ✅ Umbra armor keyword parsing fixed (30 cards)
- ✅ Partner variants (- Survivors, - Father & Son, - Character select) fixed (22 cards)
- ✅ Bare Vanishing (no counter) fixed (4 cards)
- Protection variants ("Protection from each color", "Protection from everything") still TODO

**Variable P/T:**
- ✅ Parsing implemented (X, Y, Z, AffectedX, Count$ references)
- ⏳ Runtime evaluation still TODO (values default to 0)

## Recent Completions

**Partner & Vanishing Keyword Variants (2026-03-12_#1919(4871277)):**
- ✅ "Partner - Survivors" variant (8 cards)
- ✅ "Partner - Father & Son" variant (4 cards)
- ✅ "Partner - Character select" variant (10 cards)
- ✅ Bare "Vanishing" for ETB counter cards (4 cards)
- 26 fewer keyword parsing warnings (608 → 582)

**Keyword Text Variants (2026-03-12_#1917(a5f047a)):**
- ✅ "For Mirrodin" variant (card files omit the "!")
- ✅ "Living Weapon" variant (capital W)
- ✅ "Umbra armor" variant (alternate spelling)
- 98 fewer keyword parsing warnings (706 → 608)

**DealsCombatDamage Triggers (2026-03-12_#1916(bc98cc2)):**
- ✅ Fire DealsCombatDamage triggers at runtime when creatures deal combat damage
- Enables Hypnotic Specter, Ophidian, etc. to work correctly

## Completed Work (older)

**New Effect Types (2026-03-07_#1872(e04b78d)):**
- ✅ ForceSacrifice (891 card usages) - Diabolic Edict, Barter in Blood
- ✅ TapAll (64 card usages) - Sleep, Cryptic Command tap mode
- ✅ UntapAll (100 card usages) - Mobilize, Aggravated Assault
- ✅ SetLife (39 card usages) - Angel of Grace, Blessed Wind

Checked up-to-date as of 2026-03-12_#1919(4871277) - 938 tests passing

# Notes

2026-03-07_#1869: LoseLife (108 cards), DestroyAll (34 cards), DamageAll (58 cards) implemented. Board wipes (Wrath of God) and mass damage (Pyroclasm) now work.
2026-03-10_#1898: AB$ Fight effect (125+ cards, CR 701.12) implemented.
