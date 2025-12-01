---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-12-01T11:30:19.830362978+00:00
---

# Description

Track implementation of MTG game features including keywords, abilities, card effects, and the card script parsing infrastructure.

## Active Issues

**Card Parsing & Selectors:**
- mtg-147: Unhandled Affected$ selectors (EnchantedBy, EquippedBy, tribal types, compound selectors)
- Many static abilities silently fail due to unknown selectors

**Keywords:**
- Living Weapon keyword unimplemented
- Protection variants ("Protection from each color")
- Variable power/toughness (X, Y, Z) parsing

**Equipment:**
- ✅ Basic Equipment implemented (mtg-77 series)
- Variable buffs (AddPower$ X) not yet supported

## Completed Work

**Trigger Self-Only Fix (2025-12-01_#1057(6d87c69)):**
- ✅ ETB triggers now correctly only fire for Card.Self triggers
- ✅ Added trigger_self_only field to Trigger struct
- ✅ Spider-Ham and similar cards with ETB triggers no longer incorrectly fire when other copies enter

**Death Triggers (2025-11, commit 6b4ff21):**
- ✅ Parse "dies" triggers (Mode$ ChangesZone with Origin$ Battlefield, Destination$ Graveyard)
- ✅ Execute death triggers before moving creatures to graveyard
- ✅ Su-Chi death trigger adds {C}{C}{C}{C} correctly
- ✅ Deterministic trigger ordering when multiple creatures die

**Upkeep Triggers (2025-11, commit a11add5):**
- ✅ Parse upkeep triggers (Mode$ Phase | Phase$ Upkeep)
- ✅ ValidPlayer$ You filtering for controller-only triggers
- ✅ Juzám Djinn / Serendib Efreet damage triggers work

**Equipment System (2025-11):**
- ✅ Equip ability timing (sorcery-speed)
- ✅ Target validation
- ✅ Attachment mechanics
- ✅ Basic static buffs (+N/+N)
- ✅ E2E tests with wildcards

**Deck Loading:**
- ✅ Foil card parsing (ee5d90b) - Strip '+' suffix from card names

**Bug Fixes (2025-11):**
- ✅ Entity not found: 0 bug (2eabce7) - Fixed activated abilities being parsed as spell effects

## Related Issues
- mtg-111: Phase triggers / Execute$ SVar resolution
