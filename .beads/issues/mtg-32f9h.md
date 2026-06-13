---
title: 'push_activatable_abilities: missing discard-cost check and activation-limit enforcement'
status: open
priority: 4
issue_type: task
created_at: 2026-06-13T16:23:14.044311189+00:00
updated_at: 2026-06-13T16:23:14.044311189+00:00
---

# Description

In game_loop/actions.rs push_activatable_abilities(), two cost-checking gaps remain (discovered during engine-cleanup wave2):\n1. Discard-cost affordability is not checked — abilities that cost 'discard a card' are offered even when the player has an empty hand. This means the heuristic and random controllers may choose an ability they cannot legally pay for.\n2. Activation-limit enforcement is not checked — abilities restricted to 'activate only once per turn' or 'activate only as a sorcery' are partially handled (sorcery-speed restriction is handled via ability.sorcery_speed), but global 'once per turn' limits (e.g. a card with ActivateLimit$ 1 per turn) are not enforced.\nFix: extend the ability.cost check loop to also test get_discard_cost() and any ActivateLimit statics. See push_activatable_abilities around the 'TODO: Check other cost types' comment.
