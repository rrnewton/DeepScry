---
title: Modal spells (SP$ Charm) not implemented
status: open
priority: 2
issue_type: bug
created_at: 2026-01-03T00:41:33.466026335+00:00
updated_at: 2026-01-03T00:41:33.466026335+00:00
---

# Description

## Bug Summary

Modal/Charm spells using SP$ Charm API type are not implemented in the Rust engine. These are spells where the player chooses one or more modes.

## Affected Cards in Booster Draft Decks

From william_avatar_deck.dck:
- **Heartless Act** (1B Instant): SP$ Charm | Choices$ Destroy,Remove
  - Choose: Destroy creature with no counters OR remove up to 3 counters
- **Azula Always Lies** (1B Instant Lesson): SP$ Charm | MinCharmNum$ 1 | CharmNum$ 2
  - Choose one or both: -1/-1 until EOT OR put +1/+1 counter

## Root Cause

The ApiType enum in ability_parser.rs has no Charm variant. When SP$ Charm is parsed, it becomes ApiType::Unknown("Charm") which is then silently ignored.

## Implementation Notes

Charm spells require:
1. Add Charm to ApiType enum
2. Parse Choices$ parameter to get list of SVar names for modes
3. Parse MinCharmNum$ and CharmNum$ for mode selection constraints
4. Present player with mode selection before resolving
5. Execute the chosen mode(s) by calling their SVar definitions

## Related

- mtg-143: Missing player choice opportunities (tracks modal spell mode selection)
- mtg-21: SVar resolution (DB$ sub-abilities) - needed to execute chosen modes
