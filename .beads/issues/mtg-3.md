---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-12T01:55:29.959815984+00:00
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
- Protection variants ("Protection from each color") still TODO

**Variable P/T:**
- ✅ Parsing implemented (X, Y, Z, AffectedX, Count$ references)
- ⏳ Runtime evaluation still TODO (values default to 0)

## Recent Completions

**Keyword Text Variants (2026-03-12_#1917(a5f047a)):**
- ✅ "For Mirrodin" variant (card files omit the "!")
- ✅ "Living Weapon" variant (capital W)
- ✅ "Umbra armor" variant (alternate spelling)
- 98 fewer keyword parsing warnings (706 → 608)

**DealsCombatDamage Triggers (2026-03-12_#1916(bc98cc2)):**
- ✅ Fire DealsCombatDamage triggers at runtime when creatures deal combat damage
- Enables Hypnotic Specter, Ophidian, etc. to work correctly

**New Effect Types (2026-03-07_#1872(e04b78d)):**
- ✅ ForceSacrifice (891 card usages) - Diabolic Edict, Barter in Blood
- ✅ TapAll (64 card usages) - Sleep, Cryptic Command tap mode
- ✅ UntapAll (100 card usages) - Mobilize, Aggravated Assault
- ✅ SetLife (39 card usages) - Angel of Grace, Blessed Wind

## Completed Work (older)

**Previous Effect Types (2026-03-07_#1869(4dbfd3b)):**
- ✅ LoseLife (108 card usages) - Drain Life, Sign in Blood
- ✅ DestroyAll (34 card usages) - Wrath of God, Day of Judgment
- ✅ DamageAll (58 card usages) - Pyroclasm, Earthquake

**Affected$ Selector Expansion (2026-01-03_#1477):**
- ✅ Card.Treasure+YouCtrl, Card.YouCtrl+wasCast, Card.Self+TopLibrary
- ✅ Instant.COLOR+YouCtrl, Sorcery.COLOR+YouCtrl
- ✅ Dynamic Subtype.YouOwn parsing (Merfolk.YouOwn, etc.)

Checked up-to-date as of 2026-03-12_#1917(a5f047a) - 938 tests passing

# Notes

2026-03-07_#1869: LoseLife (108 cards), DestroyAll (34 cards), DamageAll (58 cards) implemented. Board wipes (Wrath of God) and mass damage (Pyroclasm) now work.
2026-03-10_#1898: AB$ Fight effect (125+ cards, CR 701.12) implemented.
