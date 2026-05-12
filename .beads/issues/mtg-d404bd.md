---
title: 'Bazaar of Baghdad: draw/discard activation broken'
status: closed
priority: 3
issue_type: task
labels:
- single-card
created_at: 2026-04-03T20:52:39.930087763+00:00
updated_at: 2026-05-12T13:57:36.799189016+00:00
closed_at: 2026-05-12T13:57:36.799188946+00:00
---

# Description

## Bazaar of Baghdad: draw/discard activation broken

**Card script:** `cardsfolder/b/bazaar_of_baghdad.txt`
**Ability:** `A:AB$ Draw | Cost$ T | NumCards$ 2 | SubAbility$ DBDiscard`
**SVar:** `SVar:DBDiscard:DB$ Discard | NumCards$ 3 | Mode$ TgtChoose`

### Root Cause Analysis (2026-04-03, thread-0)

**Two confirmed bugs:**

1. **Draw 2 works, Discard 3 silently fails.** The `parse_activated_abilities()` in `card.rs:2360` calls `params_to_effect_with_svars()` which returns a single effect (DrawCards). But it does NOT follow the SubAbility$ chain — it just does `vec![effect]`. The `follow_sub_ability_chain()` method exists and is used by `parse_effects()` (for spell abilities), but `parse_activated_abilities()` never calls it. So the DiscardCards effect from `SVar:DBDiscard` is never added to `ability.effects`.

   - **Fix:** In `parse_activated_abilities()` (~line 2360), after getting the initial effect, call `self.follow_sub_ability_chain(&params, &mut effects)` to chain SubAbility SVars into the effects list.

2. **No log lines for draw/discard.** When the activated ability resolves (priority.rs:1285), effects are executed via `game.execute_effect()` which does produce log output for draws. But since the discard effect is missing entirely, there's nothing to log for it.

### Reproduction
