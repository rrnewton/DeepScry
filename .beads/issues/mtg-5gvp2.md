---
title: Attack triggers (T:Mode$ Attacks) not firing
status: open
priority: 2
issue_type: bug
created_at: 2026-01-05T21:16:53.606707469+00:00
updated_at: 2026-01-05T21:16:53.606707469+00:00
---

# Description

## Summary

Attack triggers (T:Mode$ Attacks | ValidCard$ Card.Self) are defined but never fire when creatures attack. Similar to mtg-ijo2m (SpellCast triggers).

## Evidence

grep shows TriggerEvent::Attacks is defined but check_triggers is never called with it:
- TriggerEvent::Attacks exists in effects.rs:539
- declare_attacker() in actions/combat.rs doesn't call check_triggers

## Currently Implemented Triggers

Only these trigger types actually fire:
1. EntersBattlefield - called in actions/mod.rs:205
2. BeginningOfUpkeep - called in steps.rs:155

## Affected Cards (ryan_avatar_draft)

- Beetle-Headed Merchants: "Whenever this creature attacks, you may sacrifice..."
- Any card with attack triggers

## Fix

Add to declare_attacker() in actions/combat.rs:
```rust
// After declaring attacker, check for attack triggers
self.check_triggers(TriggerEvent::Attacks, card_id)?;
```

## Related Issues

- mtg-ijo2m: SpellCast triggers not implemented
- Should consider implementing ALL missing trigger types systematically
