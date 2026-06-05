---
title: 'PRIZE BLOCKER: robots seed-19 Fireball shadow divergence -> client sends illegal choice index (turn 24)'
status: open
priority: 2
issue_type: bug
created_at: 2026-06-05T17:36:16.616566150+00:00
updated_at: 2026-06-05T17:36:16.616566150+00:00
---

# Description

slot04 desync-review 2026-06-05. Robots seed 19 FATAL (both action_count configs): client shadow state diverged enough to send an illegal choice (index 2 of only 2 options) at a Fireball cast, turn 24 — action-list/option-set divergence (client options != server options), same family as mtg-0e1wo (turn-9 CastFromExile over-generation). Find the FIRST divergent action_count in strict mode, root-cause why the shadow Fireball option set diverges. Activating the SpellAbility cross-check in the WASM client (mtg-j4krs) would catch this earlier by CardId. BLOCKS the prize. Related: mtg-0e1wo, mtg-j4krs, mtg-o99ow.
