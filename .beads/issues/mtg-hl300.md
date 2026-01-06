---
title: SpellCast triggers not firing (Boar-q-pine, Prowess)
status: closed
priority: 3
issue_type: bug
created_at: 2026-01-06T02:09:29.646191070+00:00
updated_at: 2026-01-06T15:00:09.731251663+00:00
closed_at: 2026-01-06T15:00:09.731251603+00:00
---

# Description

TriggerEvent::SpellCast is defined but never used. Cards like Boar-q-pine (whenever you cast a noncreature spell, put +1/+1 counter) don't trigger. Also affects Prowess keyword. Need to call check_triggers(TriggerEvent::SpellCast, spell_id) when resolving spells.
