---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-03-07T22:00:42.732486542+00:00
---

# Description

Track implementation of MTG game features including keywords, abilities, card effects, and the card script parsing infrastructure.

## Active Issues

**Card Parsing & Selectors:**
- mtg-147: Unhandled Affected$ selectors tracking (772 warnings remaining, 71% reduction from 2,672)

**ETB & Replacement Effects:**
- mtg-zeuy0: Thriving Grove doesn't enter tapped or prompt for color choice (affects all Thriving lands)

**Keywords:**
- Living Weapon keyword unimplemented (38 cards)
- Protection variants ("Protection from each color")

**Variable P/T:**
- ✅ Parsing implemented (X, Y, Z, AffectedX, Count$ references)
- ⏳ Runtime evaluation still TODO (values default to 0)

## Recent Completions

**New Effect Types (2026-03-07_#1872(e04b78d)):**
- ✅ ForceSacrifice (891 card usages) - Diabolic Edict, Barter in Blood
- ✅ TapAll (64 card usages) - Sleep, Cryptic Command tap mode
- ✅ UntapAll (100 card usages) - Mobilize, Aggravated Assault
- ✅ SetLife (39 card usages) - Angel of Grace, Blessed Wind

**Previous Effect Types (2026-03-07_#1869(4dbfd3b)):**
- ✅ LoseLife (108 card usages) - Drain Life, Sign in Blood
- ✅ DestroyAll (34 card usages) - Wrath of God, Day of Judgment
- ✅ DamageAll (58 card usages) - Pyroclasm, Earthquake

## Completed Work (older)

**Affected$ Selector Expansion (2026-01-03_#1477):**
- ✅ Card.Treasure+YouCtrl, Card.YouCtrl+wasCast, Card.Self+TopLibrary
- ✅ Instant.COLOR+YouCtrl, Sorcery.COLOR+YouCtrl
- ✅ Dynamic Subtype.YouOwn parsing (Merfolk.YouOwn, etc.)
- Warning count: 854 → 772

**Variable P/T Parsing (2025-12-04_#1131(4cec306)):**
- ✅ Accept AddPower$/AddToughness$ with X, Y, Z, -X, AffectedX

Checked up-to-date as of 2026-03-10_#1898(7de2da0) - 891 tests passing

# Notes

2026-03-07_#1869: LoseLife (108 cards), DestroyAll (34 cards), DamageAll (58 cards) implemented. Board wipes (Wrath of God) and mass damage (Pyroclasm) now work.
2026-03-10_#1898: AB$ Fight effect (125+ cards, CR 701.12) implemented.
