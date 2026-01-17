---
title: SpellCast triggers (T:Mode$ SpellCast) not firing
status: closed
priority: 2
issue_type: bug
created_at: 2026-01-05T20:15:53.557464015+00:00
updated_at: 2026-01-17T04:28:38.704688283+00:00
---

# Description

## Summary

SpellCast triggers (T:Mode$ SpellCast) are IMPLEMENTED in the Rust engine.

**CLOSED 2026-01-17**: SpellCast triggers work correctly:
- check_spellcast_triggers() in actions/mod.rs handles trigger execution
- Called during cast_spell_8_step() at the correct time
- Prowess test passes: test_prowess_keyword_expansion

## Implementation Details

1. TriggerEvent::SpellCast is defined
2. check_spellcast_triggers() iterates over battlefield permanents
3. Matches triggers with event == SpellCast
4. Filters noncreature-only triggers correctly
5. Resolves placeholder targets (CardId 0 → self)
6. Supports both PutCounter and PumpCreature effects

The original issue may have been filed when the feature was not yet connected
to the main spell casting flow. It is now properly called from cast_spell_8_step.
