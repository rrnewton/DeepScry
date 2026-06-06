---
title: 'Bug: Mode$ UnlockDoor trigger not supported (Roaring Furnace / Room cards)'
status: open
priority: 3
issue_type: task
created_at: 2026-06-06T04:30:10.615685149+00:00
updated_at: 2026-06-06T04:30:10.615685149+00:00
---

# Description

Room enchantments (double-faced enchantments with two "Door" halves) use Mode$ UnlockDoor for their triggered abilities. This trigger mode is not implemented in parse_triggers() and falls through silently.

Affected card example: Roaring Furnace (ATLA)
Script line: T:Mode$ UnlockDoor | ValidPlayer$ You | ValidCard$ Card.Self | ThisDoor$ True | Execute$ TrigDamage | TriggerDescription$ When you unlock this door, this Room deals damage equal to the number of cards in your hand to target creature an opponent controls.

Room mechanics also require:
- AlternateMode:Split (MDFC with Room subtype)
- Two halves with separate abilities
- "Unlock" as a sorcery action costs the second half's mana

Findings (2026-06-05_#3008(50175e06)):
- Roaring Furnace enters the battlefield correctly as an enchantment
- But UnlockDoor trigger never fires
- The "unlock door" action (paying mana as a sorcery) may also not be available
- StaticAbilityMode has no "UnlockDoor" variant
- TriggerEvent has no DoorUnlocked variant

CARD STATUS: PARTIAL (enters battlefield, but unlock trigger missing)
