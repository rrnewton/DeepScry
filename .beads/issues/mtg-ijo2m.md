---
title: SpellCast triggers (T:Mode$ SpellCast) not firing
status: open
priority: 2
issue_type: bug
created_at: 2026-01-05T20:15:53.557464015+00:00
updated_at: 2026-01-05T20:15:53.557464015+00:00
---

# Description

## Summary

SpellCast triggers (T:Mode$ SpellCast) are defined in the loader but never fire during gameplay.
Creatures like Boar-q-pine that have "Whenever you cast a noncreature spell" triggers don't work.

## Evidence

Puzzle test with Boar-q-pine:
- P1 has Boar-q-pine (2/2) on battlefield
- P1 casts Lightning Strike (noncreature spell)
- Expected: Boar-q-pine gets +1/+1 counter, becomes 3/3
- Actual: Boar-q-pine remains 2/2

## Root Cause

1. cast_spell_8_step in actions/mod.rs:531 has TODO comment:
   "8. Spell becomes cast (trigger abilities) - TODO"

2. No code calls check_triggers(TriggerEvent::SpellCast, spell_id)

3. The Trigger struct doesn't have a ValidCard field for filtering
   (e.g., "noncreature spells only")

## Affected Cards

- Boar-q-pine: Whenever you cast a noncreature spell, +1/+1 counter
- Prowess creatures: +1/+1 until EOT when casting noncreature
- Young Pyromancer: Create 1/1 elemental on instant/sorcery
- Storm count tracking (if implemented)
