---
title: 'MTG feature completeness: keywords, abilities, effects'
status: open
priority: 1
issue_type: epic
labels:
- tracking
created_at: 2025-10-26T21:06:34+00:00
updated_at: 2025-11-23T00:00:00+00:00
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

See individual mtg-* issues for specific features and bugs.

---
Updated 2025-11-23_#895(2eabce7)
