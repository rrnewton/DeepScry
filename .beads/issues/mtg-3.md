---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-01-03T03:30:02.918195915+00:00
---

# Description

Track implementation of MTG game features including keywords, abilities, card effects, and the card script parsing infrastructure.

## Active Issues

**Card Parsing & Selectors:**
- mtg-147: Unhandled Affected$ selectors tracking (792 warnings remaining, 70% reduction from 2,672)

**ETB & Replacement Effects:**
- mtg-zeuy0: Thriving Grove doesn't enter tapped or prompt for color choice (affects all Thriving lands)

**Keywords:**
- Living Weapon keyword unimplemented (38 cards)
- Protection variants ("Protection from each color")

**Variable P/T:**
- ✅ Parsing implemented (X, Y, Z, AffectedX, Count$ references)
- ⏳ Runtime evaluation still TODO (values default to 0)

## Recent Fixes (2026-01-03_#1475)

**Affected$ Selector Expansion (2026-01-03):**
- ✅ Dynamic Subtype.YouOwn parsing (Merfolk.YouOwn, Druid.YouOwn, etc.)
- ✅ CardType.TopLibrary+YouCtrl patterns (Instant, Sorcery)
- ✅ Permanent.Subtype+YouCtrl patterns (Servo, Thopter)
- ✅ Card.EquippedBy+TYPE patterns (Human, Angel)
- ✅ Artifact.nonCreature+YouCtrl, Artifact.Creature+YouCtrl+Other
- Warning count: 854 → 792 (62 fewer warnings)

**Avatar Set Mana Engine Fixes (2026-01-02):**
- ✅ Ba Sing Se (non-basic land with Fixed mana production) now taps correctly for {G}
- ✅ Foggy Swamp Vinebender no longer incorrectly marked as mana source
- Avatar decks now play 200+ seeds without mana errors

## Completed Work (2025-12-04_#1134)

**Variable P/T Parsing (2025-12-04_#1131(4cec306)):**
- ✅ Accept AddPower$/AddToughness$ with X, Y, Z, -X, AffectedX
- ✅ Accept Count$ expressions and named variables
- ✅ Parse as 0 placeholder until SVar evaluation implemented

Checked up-to-date as of 2026-01-03.
