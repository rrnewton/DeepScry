---
title: Re-materialization on rewind loses 'controller' too (twin of tapped bug; robots deck cannot catch it)
status: open
priority: 2
issue_type: bug
created_at: 2026-06-05T17:36:16.634288391+00:00
updated_at: 2026-06-05T17:36:16.634288391+00:00
---

# Description

slot04 desync-review 2026-06-05 (completeness answer = tracked-latent-gap). On rewind the client rebuilds an opponent permanent from the blank card template (instantiate defaults ALL per-instance fields); slot03 fix reconstructs ONLY tapped. The EXACT twin exists for controller — the other hashed per-card field: ChangeController is undo-logged but nothing reconstructs it on re-materialization (reverts to owner). Robots deck has NO control-change cards, so a 100%-green robots sweep CANNOT exercise it = green-masks-coverage-gap trap. Reconstruct controller from the undo log on re-materialization (mirror reconstruct_tapped_states), OR prefer the principled generalization. Related: mtg-o99ow, mtg-677.
