---
title: 'targeting.rs: RemoveCounter/PutCounter should allow any permanent; ValidTgts for non-creature targets'
status: open
priority: 4
issue_type: task
created_at: 2026-06-13T16:29:18.257207804+00:00
updated_at: 2026-06-13T16:29:18.257207804+00:00
---

# Description

Several places in game/actions/targeting.rs get_valid_targets_for_spell() currently restrict targets to creatures when they should allow broader targeting:\n1. Effect::RemoveCounter: hardcoded to creatures; some cards (e.g. Vona's Hunger alternative) can target any permanent. Should check ValidTgts on the spell/ability.\n2. Effect::PutCounter: hardcoded to creatures; enchantments and artifacts can receive counters (Sphere of Safety, etc.). Should check ValidTgts.\n3. Effect::UnlessCostWrapper: inner effect targeting is skipped entirely — UnlessCost resolution delegates to the wrapped effect, but that wrapped effect's targets are not computed.\n4. Airbend 'GrantCastWithFlash' ValidTgts hardcoded to creatures; should read ValidTgts$ from the ability definition.\nFix: pass ValidTgts through get_valid_targets_for_spell() to select the correct target zone and type, rather than hardcoding creature-only.
