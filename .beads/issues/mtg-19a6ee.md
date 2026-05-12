---
title: Unknown spell effects resolve silently as no-ops (affects Time Walk, many others)
status: closed
priority: 3
issue_type: task
labels:
- single-card
created_at: 2026-04-03T21:21:46.383140110+00:00
updated_at: 2026-05-12T13:57:36.799711356+00:00
closed_at: 2026-05-12T13:57:36.799711265+00:00
---

# Description

## Unknown/Unimplemented Spell Effects Are Silent No-ops

Context:
- Date: 2026-04-03
- Source code audit of card loading and spell resolution
- Discovered during extra turn / temporal effects audit

### Steps to Reproduce
1. Load any deck containing Time Walk (or any card with unimplemented ApiType)
2. Cast Time Walk
3. The spell goes on the stack, resolves, and goes to graveyard
4. No effect happens -- no log warning, no error

### Root Cause
In effect_converter.rs:865, unknown API types return None from params_to_effect().
In card.rs:2362-2365, None effect is converted to empty effects vec (vec![]).
In actions/mod.rs:294, resolve_spell loops over effects (0..effects_len), doing nothing for len 0.

### Expected Behavior
At minimum, a warning log message when:
1. A card with unimplemented abilities is loaded (card.rs)
2. A spell with no effects resolves (actions/mod.rs)

Ideally, the card loading should track which cards have partial/missing implementations
and report this to the user so they can choose different decks.

### Actual Behavior
Complete silence. A player could cast Time Walk dozens of times (via Regrowth/Recall)
and never know the engine is ignoring the effect entirely.

### Severity
This affects ANY card with unimplemented API types. Cards in current old school decks:
- Time Walk (ApiType AddTurn -- extra turns)
- Time Vault (ApiType AddTurn -- extra turns)
- Recall (may be partially working -- needs check)
- Potentially many others

### Suggested Fix
1. Log warning at card load time: "Card X has N unimplemented abilities"
2. Log warning at resolve time: "Spell X resolved with 0 effects (unimplemented)"
3. Consider a --strict mode that refuses to load decks with unimplemented cards
