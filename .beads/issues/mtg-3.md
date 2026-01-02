---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2026-01-02T04:47:45.609994847+00:00
---

# Description

Track implementation of MTG game features including keywords, abilities, card effects, and the card script parsing infrastructure.

## Active Issues

**Card Parsing & Selectors:**
- mtg-147: Unhandled Affected$ selectors tracking (978 warnings reduced)

**Keywords:**
- Living Weapon keyword unimplemented (38 cards)
- Protection variants ("Protection from each color")

**Variable P/T:**
- ✅ Parsing implemented (X, Y, Z, AffectedX, Count$ references)
- ⏳ Runtime evaluation still TODO (values default to 0)

## Recent Fixes (2026-01-02)

**Avatar Set Mana Engine Fixes (2026-01-02):**
- ✅ Ba Sing Se (non-basic land with Fixed mana production) now taps correctly for {G}
  - Root cause: tap_for_mana_for_cost didn't handle ManaProductionKind::Fixed for non-basic lands
  - Fix: Added match arm for Fixed mana production in actions/mod.rs
- ✅ Foggy Swamp Vinebender no longer incorrectly marked as mana source
  - Root cause: Name-based fallback detected "Swamp" in creature name and set has_swamp_subtype=true
  - Fix: Guard name-based land subtype detection with `if self.is_land` check
- Avatar decks now play 200+ seeds without mana errors

## Completed Work (2025-12-04_#1134)

**Variable P/T Parsing (2025-12-04_#1131(4cec306)):**
- ✅ Accept AddPower$/AddToughness$ with X, Y, Z, -X, AffectedX
- ✅ Accept Count$ expressions and named variables
- ✅ Parse as 0 placeholder until SVar evaluation implemented

**EnchantedBy Selectors (2025-12-04_#1133(bb82a4b)):**
- ✅ Artifact.EnchantedBy, Planeswalker.EnchantedBy, Equipment.EnchantedBy

**Trigger Self-Only Fix (2025-12-01_#1057(6d87c69)):**
- ✅ ETB triggers now correctly only fire for Card.Self triggers
- ✅ Added trigger_self_only field to Trigger struct

**Death Triggers (2025-11, commit 6b4ff21):**
- ✅ Parse "dies" triggers (Mode$ ChangesZone)
- ✅ Execute death triggers before moving to graveyard
- ✅ Su-Chi death trigger adds {C}{C}{C}{C} correctly

**Upkeep Triggers (2025-11, commit a11add5):**
- ✅ Parse upkeep triggers (Mode$ Phase | Phase$ Upkeep)
- ✅ ValidPlayer$ You filtering for controller-only triggers

**Equipment System (2025-11):**
- ✅ Equip ability timing, target validation, attachment
- ✅ Basic static buffs (+N/+N)

**Mana Effects (2025-12-04_#1130(72d1030)):**
- ✅ AddMana effect player placeholder resolution
- ✅ Dark Ritual and similar mana rituals now work correctly

## Related Issues
- mtg-111: Phase triggers / Execute$ SVar resolution
- mtg-147: Affected$ selector parsing improvements

---
**Checked up-to-date as of 2026-01-02**
